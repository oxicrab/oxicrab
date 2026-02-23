mod subcommands;

#[cfg(test)]
mod tests;

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
        /// Echo mode: test channel connectivity without an LLM
        #[arg(long)]
        echo: bool,
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
    /// Show memory and cost statistics
    Stats {
        #[command(subcommand)]
        cmd: StatsCommands,
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
enum StatsCommands {
    /// Show LLM cost summary
    Costs {
        /// Number of days to look back (default: 7)
        #[arg(long, short = 'd', default_value = "7")]
        days: u32,
    },
    /// Show memory search statistics
    Search,
    /// Show cost for today
    Today,
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
    /// Check if a credential exists (shows \[set\] or \[empty\])
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
        Commands::Gateway {
            model,
            provider,
            echo,
        } => {
            if echo {
                gateway_echo().await?;
            } else {
                gateway(model, provider).await?;
            }
        }
        Commands::Agent {
            message,
            session,
            provider,
        } => {
            subcommands::agent(message, session, provider).await?;
        }
        Commands::Cron { cmd } => {
            subcommands::cron_command(cmd).await?;
        }
        Commands::Auth { cmd } => {
            subcommands::auth_command(cmd).await?;
        }
        Commands::Channels { cmd } => {
            subcommands::channels_command(cmd).await?;
        }
        Commands::Status => {
            subcommands::status_command()?;
        }
        Commands::Doctor => {
            crate::cli::doctor::doctor_command().await?;
        }
        Commands::Pairing { cmd } => {
            subcommands::pairing_command(cmd)?;
        }
        Commands::Credentials { cmd } => {
            subcommands::credentials_command(cmd)?;
        }
        Commands::Stats { ref cmd } => {
            subcommands::stats_command(cmd)?;
        }
    }

    Ok(())
}

fn onboard() -> Result<()> {
    println!("\u{1f916} Initializing oxicrab...");

    let config_path = crate::config::get_config_path()?;
    if config_path.exists() {
        println!(
            "\u{26a0}\u{fe0f}  Config already exists at {}",
            config_path.display()
        );
        println!("Overwrite? (y/N): ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            return Ok(());
        }
    }

    let config = Config::default();
    crate::config::save_config(&config, Some(config_path.as_path()))?;
    println!("\u{2713} Created config at {}", config_path.display());

    let workspace = config.workspace_path();
    crate::utils::ensure_dir(&workspace)?;
    println!("\u{2713} Created workspace at {}", workspace.display());

    create_workspace_templates(&workspace)?;

    println!("\n\u{1f916} oxicrab is ready!");
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
    let memory_db_for_dlq = agent.memory_db();
    setup_cron_callbacks(
        cron.clone(),
        agent.clone(),
        bus_for_channels.clone(),
        memory_db_for_dlq,
    )
    .await?;
    let heartbeat = setup_heartbeat(&config, &agent);

    // Start HTTP API server (needs inbound_tx clone before channels takes ownership)
    let http_state = if config.gateway.enabled {
        let a2a_config = if config.gateway.a2a.enabled {
            Some(config.gateway.a2a.clone())
        } else {
            None
        };
        let (_http_task, state) = crate::gateway::start(
            &config.gateway.host,
            config.gateway.port,
            Arc::new(inbound_tx.clone()),
            Some(outbound_tx.clone()),
            config.gateway.webhooks.clone(),
            a2a_config,
        )
        .await?;
        Some(state)
    } else {
        info!("HTTP API server disabled");
        None
    };

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
    start_services(cron.clone(), heartbeat.clone()).await?;

    // Run agent and channels
    let agent_task = start_agent_loop(agent.clone());
    let channels_task = start_channels_loop(channels, outbound_rx, typing_rx, http_state);

    info!("All services started, gateway is running");

    // Handle shutdown
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nShutting down...");
            heartbeat.stop().await;
            cron.stop().await;
            agent.stop().await;
            // Channels will stop themselves when the task ends
        }
        _ = agent_task => {}
        _ = channels_task => {}
    }

    Ok(())
}

async fn gateway_echo() -> Result<()> {
    info!("Loading configuration for echo mode...");
    let config = load_config(None)?;

    let (inbound_tx, outbound_tx, outbound_rx, bus) = setup_message_bus(&config)?;
    // Create typing indicator channel (not used in echo mode but needed for channels)
    let (echo_typing_tx, typing_rx) = tokio::sync::mpsc::channel::<(String, String)>(100);
    drop(echo_typing_tx);

    // Start HTTP API server if enabled
    let http_state = if config.gateway.enabled {
        let (http_task, state) = crate::gateway::start(
            &config.gateway.host,
            config.gateway.port,
            Arc::new(inbound_tx.clone()),
            Some(outbound_tx.clone()),
            config.gateway.webhooks.clone(),
            None, // A2A not available in echo mode
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
    let mut inbound_rx = {
        let mut bus_guard = bus.lock().await;
        bus_guard
            .take_inbound_rx()
            .ok_or_else(|| anyhow::anyhow!("Inbound receiver already taken"))?
    };

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
                    .send(crate::bus::OutboundMessage {
                        channel: msg.channel,
                        chat_id: msg.chat_id,
                        content: echo_text,
                        reply_to: None,
                        media: vec![],
                        metadata: msg.metadata,
                    })
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
    bus: &Arc<Mutex<MessageBus>>,
) -> Result<Option<String>> {
    if job.payload.kind == "echo" {
        // Echo mode: deliver message directly without invoking the LLM
        for target in &job.payload.targets {
            let mut bus_guard = bus.lock().await;
            if let Err(e) = bus_guard
                .publish_outbound(crate::bus::OutboundMessage {
                    channel: target.channel.clone(),
                    chat_id: target.to.clone(),
                    content: job.payload.message.clone(),
                    reply_to: None,
                    media: vec![],
                    metadata: job.payload.origin_metadata.clone(),
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
            let mut bus_guard = bus.lock().await;
            if let Err(e) = bus_guard
                .publish_outbound(crate::bus::OutboundMessage {
                    channel: target.channel.clone(),
                    chat_id: target.to.clone(),
                    content: response.clone(),
                    reply_to: None,
                    media: vec![],
                    metadata: job.payload.origin_metadata.clone(),
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
                // Route HTTP API responses back to waiting HTTP handlers
                if let Some(ref state) = http_api_state
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
