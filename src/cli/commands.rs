use crate::agent::AgentLoop;
use crate::bus::MessageBus;
use crate::channels::manager::ChannelManager;
use crate::config::{Config, load_config};
use crate::cron::service::CronService;
use crate::cron::types::{CronJob, CronJobState, CronPayload, CronSchedule};
use crate::heartbeat::service::HeartbeatService;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

#[derive(Parser)]
#[command(name = "oxicrab")]
#[command(about = "Personal AI Assistant")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize oxicrab configuration and workspace
    Onboard,
    /// Run the gateway (channels + agent)
    Gateway {
        #[arg(long)]
        model: Option<String>,
        /// Override the LLM provider (e.g. anthropic, openai, groq, ollama)
        #[arg(long)]
        provider: Option<String>,
    },
    /// Interact with the agent directly
    Agent {
        #[arg(short, long)]
        message: Option<String>,
        #[arg(short, long, default_value = "cli:default")]
        session: String,
        /// Override the LLM provider (e.g. anthropic, openai, groq, ollama)
        #[arg(long)]
        provider: Option<String>,
    },
    /// Manage cron jobs
    Cron {
        #[command(subcommand)]
        cmd: CronCommands,
    },
    /// Manage authentication for external services
    Auth {
        #[command(subcommand)]
        cmd: AuthCommands,
    },
    /// Manage channels
    Channels {
        #[command(subcommand)]
        cmd: ChannelCommands,
    },
    /// Show oxicrab status
    Status,
    /// Run system diagnostics
    Doctor,
    /// Manage sender pairing (authorize new users to message the bot)
    Pairing {
        #[command(subcommand)]
        cmd: PairingCommands,
    },
    /// Manage credentials (keyring, env vars, credential helpers)
    Credentials {
        #[command(subcommand)]
        cmd: CredentialCommands,
    },
}

#[derive(Subcommand)]
enum CronCommands {
    /// List scheduled jobs
    List {
        #[arg(long, short = 'a')]
        all: bool,
    },
    /// Add a new job
    Add {
        #[arg(long, short = 'n')]
        name: String,
        #[arg(long, short = 'm')]
        message: String,
        #[arg(long, short = 'e')]
        every: Option<u64>,
        #[arg(long, short = 'c')]
        cron: Option<String>,
        #[arg(long)]
        tz: Option<String>,
        #[arg(long)]
        at: Option<String>,
        #[arg(long)]
        agent_echo: bool,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        all_channels: bool,
    },
    /// Remove a job
    Remove {
        #[arg(long)]
        id: String,
    },
    /// Enable or disable a job
    Enable {
        #[arg(long)]
        id: String,
        #[arg(long)]
        disable: bool,
    },
    /// Edit an existing job
    Edit {
        #[arg(long)]
        id: String,
        #[arg(long, short = 'n')]
        name: Option<String>,
        #[arg(long, short = 'm')]
        message: Option<String>,
        #[arg(long, short = 'e')]
        every: Option<u64>,
        #[arg(long, short = 'c')]
        cron: Option<String>,
        #[arg(long)]
        tz: Option<String>,
        #[arg(long)]
        at: Option<String>,
        #[arg(long)]
        agent_echo: Option<bool>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        all_channels: bool,
    },
    /// Manually run a job
    Run {
        #[arg(long)]
        id: String,
        #[arg(long, short = 'f')]
        force: bool,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Authenticate with Google (Gmail, Calendar)
    Google {
        #[arg(long, short = 'p', default_value = "8099")]
        port: u16,
        #[arg(long)]
        headless: bool,
    },
}

#[derive(Subcommand)]
enum PairingCommands {
    /// List pending pairing requests and paired sender counts
    List,
    /// Approve a pending request by its 8-character code (e.g. ABC12345)
    Approve {
        /// The pairing code shown by `oxicrab pairing list`
        code: String,
    },
    /// Revoke a previously approved sender's access
    Revoke {
        /// Channel name: telegram, discord, slack, whatsapp, or twilio
        channel: String,
        /// The sender ID to remove (same format as allowFrom entries)
        sender_id: String,
    },
}

#[derive(Subcommand)]
enum ChannelCommands {
    /// Show channel status
    Status,
    /// Link `WhatsApp` device via QR code
    Login,
}

#[derive(Subcommand)]
enum CredentialCommands {
    /// Store a credential in the OS keyring
    Set {
        /// Credential slot name (e.g. "anthropic-api-key")
        name: String,
        /// Value to store (reads from stdin if omitted)
        value: Option<String>,
    },
    /// Check if a credential exists (shows [set] or [empty])
    Get {
        /// Credential slot name
        name: String,
    },
    /// Remove a credential from the OS keyring
    Delete {
        /// Credential slot name
        name: String,
    },
    /// List all credential slots and their sources
    List,
    /// Import non-empty credentials from config.json into the OS keyring
    Import,
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Onboard => {
            onboard()?;
        }
        Commands::Gateway { model, provider } => {
            gateway(model, provider).await?;
        }
        Commands::Agent {
            message,
            session,
            provider,
        } => {
            agent(message, session, provider).await?;
        }
        Commands::Cron { cmd } => {
            cron_command(cmd).await?;
        }
        Commands::Auth { cmd } => {
            auth_command(cmd).await?;
        }
        Commands::Channels { cmd } => {
            channels_command(cmd).await?;
        }
        Commands::Status => {
            status_command()?;
        }
        Commands::Doctor => {
            crate::cli::doctor::doctor_command().await?;
        }
        Commands::Pairing { cmd } => {
            pairing_command(cmd)?;
        }
        Commands::Credentials { cmd } => {
            credentials_command(cmd)?;
        }
    }

    Ok(())
}

fn onboard() -> Result<()> {
    println!("🤖 Initializing oxicrab...");

    let config_path = crate::config::get_config_path()?;
    if config_path.exists() {
        println!("⚠️  Config already exists at {}", config_path.display());
        println!("Overwrite? (y/N): ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            return Ok(());
        }
    }

    let config = Config::default();
    crate::config::save_config(&config, Some(config_path.as_path()))?;
    println!("✓ Created config at {}", config_path.display());

    let workspace = config.workspace_path();
    crate::utils::ensure_dir(&workspace)?;
    println!("✓ Created workspace at {}", workspace.display());

    create_workspace_templates(&workspace)?;

    println!("\n🤖 oxicrab is ready!");
    println!("\nNext steps:");
    println!("  1. Add your API key to ~/.oxicrab/config.json");
    println!("     Get one at: https://openrouter.ai/keys");
    println!("  2. Chat: oxicrab agent -m \"Hello!\"");

    Ok(())
}

fn create_workspace_templates(workspace: &std::path::Path) -> Result<()> {
    let templates = vec![
        (
            "USER.md",
            r"# User

Information about the user goes here.

## Preferences

- Communication style: (casual/formal)
- Timezone: (your timezone)
- Language: (your preferred language)
",
        ),
        (
            "AGENTS.md",
            r#"# oxicrab

I am oxicrab, a personal AI assistant.

## Personality

- Friendly but professional
- Direct and concise, with detail when needed
- Accuracy over speed

## Capabilities

I have access to a variety of tools including file operations, web search, shell commands, messaging, and more. Some tools (Google services, GitHub, weather, etc.) require additional configuration.

## Behavioral Rules

- When responding to direct questions or conversations, reply directly with text. Your text response will be delivered to the user automatically.
- Always be helpful, accurate, and concise. When using tools, explain what you're doing.
- Ask for clarification when the request is ambiguous.
- Never invent, guess, or make up information. If you don't know something:
  - Say "I don't know" or "I'm not sure" clearly
  - Use tools (web_search, read_file) to find accurate information before answering
  - Never guess file paths, command syntax, API details, or factual claims

### Action Integrity

Never claim you performed an action (created, updated, wrote, deleted, configured, set up, etc.) unless you actually called a tool to do it in this conversation turn. If you cannot perform the requested action, explain what you would need to do and offer to do it.

When asked to retry, re-run, or re-check something, you MUST actually call the tool again. Never repeat a previous result from conversation history.

## Memory Management

I actively maintain my memory to be useful across sessions:

- **MEMORY.md**: Long-term facts, user preferences, and important context
- **Daily notes** (`memory/YYYY-MM-DD.md`): Session summaries and daily context
- **AGENTS.md**: My own identity. Update the "Learned Adaptations" section when I discover consistent user preferences
- **USER.md**: User preferences and habits. Update when I notice patterns

Be selective — only record genuinely useful facts, not transient conversation details.

## Learned Adaptations

*(This section is updated as I learn about user preferences)*
"#,
        ),
        (
            "TOOLS.md",
            r"# Tool Notes

Notes and configuration details for tools.

## Configured Tools

*(List tools you've configured and any important notes about them)*

## API Keys & Services

*(Record which services are set up — do NOT store actual keys here)*
",
        ),
    ];

    for (filename, content) in templates {
        let file_path = workspace.join(filename);
        if !file_path.exists() {
            std::fs::write(&file_path, content)?;
            println!("  Created {}", filename);
        }
    }

    // Create memory directory and MEMORY.md
    let memory_dir = workspace.join("memory");
    crate::utils::ensure_dir(&memory_dir)?;
    let memory_file = memory_dir.join("MEMORY.md");
    if !memory_file.exists() {
        let memory_content = r"# Long-term Memory

This file stores important information that should persist across sessions.

## User Information

(Important facts about the user)

## Preferences

(User preferences learned over time)

## Important Notes

(Things to remember)
";
        std::fs::write(&memory_file, memory_content)?;
        println!("  Created memory/MEMORY.md");
    }

    Ok(())
}

async fn gateway(model: Option<String>, provider: Option<String>) -> Result<()> {
    info!("Loading configuration...");
    let mut config = load_config(None)?;
    if let Some(ref p) = provider {
        config.agents.defaults.provider = Some(p.clone());
    }
    let effective_model = model.as_deref().unwrap_or(&config.agents.defaults.model);
    info!("Configuration loaded. Using model: {}", effective_model);
    debug!("Workspace: {:?}", config.workspace_path());

    // Setup components
    let provider = setup_provider(&config, model.as_deref())?;

    // Warmup provider connection (non-blocking, non-fatal)
    if let Err(e) = provider.warmup().await {
        warn!("provider warmup failed (non-fatal): {}", e);
    }

    let (inbound_tx, outbound_tx, outbound_rx, bus_for_channels) = setup_message_bus(&config)?;
    let cron = setup_cron_service()?;
    // Create typing indicator channel
    let (typing_tx, typing_rx) = tokio::sync::mpsc::channel::<(String, String)>(100);
    let typing_tx = Arc::new(typing_tx);

    let agent = setup_agent(
        SetupAgentParams {
            bus: bus_for_channels.clone(),
            provider,
            model: model.clone(),
            outbound_tx: outbound_tx.clone(),
            cron: Some(cron.clone()),
            typing_tx: Some(typing_tx),
            channels_config: Some(config.channels.clone()),
        },
        &config,
    )
    .await?;
    setup_cron_callbacks(cron.clone(), agent.clone(), bus_for_channels.clone()).await?;
    let heartbeat = setup_heartbeat(&config, &agent);
    let channels = setup_channels(&config, inbound_tx);

    println!("Starting oxicrab gateway...");
    println!("Enabled channels: {:?}", channels.enabled_channels());

    // Start services
    start_services(cron.clone(), heartbeat.clone()).await?;

    // Run agent and channels
    let agent_task = start_agent_loop(agent.clone());
    let channels_task = start_channels_loop(channels, outbound_rx, typing_rx);

    info!("All services started, gateway is running");

    // Handle shutdown
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nShutting down...");
            heartbeat.stop().await;
            cron.stop().await;
            agent.stop();
            // Channels will stop themselves when the task ends
        }
        _ = agent_task => {}
        _ = channels_task => {}
    }

    Ok(())
}

fn setup_provider(
    config: &Config,
    model: Option<&str>,
) -> Result<Arc<dyn crate::providers::base::LLMProvider>> {
    let effective_model = model.unwrap_or(&config.agents.defaults.model);
    info!("Creating LLM provider for model: {}", effective_model);
    let provider = config.create_provider(model)?;
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

type MessageBusSetup = (
    tokio::sync::mpsc::Sender<crate::bus::InboundMessage>,
    Arc<tokio::sync::mpsc::Sender<crate::bus::OutboundMessage>>,
    tokio::sync::mpsc::Receiver<crate::bus::OutboundMessage>,
    Arc<Mutex<MessageBus>>,
);

fn setup_message_bus(config: &Config) -> Result<MessageBusSetup> {
    debug!("Creating message bus...");
    let mut bus = MessageBus::default();

    // Register known secrets so the leak detector can find encoded variants
    let secrets = config.collect_secrets();
    if !secrets.is_empty() {
        debug!(
            "registering {} known secrets with leak detector",
            secrets.len()
        );
        bus.add_known_secrets(&secrets);
    }

    let inbound_tx = bus.inbound_tx.clone();
    let outbound_tx = Arc::new(bus.outbound_tx.clone());
    let outbound_rx = bus
        .take_outbound_rx()
        .ok_or_else(|| anyhow::anyhow!("Outbound receiver already taken"))?;
    let bus_for_channels = Arc::new(Mutex::new(bus));
    debug!("Message bus initialized");
    Ok((inbound_tx, outbound_tx, outbound_rx, bus_for_channels))
}

fn setup_cron_service() -> Result<Arc<CronService>> {
    debug!("Initializing cron service...");
    let cron_store_path = crate::utils::get_oxicrab_home()?
        .join("cron")
        .join("jobs.json");
    let cron = CronService::new(cron_store_path);
    debug!("Cron service initialized");
    Ok(Arc::new(cron))
}

struct SetupAgentParams {
    bus: Arc<Mutex<MessageBus>>,
    provider: Arc<dyn crate::providers::base::LLMProvider>,
    model: Option<String>,
    outbound_tx: Arc<tokio::sync::mpsc::Sender<crate::bus::OutboundMessage>>,
    cron: Option<Arc<CronService>>,
    typing_tx: Option<Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
    channels_config: Option<crate::config::ChannelsConfig>,
}

async fn setup_agent(params: SetupAgentParams, config: &Config) -> Result<Arc<AgentLoop>> {
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
            },
        ))
        .await?,
    );
    info!("Agent loop initialized");
    Ok(agent)
}

async fn setup_cron_callbacks(
    cron: Arc<CronService>,
    agent: Arc<AgentLoop>,
    bus: Arc<Mutex<MessageBus>>,
) -> Result<()> {
    debug!("Setting up cron job callback...");
    let agent_clone = agent.clone();
    let bus_clone = bus.clone();
    cron.set_on_job(move |job| {
        debug!("Cron job triggered: {} - {}", job.id, job.payload.message);
        let agent = agent_clone.clone();
        let bus = bus_clone.clone();
        Box::pin(async move {
            if job.payload.kind == "echo" {
                // Echo mode: deliver message directly without invoking the LLM
                for target in &job.payload.targets {
                    let bus_guard = bus.lock().await;
                    if let Err(e) = bus_guard
                        .publish_outbound(crate::bus::OutboundMessage {
                            channel: target.channel.clone(),
                            chat_id: target.to.clone(),
                            content: job.payload.message.clone(),
                            reply_to: None,
                            media: vec![],
                            metadata: std::collections::HashMap::new(),
                        })
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

            let response = agent
                .process_direct(
                    &job.payload.message,
                    &format!("cron:{}", job.id),
                    ctx_channel,
                    ctx_chat_id,
                )
                .await?;

            if job.payload.agent_echo {
                for target in &job.payload.targets {
                    let bus_guard = bus.lock().await;
                    if let Err(e) = bus_guard
                        .publish_outbound(crate::bus::OutboundMessage {
                            channel: target.channel.clone(),
                            chat_id: target.to.clone(),
                            content: response.clone(),
                            reply_to: None,
                            media: vec![],
                            metadata: std::collections::HashMap::new(),
                        })
                        .await
                    {
                        error!(
                            "Failed to publish outbound message from cron to {}:{}: {}",
                            target.channel, target.to, e
                        );
                    }
                }
            }

            Ok(Some(response))
        })
    })
    .await;
    Ok(())
}

fn setup_heartbeat(config: &Config, agent: &Arc<AgentLoop>) -> Arc<HeartbeatService> {
    debug!("Initializing heartbeat service...");
    debug!("  - Enabled: {}", config.agents.defaults.daemon.enabled);
    debug!("  - Interval: {}s", config.agents.defaults.daemon.interval);
    debug!(
        "  - Strategy file: {}",
        config.agents.defaults.daemon.strategy_file
    );

    // Build daemon-specific overrides from config
    let daemon_cfg = &config.agents.defaults.daemon;
    let daemon_overrides = Arc::new(crate::agent::AgentRunOverrides {
        model: daemon_cfg.execution_model.clone(),
        max_iterations: Some(daemon_cfg.max_iterations),
    });

    if daemon_cfg.execution_model.is_some() {
        info!(
            "daemon will use model override: {}",
            daemon_cfg.execution_model.as_deref().unwrap_or("(none)")
        );
    }
    if daemon_cfg.execution_provider.is_some() {
        warn!(
            "daemon executionProvider is not yet supported and will be ignored; \
             the default provider will be used"
        );
    }

    let agent_for_heartbeat = agent.clone();
    let heartbeat = HeartbeatService::new(
        config.workspace_path(),
        Some(Arc::new(move |prompt| {
            debug!("Heartbeat triggered with prompt: {}", prompt);
            let agent = agent_for_heartbeat.clone();
            let overrides = daemon_overrides.clone();
            Box::pin(async move {
                agent
                    .process_direct_with_overrides(&prompt, "daemon", "cli", "direct", &overrides)
                    .await
            })
        })),
        config.agents.defaults.daemon.interval,
        config.agents.defaults.daemon.enabled,
        config.agents.defaults.daemon.strategy_file.clone(),
    );
    debug!("Heartbeat service initialized");
    Arc::new(heartbeat)
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

async fn start_services(cron: Arc<CronService>, heartbeat: Arc<HeartbeatService>) -> Result<()> {
    info!("Starting cron service...");
    cron.start().await?;
    info!("Cron service started");

    info!("Starting heartbeat service...");
    heartbeat.start().await?;
    info!("Heartbeat service started");
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
                debug!(
                    "Consumed outbound message: channel={}, chat_id={}, content_len={}",
                    msg.channel,
                    msg.chat_id,
                    msg.content.len()
                );

                let is_status = msg
                    .metadata
                    .get("status")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
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
                            media: vec![],
                            metadata: msg.metadata.clone(),
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

async fn agent(message: Option<String>, session: String, provider: Option<String>) -> Result<()> {
    let mut config = load_config(None)?;
    if let Some(ref p) = provider {
        config.agents.defaults.provider = Some(p.clone());
    }
    config.validate()?;

    let provider = config.create_provider(None)?;

    let mut bus = MessageBus::default();
    let secrets = config.collect_secrets();
    if !secrets.is_empty() {
        bus.add_known_secrets(&secrets);
    }
    let outbound_tx = Arc::new(bus.outbound_tx.clone());
    let bus_for_agent = Arc::new(Mutex::new(bus));

    let agent = setup_agent(
        SetupAgentParams {
            bus: bus_for_agent,
            provider,
            model: None,
            outbound_tx,
            cron: None,
            typing_tx: None,
            channels_config: None,
        },
        &config,
    )
    .await?;

    if let Some(msg) = message {
        let response = agent
            .process_direct(&msg, &session, "cli", "direct")
            .await?;
        println!("🤖 {}", response);
    } else {
        interactive_repl(&agent, &session).await?;
    }

    Ok(())
}

async fn interactive_repl(agent: &AgentLoop, session: &str) -> Result<()> {
    use std::io::{self, BufRead, Write};

    println!("🤖 Interactive mode (Ctrl+C to exit)\n");
    loop {
        print!("You: ");
        io::stdout().flush()?;

        let stdin = io::stdin();
        let mut input = String::new();
        stdin.lock().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            continue;
        }

        let response = agent
            .process_direct(input, session, "cli", "direct")
            .await?;
        println!("\n🤖 {}\n", response);
    }
}

#[allow(clippy::too_many_lines)]
async fn cron_command(cmd: CronCommands) -> Result<()> {
    let _config = load_config(None)?;
    let cron_store_path = crate::utils::get_oxicrab_home()?
        .join("cron")
        .join("jobs.json");
    let cron = CronService::new(cron_store_path);

    match cmd {
        CronCommands::List { all } => {
            let jobs = cron.list_jobs(all).await?;
            if jobs.is_empty() {
                println!("No cron jobs found.");
            } else {
                println!("Cron jobs:");
                for job in jobs {
                    let status = if job.enabled { "enabled" } else { "disabled" };
                    let next_run = job.state.next_run_at_ms.map_or_else(
                        || "never".to_string(),
                        |ms| {
                            chrono::DateTime::from_timestamp(ms / 1000, 0).map_or_else(
                                || "invalid timestamp".to_string(),
                                |dt| format!("{}", dt.format("%Y-%m-%d %H:%M:%S")),
                            )
                        },
                    );
                    println!(
                        "  [{}] {} - {} (next: {})",
                        job.id, job.name, status, next_run
                    );
                }
            }
        }
        CronCommands::Add {
            name,
            message,
            every,
            cron: cron_expr,
            tz,
            at,
            agent_echo,
            to,
            channel,
            all_channels,
        } => {
            use crate::agent::tools::cron::resolve_all_channel_targets_from_config;
            use crate::cron::types::CronTarget;

            let targets = if all_channels {
                let config = load_config(None)?;
                let targets = resolve_all_channel_targets_from_config(Some(&config.channels));
                if targets.is_empty() {
                    anyhow::bail!("No enabled channels with allowFrom configured");
                }
                targets
            } else if let (Some(ch), Some(to_val)) = (channel, to) {
                vec![CronTarget {
                    channel: ch,
                    to: to_val,
                }]
            } else {
                anyhow::bail!("Either --channel + --to or --all-channels is required");
            };

            let schedule = if let Some(every_sec) = every {
                CronSchedule::Every {
                    every_ms: Some(every_sec.saturating_mul(1000).min(i64::MAX as u64) as i64),
                }
            } else if let Some(expr) = cron_expr {
                // Validate the expression parses
                crate::cron::service::validate_cron_expr(&expr)?;
                let tz = tz.or_else(crate::cron::service::detect_system_timezone);
                CronSchedule::Cron {
                    expr: Some(expr),
                    tz,
                }
            } else if let Some(at_str) = at {
                let dt = chrono::DateTime::parse_from_rfc3339(&at_str)
                    .or_else(|_| chrono::DateTime::parse_from_str(&at_str, "%Y-%m-%d %H:%M:%S"))
                    .context("Invalid date format. Use ISO 8601 or YYYY-MM-DD HH:MM:SS")?;
                CronSchedule::At {
                    at_ms: Some(dt.timestamp_millis()),
                }
            } else {
                anyhow::bail!("Must specify --every, --cron, or --at");
            };

            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("System time is before UNIX epoch")
                .map(|d| d.as_millis() as i64)?;

            let job = CronJob {
                id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
                name,
                enabled: true,
                schedule,
                payload: CronPayload {
                    kind: "agent_turn".to_string(),
                    message,
                    agent_echo,
                    targets,
                },
                state: CronJobState {
                    next_run_at_ms: None, // Will be computed by service
                    last_run_at_ms: None,
                    last_status: None,
                    last_error: None,
                    run_count: 0,
                    last_fired_at_ms: None,
                },
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
                delete_after_run: false,
                expires_at_ms: None,
                max_runs: None,
                cooldown_secs: None,
                max_concurrent: None,
            };

            cron.add_job(job).await?;
            println!("Cron job added successfully.");
        }
        CronCommands::Remove { id } => match cron.remove_job(&id).await? {
            Some(job) => {
                println!("Removed cron job: {} ({})", job.name, job.id);
            }
            None => {
                println!("Cron job {} not found.", id);
            }
        },
        CronCommands::Enable { id, disable } => match cron.enable_job(&id, !disable).await? {
            Some(job) => {
                let status = if job.enabled { "enabled" } else { "disabled" };
                println!("Job {} ({}) {}", job.name, job.id, status);
            }
            None => {
                println!("Cron job {} not found.", id);
            }
        },
        CronCommands::Edit {
            id,
            name,
            message,
            every,
            cron: cron_expr,
            tz,
            at,
            agent_echo,
            to,
            channel,
            all_channels,
        } => {
            use crate::agent::tools::cron::resolve_all_channel_targets_from_config;
            use crate::cron::types::CronTarget;

            let schedule = if let Some(every_sec) = every {
                Some(CronSchedule::Every {
                    every_ms: Some(every_sec.saturating_mul(1000).min(i64::MAX as u64) as i64),
                })
            } else if let Some(expr) = cron_expr {
                crate::cron::service::validate_cron_expr(&expr)?;
                Some(CronSchedule::Cron {
                    expr: Some(expr),
                    tz,
                })
            } else if let Some(at_str) = at {
                let dt = chrono::DateTime::parse_from_rfc3339(&at_str)
                    .or_else(|_| chrono::DateTime::parse_from_str(&at_str, "%Y-%m-%d %H:%M:%S"))
                    .context("Invalid date format. Use ISO 8601 or YYYY-MM-DD HH:MM:SS")?;
                Some(CronSchedule::At {
                    at_ms: Some(dt.timestamp_millis()),
                })
            } else if tz.is_some() {
                // Just updating timezone - need to get current job
                let jobs = cron.list_jobs(true).await?;
                let current_job = jobs.iter().find(|j| j.id == id);
                if let Some(job) = current_job {
                    if let CronSchedule::Cron { expr, .. } = &job.schedule {
                        Some(CronSchedule::Cron {
                            expr: expr.clone(),
                            tz,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            let targets = if all_channels {
                let config = load_config(None)?;
                let targets = resolve_all_channel_targets_from_config(Some(&config.channels));
                if targets.is_empty() {
                    anyhow::bail!("No enabled channels with allowFrom configured");
                }
                Some(targets)
            } else if let (Some(ch), Some(to_val)) = (channel, to) {
                Some(vec![CronTarget {
                    channel: ch,
                    to: to_val,
                }])
            } else {
                None
            };

            match cron
                .update_job(
                    &id,
                    crate::cron::types::UpdateJobParams {
                        name,
                        message,
                        schedule,
                        agent_echo,
                        targets,
                    },
                )
                .await?
            {
                Some(job) => {
                    println!("Updated job: {} ({})", job.name, job.id);
                }
                None => {
                    println!("Cron job {} not found.", id);
                }
            }
        }
        CronCommands::Run { id, force } => match cron.run_job(&id, force).await? {
            Some(result) => {
                println!("Job executed successfully.");
                if let Some(output) = result {
                    println!("{}", output);
                }
            }
            None => {
                println!("Failed to run job {} (not found or disabled)", id);
            }
        },
    }

    Ok(())
}

async fn auth_command(cmd: AuthCommands) -> Result<()> {
    match cmd {
        AuthCommands::Google { port, headless } => {
            let config = load_config(None)?;
            let gcfg = &config.tools.google;

            if gcfg.client_id.is_empty() || gcfg.client_secret.is_empty() {
                eprintln!("Error: Google client_id and client_secret are not configured.");
                eprintln!("\nAdd them to ~/.oxicrab/config.json under tools.google:");
                eprintln!("  \"tools\": {{");
                eprintln!("    \"google\": {{");
                eprintln!("      \"enabled\": true,");
                eprintln!("      \"clientId\": \"YOUR_CLIENT_ID\",");
                eprintln!("      \"clientSecret\": \"YOUR_CLIENT_SECRET\"");
                eprintln!("    }}");
                eprintln!("  }}");
                eprintln!(
                    "\nGet credentials at: https://console.cloud.google.com/apis/credentials"
                );
                return Ok(());
            }

            // Check if already authenticated
            if crate::auth::google::has_valid_credentials(
                &gcfg.client_id,
                &gcfg.client_secret,
                Some(&gcfg.scopes),
                None,
            ) {
                println!("✓ Already authenticated with Google.");
                // In real implementation, would prompt for re-auth
            }

            if headless {
                println!("🤖 Starting Google OAuth2 flow (headless mode)...");
            } else {
                println!("🤖 Starting Google OAuth2 flow...");
                println!(
                    "A browser window will open — or fall back to manual mode if unavailable.\n"
                );
            }

            let _creds = crate::auth::google::run_oauth_flow(
                &gcfg.client_id,
                &gcfg.client_secret,
                Some(&gcfg.scopes),
                None,
                port,
                headless,
            )
            .await?;

            println!("\n✓ Google authentication successful!");
            println!("  Tokens saved to ~/.oxicrab/google_tokens.json");
            println!(
                "\nMake sure tools.google.enabled is true in your config, then restart the gateway."
            );
        }
    }
    Ok(())
}

// Variables/async used conditionally inside #[cfg(feature)] blocks
#[allow(clippy::too_many_lines, unused_variables, clippy::unused_async)]
async fn channels_command(cmd: ChannelCommands) -> Result<()> {
    match cmd {
        ChannelCommands::Status => {
            let config = load_config(None)?;

            println!("Channel Status");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

            // WhatsApp
            #[cfg(feature = "channel-whatsapp")]
            {
                let wa = &config.channels.whatsapp;
                println!(
                    "WhatsApp: {}",
                    if wa.enabled {
                        "✓ enabled"
                    } else {
                        "✗ disabled"
                    }
                );
                if wa.enabled {
                    let session_path = crate::utils::get_oxicrab_home().map_or_else(
                        |_| std::path::PathBuf::from(".oxicrab/whatsapp/whatsapp.db"),
                        |h| h.join("whatsapp").join("whatsapp.db"),
                    );
                    let session_exists = session_path.exists();
                    println!(
                        "  Session: {} ({})",
                        session_path.display(),
                        if session_exists {
                            "exists"
                        } else {
                            "not paired - run 'oxicrab channels login'"
                        }
                    );
                }
            }
            #[cfg(not(feature = "channel-whatsapp"))]
            println!("WhatsApp: not compiled (enable 'channel-whatsapp' feature)");

            // Discord
            #[cfg(feature = "channel-discord")]
            {
                let dc = &config.channels.discord;
                println!(
                    "Discord: {}",
                    if dc.enabled {
                        "✓ enabled"
                    } else {
                        "✗ disabled"
                    }
                );
                if dc.enabled {
                    println!(
                        "  Token: {}",
                        if dc.token.is_empty() {
                            "not set"
                        } else {
                            "configured"
                        }
                    );
                }
            }
            #[cfg(not(feature = "channel-discord"))]
            println!("Discord: not compiled (enable 'channel-discord' feature)");

            // Telegram
            #[cfg(feature = "channel-telegram")]
            {
                let tg = &config.channels.telegram;
                println!(
                    "Telegram: {}",
                    if tg.enabled {
                        "✓ enabled"
                    } else {
                        "✗ disabled"
                    }
                );
                if tg.enabled {
                    println!(
                        "  Token: {}",
                        if tg.token.is_empty() {
                            "not set"
                        } else {
                            "configured"
                        }
                    );
                }
            }
            #[cfg(not(feature = "channel-telegram"))]
            println!("Telegram: not compiled (enable 'channel-telegram' feature)");

            // Slack
            #[cfg(feature = "channel-slack")]
            {
                let sl = &config.channels.slack;
                println!(
                    "Slack: {}",
                    if sl.enabled {
                        "✓ enabled"
                    } else {
                        "✗ disabled"
                    }
                );
                if sl.enabled {
                    println!(
                        "  Bot Token: {}",
                        if sl.bot_token.is_empty() {
                            "not set"
                        } else {
                            "configured"
                        }
                    );
                }
            }
            #[cfg(not(feature = "channel-slack"))]
            println!("Slack: not compiled (enable 'channel-slack' feature)");
        }
        ChannelCommands::Login => {
            #[cfg(feature = "channel-whatsapp")]
            whatsapp_login().await?;
            #[cfg(not(feature = "channel-whatsapp"))]
            anyhow::bail!("WhatsApp support not compiled (enable 'channel-whatsapp' feature)");
        }
    }
    Ok(())
}

fn status_command() -> Result<()> {
    let config = load_config(None)?;
    let config_path = crate::config::get_config_path()?;
    let workspace = config.workspace_path();

    println!("🤖 oxicrab Status\n");

    println!(
        "Config: {} {}",
        config_path.display(),
        if config_path.exists() { "✓" } else { "✗" }
    );
    println!(
        "Workspace: {} {}",
        workspace.display(),
        if workspace.exists() { "✓" } else { "✗" }
    );

    if config_path.exists() {
        println!("Model: {}", config.agents.defaults.model);

        // Check API keys
        let has_openrouter = !config.providers.openrouter.api_key.is_empty();
        let has_anthropic = !config.providers.anthropic.api_key.is_empty();
        let has_openai = !config.providers.openai.api_key.is_empty();
        let has_gemini = !config.providers.gemini.api_key.is_empty();
        let has_vllm = config.providers.vllm.api_base.is_some();

        println!(
            "OpenRouter API: {}",
            if has_openrouter { "✓" } else { "not set" }
        );
        println!(
            "Anthropic API: {}",
            if has_anthropic { "✓" } else { "not set" }
        );
        println!("OpenAI API: {}", if has_openai { "✓" } else { "not set" });
        println!("Gemini API: {}", if has_gemini { "✓" } else { "not set" });
        if has_vllm {
            if let Some(api_base) = config.providers.vllm.api_base.as_ref() {
                println!("vLLM/Local: ✓ {}", api_base);
            } else {
                println!("vLLM/Local: not set");
            }
        } else {
            println!("vLLM/Local: not set");
        }

        // Voice transcription status
        let has_cloud =
            config.voice.transcription.enabled && !config.voice.transcription.api_key.is_empty();
        let has_local = config.voice.transcription.enabled
            && !config.voice.transcription.local_model_path.is_empty();
        let voice_status = match (has_local, has_cloud) {
            (true, true) => "✓ local + cloud fallback".to_string(),
            (true, false) => "✓ local only".to_string(),
            (false, true) => format!("✓ cloud only ({})", config.voice.transcription.model),
            (false, false) => "not configured".to_string(),
        };
        println!("Voice transcription: {}", voice_status);

        // Google status
        let gcfg = &config.tools.google;
        if gcfg.enabled {
            let google_authed = if !gcfg.client_id.is_empty() && !gcfg.client_secret.is_empty() {
                crate::auth::google::has_valid_credentials(
                    &gcfg.client_id,
                    &gcfg.client_secret,
                    Some(&gcfg.scopes),
                    None,
                )
            } else {
                false
            };
            let status_str = if google_authed {
                "✓ authenticated"
            } else {
                "not authenticated (run: oxicrab auth google)"
            };
            println!("Google: {}", status_str);
        } else {
            println!("Google: disabled");
        }
    }

    Ok(())
}

fn pairing_command(cmd: PairingCommands) -> Result<()> {
    match cmd {
        PairingCommands::List => {
            let store = crate::pairing::PairingStore::new()?;
            let pending = store.list_pending();
            if pending.is_empty() {
                println!("No pending pairing requests.");
            } else {
                println!("Pending pairing requests:");
                for req in pending {
                    let age_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        .saturating_sub(req.created_at);
                    let remaining = (15u64 * 60).saturating_sub(age_secs);
                    println!(
                        "  [{}] {}:{} (expires in {}m {}s)",
                        req.code,
                        req.channel,
                        req.sender_id,
                        remaining / 60,
                        remaining % 60,
                    );
                }
            }
            println!("\nPaired senders: {}", store.paired_count());
        }
        PairingCommands::Approve { code } => {
            let mut store = crate::pairing::PairingStore::new()?;
            match store.approve(&code)? {
                Some((channel, sender_id)) => {
                    println!("Approved: {}:{}", channel, sender_id);
                }
                None => {
                    println!("Invalid or expired code: {}", code);
                }
            }
        }
        PairingCommands::Revoke { channel, sender_id } => {
            let mut store = crate::pairing::PairingStore::new()?;
            if store.revoke(&channel, &sender_id)? {
                println!("Revoked: {}:{}", channel, sender_id);
            } else {
                println!("Sender not found: {}:{}", channel, sender_id);
            }
        }
    }
    Ok(())
}

fn credentials_command(cmd: CredentialCommands) -> Result<()> {
    use crate::config::credentials::{
        CREDENTIAL_ENV_VARS, CREDENTIAL_NAMES, detect_source, get_credential_value,
    };

    match cmd {
        CredentialCommands::Set { name, value } => {
            if !CREDENTIAL_NAMES.contains(&name.as_str()) {
                anyhow::bail!(
                    "unknown credential: {name}\nRun `oxicrab credentials list` to see valid names"
                );
            }

            #[cfg(not(feature = "keyring-store"))]
            {
                let _ = value;
                anyhow::bail!("keyring support not compiled (enable 'keyring-store' feature)");
            }

            #[cfg(feature = "keyring-store")]
            {
                let secret = if let Some(v) = value {
                    v
                } else {
                    use std::io::BufRead;
                    eprint!("Enter value for {name}: ");
                    let stdin = std::io::stdin();
                    let mut line = String::new();
                    stdin.lock().read_line(&mut line)?;
                    line.trim().to_string()
                };

                if secret.is_empty() {
                    anyhow::bail!("value cannot be empty");
                }

                crate::config::credentials::keyring_set(&name, &secret)?;
                println!("Stored {name} in keyring");
            }
        }
        CredentialCommands::Get { name } => {
            if !CREDENTIAL_NAMES.contains(&name.as_str()) {
                anyhow::bail!(
                    "unknown credential: {name}\nRun `oxicrab credentials list` to see valid names"
                );
            }

            #[cfg(not(feature = "keyring-store"))]
            {
                println!("{name}: keyring support not compiled");
            }

            #[cfg(feature = "keyring-store")]
            {
                let status = if crate::config::credentials::keyring_has(&name) {
                    "[set]"
                } else {
                    "[empty]"
                };
                println!("{name}: {status}");
            }
        }
        CredentialCommands::Delete { name } => {
            if !CREDENTIAL_NAMES.contains(&name.as_str()) {
                anyhow::bail!(
                    "unknown credential: {name}\nRun `oxicrab credentials list` to see valid names"
                );
            }

            #[cfg(not(feature = "keyring-store"))]
            anyhow::bail!("keyring support not compiled (enable 'keyring-store' feature)");

            #[cfg(feature = "keyring-store")]
            {
                crate::config::credentials::keyring_delete(&name)?;
                println!("Deleted {name} from keyring");
            }
        }
        CredentialCommands::List => {
            let config = load_config(None)?;

            println!("{:<30} Source", "Credential");
            println!("{}", "\u{2500}".repeat(50));

            for &name in CREDENTIAL_NAMES {
                let source = detect_source(name, &config);
                println!("{:<30} {}", name, source);
            }

            println!(
                "\n{} credential slot(s), {} populated",
                CREDENTIAL_NAMES.len(),
                CREDENTIAL_NAMES
                    .iter()
                    .filter(|&&n| {
                        get_credential_value(&config, n).is_some_and(|v: &str| !v.is_empty())
                    })
                    .count()
            );

            // Show env var hint
            let env_count = CREDENTIAL_ENV_VARS
                .iter()
                .filter(|(_, env)| std::env::var(env).ok().is_some_and(|v| !v.is_empty()))
                .count();
            if env_count > 0 {
                println!("{env_count} credential(s) from environment variables");
            }
        }
        CredentialCommands::Import => {
            #[cfg(not(feature = "keyring-store"))]
            anyhow::bail!("keyring support not compiled (enable 'keyring-store' feature)");

            #[cfg(feature = "keyring-store")]
            {
                let config = load_config(None)?;
                let mut imported = 0u32;

                for &name in CREDENTIAL_NAMES {
                    if let Some(val) = get_credential_value(&config, name)
                        && !val.is_empty()
                    {
                        match crate::config::credentials::keyring_set(name, val) {
                            Ok(()) => {
                                println!("  Imported {name}");
                                imported += 1;
                            }
                            Err(e) => {
                                eprintln!("  Failed to import {name}: {e}");
                            }
                        }
                    }
                }

                if imported == 0 {
                    println!("No credentials to import (all slots empty in config).");
                } else {
                    println!(
                        "\nImported {imported} credential(s) into keyring.\n\
                         You can now remove them from config.json if desired."
                    );
                }
            }
        }
    }
    Ok(())
}

#[cfg(feature = "channel-whatsapp")]
async fn whatsapp_login() -> Result<()> {
    use crate::utils::get_oxicrab_home;
    use std::sync::Arc;
    use whatsapp_rust::bot::Bot;
    use whatsapp_rust::store::SqliteStore;
    use whatsapp_rust::types::events::Event;
    use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;
    use whatsapp_rust_ureq_http_client::UreqHttpClient;

    println!("🤖 Starting WhatsApp authentication...");
    println!("Scan the QR code that appears below to connect.\n");

    // Determine session path
    let session_path = get_oxicrab_home()?.join("whatsapp");
    std::fs::create_dir_all(&session_path)?;

    let session_db = session_path.join("whatsapp.db");
    let session_db_str = session_db.to_string_lossy().to_string();

    // Create backend
    let backend = Arc::new(SqliteStore::new(&session_db_str).await?);

    // Create transport and HTTP client
    let transport_factory = TokioWebSocketTransportFactory::new();
    let http_client = UreqHttpClient::new();

    // Build bot with QR code display
    let bot = Bot::builder()
        .with_backend(backend)
        .with_transport_factory(transport_factory)
        .with_http_client(http_client)
        .on_event(|event, _client| async move {
            match event {
                Event::PairingQrCode { code, .. } => {
                    println!("\n🤖 WhatsApp QR Code:");
                    // Render QR code in terminal (compact)
                    match qrcode::QrCode::new(&code) {
                        Ok(qr) => {
                            let string = qr
                                .render::<char>()
                                .quiet_zone(false)
                                .module_dimensions(1, 1)
                                .build();
                            println!("{}", string);
                        }
                        Err(e) => {
                            eprintln!("Failed to generate QR code: {}. Raw code: {}", e, code);
                            println!("Raw QR code data: {}", code);
                        }
                    }
                }
                Event::PairingCode { code, timeout } => {
                    println!("\n🤖 WhatsApp Pairing Code: {}", code);
                    println!("Enter this code on your phone.");
                    println!("Code expires in: {:?}\n", timeout);
                }
                Event::PairSuccess(_pair_success) => {
                    println!("\n✅ WhatsApp connected successfully!");
                    println!("You can now close this window. The session is saved.\n");
                }
                Event::PairError(pair_error) => {
                    eprintln!("\n❌ WhatsApp pairing failed: {:?}", pair_error);
                }
                Event::Connected(_connected) => {
                    println!("\n✅ WhatsApp connected!\n");
                }
                Event::Disconnected(_disconnected) => {
                    eprintln!("\n⚠️  WhatsApp disconnected");
                }
                _ => {}
            }
        })
        .build()
        .await?;

    println!("Waiting for QR code...\n");

    // Run bot - this will display QR code and wait for pairing
    let mut bot_mut = bot;
    match bot_mut.run().await {
        Ok(handle) => {
            // Wait for pairing to complete or user interruption
            tokio::select! {
                _ = handle => {
                    println!("\nBot stopped.");
                }
                _ = tokio::signal::ctrl_c() => {
                    println!("\n\nInterrupted. Session saved - you can reconnect later.");
                }
            }
        }
        Err(e) => {
            anyhow::bail!("Failed to start WhatsApp bot: {}", e);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_workspace_templates() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().to_path_buf();

        create_workspace_templates(&workspace).unwrap();

        // Core template files should exist
        assert!(workspace.join("USER.md").exists());
        assert!(workspace.join("AGENTS.md").exists());
        assert!(workspace.join("TOOLS.md").exists());
        assert!(workspace.join("memory").join("MEMORY.md").exists());
    }

    #[test]
    fn test_create_workspace_templates_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().to_path_buf();

        create_workspace_templates(&workspace).unwrap();

        // Write custom content to USER.md
        let user_path = workspace.join("USER.md");
        std::fs::write(&user_path, "custom content").unwrap();

        // Second run should not overwrite
        create_workspace_templates(&workspace).unwrap();

        let content = std::fs::read_to_string(&user_path).unwrap();
        assert_eq!(content, "custom content");
    }

    #[test]
    fn test_create_workspace_templates_content() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().to_path_buf();

        create_workspace_templates(&workspace).unwrap();

        let agents = std::fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
        assert!(agents.contains("oxicrab"));
        assert!(agents.contains("Personality"));

        let tools = std::fs::read_to_string(workspace.join("TOOLS.md")).unwrap();
        assert!(tools.contains("Tool Notes"));

        let memory = std::fs::read_to_string(workspace.join("memory").join("MEMORY.md")).unwrap();
        assert!(memory.contains("Long-term Memory"));
    }
}
