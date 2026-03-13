use crate::agent::AgentLoop;
use crate::bus::MessageBus;
use crate::channels::manager::ChannelManager;
use crate::config::{Config, load_config};
use crate::cron::service::CronService;
use crate::cron::types::CronJob;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tracing::{debug, error, info, warn};

pub(super) async fn gateway(model: Option<String>) -> Result<()> {
    let process_start = std::time::Instant::now();
    info!("Loading configuration...");
    let config = load_config(None)?;
    let effective_model = model
        .as_deref()
        .unwrap_or(&config.agents.defaults.model_routing.default);
    info!("Configuration loaded. Using model: {}", effective_model);
    debug!("Workspace: {:?}", config.workspace_path());

    // Ensure workspace directory exists before writing templates
    crate::utils::ensure_dir(config.workspace_path())
        .context("failed to create workspace directory")?;

    // Ensure workspace template files exist (AGENTS.md, USER.md, etc.)
    super::create_workspace_templates(&config.workspace_path())?;

    // Create MemoryDB early so OAuth providers can use it for token caching.
    // Use spawn_blocking: SQLite open + PRAGMAs + schema migration are sync I/O.
    let memory_db = {
        let path = config
            .workspace_path()
            .join("memory")
            .join("memory.sqlite3");
        tokio::task::spawn_blocking(move || {
            crate::agent::memory::memory_db::MemoryDB::new(&path)
                .with_context(|| format!("failed to create MemoryDB at: {}", path.display()))
        })
        .await
        .context("MemoryDB init task panicked")??
    };
    let memory_db = Arc::new(memory_db);

    // Setup components
    let provider = setup_provider(&config, model.as_deref(), Some(memory_db.clone()))?;

    // Fire-and-forget warmup — don't block startup on a network round-trip
    {
        let p = provider.clone();
        tokio::spawn(async move {
            if let Err(e) = p.warmup().await {
                warn!("provider warmup failed (non-fatal): {}", e);
            }
        });
    }

    // Create a single shared leak detector with known secrets registered.
    // This is shared across the message bus, agent loop, subagents, and gateway
    // so that known-secret scanning is consistent everywhere.
    let leak_detector = {
        let mut detector = crate::safety::LeakDetector::new();
        let secrets = config.collect_secrets();
        if !secrets.is_empty() {
            detector.add_known_secrets(&secrets);
            debug!(
                "registered {} known secrets with shared leak detector",
                secrets.len()
            );
        }
        Arc::new(detector)
    };

    let (inbound_tx, outbound_tx, outbound_rx, bus_for_channels) =
        setup_message_bus_with_detector(leak_detector.clone())?;
    let cron = setup_cron_service(memory_db.clone());
    // Create typing indicator channel
    let (typing_tx, typing_rx) = tokio::sync::mpsc::channel::<(String, String)>(100);
    let typing_tx = Arc::new(typing_tx);

    // Start agent setup and HTTP gateway in parallel — gateway only needs message
    // bus channels, not the agent, so it can begin accepting connections sooner
    // (useful for container readiness probes).
    let agent_fut = setup_agent(
        SetupAgentParams {
            bus: bus_for_channels.clone(),
            provider,
            model: model.clone(),
            outbound_tx: outbound_tx.clone(),
            cron: Some(cron.clone()),
            typing_tx: Some(typing_tx),
            channels_config: Some(config.channels.clone()),
            memory_db: Some(memory_db),
            leak_detector: Some(leak_detector.clone()),
        },
        &config,
    );

    // Shared readiness flag — set to true once the agent loop is started.
    // The health endpoint checks this to distinguish liveness from readiness.
    let ready = Arc::new(AtomicBool::new(false));

    // Shared OnceLock — set after agent setup, read by status handlers.
    let status_lock = Arc::new(std::sync::OnceLock::new());

    let gateway_fut = async {
        if config.gateway.enabled {
            let a2a_config = if config.gateway.a2a.enabled {
                Some(config.gateway.a2a.clone())
            } else {
                None
            };
            let api_key = if config.gateway.api_key.is_empty() {
                None
            } else {
                Some(config.gateway.api_key.clone())
            };
            let (http_task, state) = crate::gateway::start(
                &config.gateway.host,
                config.gateway.port,
                Arc::new(inbound_tx.clone()),
                Some(outbound_tx.clone()),
                config.gateway.webhooks.clone(),
                a2a_config,
                api_key,
                &config.gateway.rate_limit,
                leak_detector.clone(),
                ready.clone(),
                status_lock.clone(),
                false, // not echo mode
            )
            .await?;
            Ok(Some((http_task, state)))
        } else {
            info!("HTTP API server disabled");
            Ok(None)
        }
    };

    let (agent, gateway_result) = tokio::join!(agent_fut, gateway_fut);
    let gateway_result: Result<
        Option<(tokio::task::JoinHandle<()>, crate::gateway::HttpApiState)>,
    > = gateway_result;

    // If agent setup failed, abort the HTTP server before propagating the error
    if agent.is_err()
        && let Ok(Some((ref http_task, _))) = gateway_result
    {
        http_task.abort();
    }

    let agent = agent?;
    let (http_state, _http_task) = match gateway_result? {
        Some((task, state)) => (Some(state), Some(task)),
        None => (None, None),
    };

    let memory_db_for_dlq = agent.memory_db();
    setup_cron_callbacks(
        cron.clone(),
        agent.clone(),
        bus_for_channels.clone(),
        memory_db_for_dlq,
    )
    .await?;

    // Build status page state now that agent (and its tool registry) is ready
    let tool_snap = crate::gateway::status::ToolSnapshot::from_registry(&agent.tool_registry());
    let config_snap = crate::gateway::status::StatusConfigSnapshot::from_config(&config);
    if status_lock
        .set(crate::gateway::status::StatusState {
            start_time: process_start,
            config_snapshot: Arc::new(config_snap),
            tool_snapshot: Arc::new(tool_snap),
            memory_db: agent.memory_db(),
        })
        .is_err()
    {
        error!("status OnceLock already set — this is a bug");
    }

    let channels = setup_channels(&config, inbound_tx);

    println!("Starting oxicrab gateway...");
    println!("Enabled channels: {:?}", channels.enabled_channels());
    if config.gateway.enabled {
        println!(
            "HTTP API listening on {}:{}",
            config.gateway.host, config.gateway.port
        );
    } else {
        println!("HTTP API server: disabled");
    }

    // Start services
    start_services(cron.clone()).await?;

    // Run agent and channels
    ready.store(true, std::sync::atomic::Ordering::SeqCst);
    let agent_task = start_agent_loop(agent.clone());
    let channels_task = start_channels_loop(channels, outbound_rx, typing_rx, http_state);

    info!("All services started, gateway is running");

    // Handle shutdown
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nShutting down...");
            cron.stop().await;
            agent.stop().await;
            // Channels will stop themselves when the task ends
        }
        _ = agent_task => {}
        _ = channels_task => {}
    }

    Ok(())
}

pub(super) async fn gateway_echo() -> Result<()> {
    info!("Loading configuration for echo mode...");
    let config = load_config(None)?;

    // Create shared leak detector for echo mode
    let leak_detector = {
        let mut detector = crate::safety::LeakDetector::new();
        let secrets = config.collect_secrets();
        if !secrets.is_empty() {
            detector.add_known_secrets(&secrets);
        }
        Arc::new(detector)
    };

    let (inbound_tx, outbound_tx, outbound_rx, bus) =
        setup_message_bus_with_detector(leak_detector.clone())?;
    // Create typing indicator channel (not used in echo mode but needed for channels)
    let (echo_typing_tx, typing_rx) = tokio::sync::mpsc::channel::<(String, String)>(100);
    drop(echo_typing_tx);

    // Echo mode is immediately ready (no agent loop to wait for)
    let ready = Arc::new(AtomicBool::new(true));

    // Start HTTP API server if enabled
    let http_state = if config.gateway.enabled {
        let api_key = if config.gateway.api_key.is_empty() {
            None
        } else {
            Some(config.gateway.api_key.clone())
        };
        let (http_task, state) = crate::gateway::start(
            &config.gateway.host,
            config.gateway.port,
            Arc::new(inbound_tx.clone()),
            Some(outbound_tx.clone()),
            config.gateway.webhooks.clone(),
            None, // A2A not available in echo mode
            api_key,
            &config.gateway.rate_limit,
            leak_detector,
            ready,
            Arc::new(std::sync::OnceLock::new()),
            true, // echo mode
        )
        .await?;
        drop(http_task);
        Some(state)
    } else {
        None
    };

    let channels = setup_channels(&config, inbound_tx);

    println!("Starting oxicrab gateway in ECHO mode (no LLM)...");
    println!("Enabled channels: {:?}", channels.enabled_channels());
    if config.gateway.enabled {
        println!(
            "HTTP API listening on {}:{}",
            config.gateway.host, config.gateway.port
        );
    }

    // Take inbound receiver from the bus
    let mut inbound_rx = bus
        .take_inbound_rx()
        .ok_or_else(|| anyhow::anyhow!("Inbound receiver already taken"))?;

    // Echo loop: read inbound, write echo outbound
    let echo_task = {
        let outbound_tx = outbound_tx.clone();
        tokio::spawn(async move {
            info!("echo loop started");
            while let Some(msg) = inbound_rx.recv().await {
                let echo_text = format!(
                    "[echo] channel={} | sender={} | message: {}",
                    msg.channel, msg.sender_id, msg.content
                );
                let _ = outbound_tx
                    .send(crate::bus::OutboundMessage::from_inbound(msg, echo_text).build())
                    .await;
            }
        })
    };

    let channels_task = start_channels_loop(channels, outbound_rx, typing_rx, http_state);

    info!("Echo gateway running");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nShutting down...");
        }
        _ = echo_task => {}
        _ = channels_task => {}
    }

    Ok(())
}

fn setup_provider(
    config: &Config,
    model: Option<&str>,
    db: Option<Arc<crate::agent::memory::memory_db::MemoryDB>>,
) -> Result<Arc<dyn crate::providers::base::LLMProvider>> {
    let effective_model = model.unwrap_or(&config.agents.defaults.model_routing.default);
    info!("Creating LLM provider for model: {}", effective_model);
    let provider = config.create_provider(model, db)?;
    info!(
        "Provider created successfully. Default model: {}",
        provider.default_model()
    );

    // Wrap with circuit breaker if enabled
    let provider = if config.providers.circuit_breaker.enabled {
        info!(
            "circuit breaker enabled (threshold={}, recovery={}s, probes={})",
            config.providers.circuit_breaker.failure_threshold,
            config.providers.circuit_breaker.recovery_timeout_secs,
            config.providers.circuit_breaker.half_open_probes,
        );
        crate::providers::circuit_breaker::CircuitBreakerProvider::wrap(
            provider,
            &config.providers.circuit_breaker,
        )
    } else {
        provider
    };

    Ok(provider)
}

pub(super) type MessageBusSetup = (
    tokio::sync::mpsc::Sender<crate::bus::InboundMessage>,
    Arc<tokio::sync::mpsc::Sender<crate::bus::OutboundMessage>>,
    tokio::sync::mpsc::Receiver<crate::bus::OutboundMessage>,
    Arc<MessageBus>,
);

fn setup_message_bus_with_detector(
    leak_detector: Arc<crate::safety::LeakDetector>,
) -> Result<MessageBusSetup> {
    debug!("Creating message bus...");
    let bus = MessageBus::with_leak_detector(
        30,   // DEFAULT_RATE_LIMIT
        60.0, // DEFAULT_RATE_WINDOW_S
        1000, // DEFAULT_INBOUND_CAPACITY
        1000, // DEFAULT_OUTBOUND_CAPACITY
        leak_detector,
    );

    let inbound_tx = bus.inbound_tx.clone();
    let outbound_tx = Arc::new(bus.outbound_tx.clone());
    let outbound_rx = bus
        .take_outbound_rx()
        .ok_or_else(|| anyhow::anyhow!("Outbound receiver already taken"))?;
    let bus_for_channels = Arc::new(bus);
    debug!("Message bus initialized");
    Ok((inbound_tx, outbound_tx, outbound_rx, bus_for_channels))
}

fn setup_cron_service(db: Arc<crate::agent::memory::memory_db::MemoryDB>) -> Arc<CronService> {
    debug!("Initializing cron service...");
    let cron = CronService::new(db);
    debug!("Cron service initialized");
    Arc::new(cron)
}

pub(super) struct SetupAgentParams {
    pub(super) bus: Arc<MessageBus>,
    pub(super) provider: Arc<dyn crate::providers::base::LLMProvider>,
    pub(super) model: Option<String>,
    pub(super) outbound_tx: Arc<tokio::sync::mpsc::Sender<crate::bus::OutboundMessage>>,
    pub(super) cron: Option<Arc<CronService>>,
    pub(super) typing_tx: Option<Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
    pub(super) channels_config: Option<crate::config::ChannelsConfig>,
    pub(super) memory_db: Option<Arc<crate::agent::memory::memory_db::MemoryDB>>,
    pub(super) leak_detector: Option<Arc<crate::safety::LeakDetector>>,
}

pub(super) async fn setup_agent(
    params: SetupAgentParams,
    config: &Config,
) -> Result<Arc<AgentLoop>> {
    info!("Initializing agent loop...");
    debug!(
        "  - Max tool iterations: {}",
        config.agents.defaults.max_tool_iterations
    );
    debug!("  - Exec timeout: {}s", config.tools.exec.timeout);
    debug!(
        "  - Restrict to workspace: {}",
        config.tools.restrict_to_workspace
    );
    debug!(
        "  - Compaction enabled: {}",
        config.agents.defaults.compaction.enabled
    );

    // Create model routing providers if configured
    let routing = match config.create_routed_providers(None) {
        Ok(Some(r)) => {
            info!("model routing active with {} task(s)", r.task_count());
            Some(Arc::new(r))
        }
        Ok(None) => None,
        Err(e) => {
            warn!("failed to create routed providers, routing disabled: {}", e);
            None
        }
    };

    let agent = Arc::new(
        AgentLoop::new(crate::agent::AgentLoopConfig::from_config(
            config,
            crate::agent::AgentLoopRuntimeParams {
                bus: params.bus,
                provider: params.provider,
                model: params.model,
                outbound_tx: params.outbound_tx,
                cron_service: params.cron,
                typing_tx: params.typing_tx,
                channels_config: params.channels_config,
                memory_db: params.memory_db,
                leak_detector: params.leak_detector,
            },
            routing,
        ))
        .await?,
    );
    info!("Agent loop initialized");
    Ok(agent)
}

async fn setup_cron_callbacks(
    cron: Arc<CronService>,
    agent: Arc<AgentLoop>,
    bus: Arc<MessageBus>,
    memory_db: Arc<crate::agent::memory::memory_db::MemoryDB>,
) -> Result<()> {
    debug!("Setting up cron job callback...");
    let agent_clone = agent.clone();
    let bus_clone = bus.clone();
    let db_clone = memory_db;
    cron.set_on_job(move |job| {
        debug!("Cron job triggered: {} - {}", job.id, job.payload.message);
        let agent = agent_clone.clone();
        let bus = bus_clone.clone();
        let db = db_clone.clone();
        Box::pin(async move {
            let result = cron_job_execute(&job, &agent, &bus).await;

            if let Err(ref e) = result {
                let payload_json =
                    serde_json::to_string(&job.payload).unwrap_or_else(|_| "{}".to_string());
                if let Err(dlq_err) =
                    db.insert_dlq_entry(&job.id, &job.name, &payload_json, &e.to_string())
                {
                    warn!("failed to insert DLQ entry for job {}: {}", job.id, dlq_err);
                }
            }

            result
        })
    })
    .await;
    Ok(())
}

async fn cron_job_execute(
    job: &CronJob,
    agent: &Arc<AgentLoop>,
    bus: &Arc<MessageBus>,
) -> Result<Option<String>> {
    if job.payload.kind == "echo" {
        // Echo mode: deliver message directly without invoking the LLM
        for target in &job.payload.targets {
            if let Err(e) = bus
                .publish_outbound(
                    crate::bus::OutboundMessage::builder(
                        target.channel.clone(),
                        target.to.clone(),
                        job.payload.message.clone(),
                    )
                    .build(),
                )
                .await
            {
                error!(
                    "Failed to publish echo message from cron to {}:{}: {}",
                    target.channel, target.to, e
                );
            }
        }
        return Ok(Some(job.payload.message.clone()));
    }

    // Agent mode: process as a full agent turn
    let (ctx_channel, ctx_chat_id) = job
        .payload
        .targets
        .first()
        .map_or(("cli", "direct"), |t| (t.channel.as_str(), t.to.as_str()));

    let mut cron_overrides = agent.resolve_overrides("cron");
    cron_overrides.metadata.insert(
        crate::bus::meta::IS_CRON_JOB.to_string(),
        serde_json::Value::Bool(true),
    );
    let result = agent
        .process_direct_with_overrides(
            &job.payload.message,
            &format!("cron:{}", job.id),
            ctx_channel,
            ctx_chat_id,
            &cron_overrides,
        )
        .await?;

    if job.payload.agent_echo {
        for target in &job.payload.targets {
            if let Err(e) = bus
                .publish_outbound(
                    crate::bus::OutboundMessage::builder(
                        target.channel.clone(),
                        target.to.clone(),
                        result.content.clone(),
                    )
                    .merge_metadata(result.metadata.clone())
                    .build(),
                )
                .await
            {
                error!(
                    "Failed to publish outbound message from cron to {}:{}: {}",
                    target.channel, target.to, e
                );
            }
        }
    }

    Ok(Some(result.content))
}

fn setup_channels(
    config: &Config,
    inbound_tx: tokio::sync::mpsc::Sender<crate::bus::InboundMessage>,
) -> ChannelManager {
    info!("Initializing channels...");
    let channels = ChannelManager::new(config, Arc::new(inbound_tx));
    info!(
        "Channels initialized. Enabled: {:?}",
        channels.enabled_channels()
    );
    channels
}

async fn start_services(cron: Arc<CronService>) -> Result<()> {
    info!("Starting cron service...");
    cron.start().await?;
    info!("Cron service started");
    Ok(())
}

fn start_agent_loop(agent: Arc<AgentLoop>) -> tokio::task::JoinHandle<()> {
    info!("Starting agent loop...");
    tokio::spawn(async move {
        info!("Agent loop running");
        match agent.run().await {
            Ok(()) => info!("Agent loop completed successfully"),
            Err(e) => error!("Agent loop exited with error: {}", e),
        }
    })
}

#[allow(clippy::too_many_lines)]
fn start_channels_loop(
    mut channels: ChannelManager,
    mut outbound_rx: tokio::sync::mpsc::Receiver<crate::bus::OutboundMessage>,
    mut typing_rx: tokio::sync::mpsc::Receiver<(String, String)>,
    http_api_state: Option<crate::gateway::HttpApiState>,
) -> tokio::task::JoinHandle<()> {
    info!("Starting all channels...");
    tokio::spawn(async move {
        match channels.start_all().await {
            Ok(()) => info!("All channels started successfully"),
            Err(e) => error!("Error starting channels: {}", e),
        }

        // Consume outbound messages and typing events
        // Use a shared reference for typing via Arc since we need it in a spawned task
        let channels = Arc::new(channels);
        let channels_for_typing = channels.clone();

        // Spawn typing indicator consumer
        tokio::spawn(async move {
            while let Some((channel, chat_id)) = typing_rx.recv().await {
                channels_for_typing.send_typing(&channel, &chat_id).await;
            }
        });

        // Track status messages for in-place editing
        let mut status_msg_ids: HashMap<(String, String), String> = HashMap::new();
        let mut status_content: HashMap<(String, String), String> = HashMap::new();

        loop {
            if let Some(msg) = outbound_rx.recv().await {
                // Route HTTP API responses back to waiting HTTP handlers.
                // Check channel first to avoid cloning for non-HTTP messages.
                if msg.channel == "http"
                    && let Some(ref state) = http_api_state
                    && crate::gateway::route_response(state, msg.clone())
                {
                    continue;
                }
                debug!(
                    "Consumed outbound message: channel={}, chat_id={}, content_len={}",
                    msg.channel,
                    msg.chat_id,
                    msg.content.len()
                );

                let is_status = msg
                    .metadata
                    .get(crate::bus::meta::STATUS)
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or_default();
                let key = (msg.channel.clone(), msg.chat_id.clone());

                if is_status {
                    // Accumulate status lines and snapshot for use after borrow ends
                    let content_snapshot = {
                        let accumulated = status_content.entry(key.clone()).or_default();
                        if !accumulated.is_empty() {
                            accumulated.push('\n');
                        }
                        accumulated.push_str(&msg.content);
                        accumulated.clone()
                    };

                    if let Some(existing_id) = status_msg_ids.get(&key) {
                        // Try to edit existing status message
                        if let Err(e) = channels
                            .edit_message(&key.0, &key.1, existing_id, &content_snapshot)
                            .await
                        {
                            debug!("Status edit failed, sending new: {}", e);
                            status_msg_ids.remove(&key);
                            status_content.remove(&key);
                        } else {
                            continue; // Edit succeeded
                        }
                    }

                    // Send new status message (first time or after edit failure)
                    if let std::collections::hash_map::Entry::Vacant(e) = status_msg_ids.entry(key)
                    {
                        let status_msg = crate::bus::OutboundMessage {
                            content: content_snapshot,
                            channel: msg.channel.clone(),
                            chat_id: msg.chat_id.clone(),
                            reply_to: msg.reply_to.clone(),
                            metadata: msg.metadata.clone(),
                            ..Default::default()
                        };
                        match channels.send_and_get_id(&status_msg).await {
                            Ok(Some(id)) => {
                                e.insert(id);
                            }
                            Ok(None) => {
                                // Channel doesn't support IDs (WhatsApp) — already sent
                            }
                            Err(err) => {
                                error!("Status send failed: {}", err);
                            }
                        }
                    }
                } else {
                    // Regular message — delete status message if one exists, then send
                    if let Some(msg_id) = status_msg_ids.remove(&key)
                        && let Err(e) = channels.delete_message(&key.0, &key.1, &msg_id).await
                    {
                        debug!("Status delete failed: {}", e);
                    }
                    status_content.remove(&key);

                    if let Err(e) = channels.send(&msg).await {
                        error!("Error sending message to channels: {}", e);
                    } else {
                        info!("Successfully sent outbound message to channel manager");
                    }
                }
            } else {
                warn!("Outbound message receiver closed");
                break;
            }
        }

        // Graceful shutdown - stop all channels when loop ends
        match Arc::try_unwrap(channels) {
            Ok(mut ch) => {
                if let Err(e) = ch.stop_all().await {
                    error!("Error stopping channels during shutdown: {}", e);
                }
            }
            Err(_) => {
                debug!("Channels still referenced by typing task, will be dropped");
            }
        }
    })
}
