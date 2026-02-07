use crate::agent::AgentLoop;
use crate::bus::MessageBus;
use crate::channels::manager::ChannelManager;
use crate::config::{load_config, Config};
use crate::cron::service::CronService;
use crate::cron::types::{CronJob, CronJobState, CronPayload, CronSchedule};
use crate::heartbeat::service::HeartbeatService;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

#[derive(Parser)]
#[command(name = "nanobot")]
#[command(about = "Personal AI Assistant")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize nanobot configuration and workspace
    Onboard,
    /// Run the gateway (channels + agent)
    Gateway {
        #[arg(long)]
        model: Option<String>,
    },
    /// Interact with the agent directly
    Agent {
        #[arg(short, long)]
        message: Option<String>,
        #[arg(short, long, default_value = "cli:default")]
        session: String,
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
    /// Show nanobot status
    Status,
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
        #[arg(long, short = 'd')]
        deliver: bool,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        channel: Option<String>,
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
        #[arg(long, short = 'd')]
        deliver: Option<bool>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        channel: Option<String>,
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
enum ChannelCommands {
    /// Show channel status
    Status,
    /// Link WhatsApp device via QR code
    Login,
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Onboard => {
            onboard().await?;
        }
        Commands::Gateway { model } => {
            gateway(model).await?;
        }
        Commands::Agent { message, session } => {
            agent(message, session).await?;
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
            status_command().await?;
        }
    }

    Ok(())
}

async fn onboard() -> Result<()> {
    println!("🤖 Initializing nanobot...");

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

    println!("\n🤖 nanobot is ready!");
    println!("\nNext steps:");
    println!("  1. Add your API key to ~/.nanobot/config.json");
    println!("     Get one at: https://openrouter.ai/keys");
    println!("  2. Chat: nanobot agent -m \"Hello!\"");

    Ok(())
}

fn create_workspace_templates(workspace: &std::path::Path) -> Result<()> {
    let templates = vec![
        (
            "AGENTS.md",
            r#"# Agent Instructions

You are a helpful AI assistant. Be concise, accurate, and friendly.

## Guidelines

- Always explain what you're doing before taking actions
- Ask for clarification when the request is ambiguous
- Use tools to help accomplish tasks
- Remember important information in your memory files
"#,
        ),
        (
            "SOUL.md",
            r#"# Soul

I am nanobot, a lightweight AI assistant.

## Personality

- Helpful and friendly
- Concise and to the point
- Curious and eager to learn

## Values

- Accuracy over speed
- User privacy and safety
- Transparency in actions
"#,
        ),
        (
            "USER.md",
            r#"# User

Information about the user goes here.

## Preferences

- Communication style: (casual/formal)
- Timezone: (your timezone)
- Language: (your preferred language)
"#,
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
        let memory_content = r#"# Long-term Memory

This file stores important information that should persist across sessions.

## User Information

(Important facts about the user)

## Preferences

(User preferences learned over time)

## Important Notes

(Things to remember)
"#;
        std::fs::write(&memory_file, memory_content)?;
        println!("  Created memory/MEMORY.md");
    }

    Ok(())
}

async fn gateway(model: Option<String>) -> Result<()> {
    tracing::info!("Loading configuration...");
    let config = load_config(None)?;
    let effective_model = model.as_deref().unwrap_or(&config.agents.defaults.model);
    tracing::info!("Configuration loaded. Using model: {}", effective_model);
    tracing::debug!("Workspace: {:?}", config.workspace_path());

    // Setup components
    let provider = setup_provider(&config, model.as_deref()).await?;
    let (inbound_tx, outbound_tx, outbound_rx, bus_for_channels) = setup_message_bus().await?;
    let cron = setup_cron_service()?;
    let agent = setup_agent(
        bus_for_channels.clone(),
        provider,
        &config,
        model.clone(),
        outbound_tx.clone(),
        cron.clone(),
    )
    .await?;
    setup_cron_callbacks(cron.clone(), agent.clone(), bus_for_channels.clone()).await?;
    let heartbeat = setup_heartbeat(&config, agent.clone()).await?;
    let channels = setup_channels(&config, inbound_tx)?;

    println!("Starting nanobot gateway...");
    println!("Enabled channels: {:?}", channels.enabled_channels());

    // Start services
    start_services(cron.clone(), heartbeat.clone()).await?;

    // Run agent and channels
    let agent_task = start_agent_loop(agent.clone());
    let channels_task = start_channels_loop(channels, outbound_rx);

    tracing::info!("All services started. Gateway is running.");

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

async fn setup_provider(
    config: &Config,
    model: Option<&str>,
) -> Result<Arc<dyn crate::providers::base::LLMProvider>> {
    let effective_model = model.unwrap_or(&config.agents.defaults.model);
    tracing::info!("Creating LLM provider for model: {}", effective_model);
    let provider = config.create_provider(model).await?;
    tracing::info!(
        "Provider created successfully. Default model: {}",
        provider.default_model()
    );
    Ok(provider)
}

async fn setup_message_bus() -> Result<(
    tokio::sync::mpsc::Sender<crate::bus::InboundMessage>,
    Arc<tokio::sync::mpsc::Sender<crate::bus::OutboundMessage>>,
    tokio::sync::mpsc::Receiver<crate::bus::OutboundMessage>,
    Arc<Mutex<MessageBus>>,
)> {
    tracing::debug!("Creating message bus...");
    let mut bus = MessageBus::default();
    let inbound_tx = bus.inbound_tx.clone();
    let outbound_tx = Arc::new(bus.outbound_tx.clone());
    let outbound_rx = bus
        .take_outbound_rx()
        .ok_or_else(|| anyhow::anyhow!("Outbound receiver already taken"))?;
    let bus_for_channels = Arc::new(Mutex::new(bus));
    tracing::debug!("Message bus initialized");
    Ok((inbound_tx, outbound_tx, outbound_rx, bus_for_channels))
}

fn setup_cron_service() -> Result<Arc<CronService>> {
    tracing::debug!("Initializing cron service...");
    let cron_store_path = crate::utils::get_nanobot_home()?
        .join("cron")
        .join("jobs.json");
    let cron = CronService::new(cron_store_path);
    tracing::debug!("Cron service initialized");
    Ok(Arc::new(cron))
}

async fn setup_agent(
    bus_for_channels: Arc<Mutex<MessageBus>>,
    provider: Arc<dyn crate::providers::base::LLMProvider>,
    config: &Config,
    model: Option<String>,
    outbound_tx: Arc<tokio::sync::mpsc::Sender<crate::bus::OutboundMessage>>,
    cron: Arc<CronService>,
) -> Result<Arc<AgentLoop>> {
    tracing::info!("Initializing agent loop...");
    tracing::debug!(
        "  - Max tool iterations: {}",
        config.agents.defaults.max_tool_iterations
    );
    tracing::debug!("  - Exec timeout: {}s", config.tools.exec.timeout);
    tracing::debug!(
        "  - Restrict to workspace: {}",
        config.tools.restrict_to_workspace
    );
    tracing::debug!(
        "  - Compaction enabled: {}",
        config.agents.defaults.compaction.enabled
    );
    let agent = Arc::new(
        AgentLoop::new(
            bus_for_channels,
            provider,
            config.workspace_path(),
            model,
            config.agents.defaults.max_tool_iterations,
            Some(config.tools.web.search.api_key.clone()),
            config.tools.exec.timeout,
            config.tools.restrict_to_workspace,
            config.agents.defaults.compaction.clone(),
            outbound_tx,
            Some(cron),
            Some(config.tools.google.clone()),
        )
        .await?,
    );
    tracing::info!("Agent loop initialized");
    Ok(agent)
}

async fn setup_cron_callbacks(
    cron: Arc<CronService>,
    agent: Arc<AgentLoop>,
    bus: Arc<Mutex<MessageBus>>,
) -> Result<()> {
    tracing::debug!("Setting up cron job callback...");
    let agent_clone = agent.clone();
    let bus_clone = bus.clone();
    cron.set_on_job(move |job| {
        tracing::debug!("Cron job triggered: {} - {}", job.id, job.payload.message);
        let agent = agent_clone.clone();
        let bus = bus_clone.clone();
        Box::pin(async move {
            let response = agent
                .process_direct(
                    &job.payload.message,
                    &format!("cron:{}", job.id),
                    &job.payload.channel.as_deref().unwrap_or("cli"),
                    &job.payload.to.as_deref().unwrap_or("direct"),
                )
                .await?;

            if job.payload.deliver {
                if let Some(channel) = &job.payload.channel {
                    if let Some(to) = &job.payload.to {
                        let bus_guard = bus.lock().await;
                        if let Err(e) = bus_guard
                            .publish_outbound(crate::bus::OutboundMessage {
                                channel: channel.clone(),
                                chat_id: to.clone(),
                                content: response.clone(),
                                reply_to: None,
                                media: vec![],
                                metadata: std::collections::HashMap::new(),
                            })
                            .await
                        {
                            tracing::error!("Failed to publish outbound message from cron: {}", e);
                        }
                    }
                }
            }

            Ok(Some(response))
        })
    })
    .await;
    Ok(())
}

async fn setup_heartbeat(
    config: &Config,
    agent: Arc<AgentLoop>,
) -> Result<Arc<HeartbeatService>> {
    tracing::debug!("Initializing heartbeat service...");
    tracing::debug!("  - Enabled: {}", config.agents.defaults.daemon.enabled);
    tracing::debug!("  - Interval: {}s", config.agents.defaults.daemon.interval);
    tracing::debug!(
        "  - Strategy file: {}",
        config.agents.defaults.daemon.strategy_file
    );
    let agent_for_heartbeat = agent.clone();
    let heartbeat = HeartbeatService::new(
        config.workspace_path(),
        Some(Arc::new(move |prompt| {
            tracing::debug!("Heartbeat triggered with prompt: {}", prompt);
            let agent = agent_for_heartbeat.clone();
            Box::pin(async move {
                agent
                    .process_direct(&prompt, "daemon", "cli", "direct")
                    .await
            })
        })),
        config.agents.defaults.daemon.interval,
        config.agents.defaults.daemon.enabled,
        None, // triage_provider
        config.agents.defaults.daemon.triage_model.clone(),
        config.agents.defaults.daemon.strategy_file.clone(),
        config.agents.defaults.daemon.cooldown_after_action,
    );
    tracing::debug!("Heartbeat service initialized");
    Ok(Arc::new(heartbeat))
}

fn setup_channels(
    config: &Config,
    inbound_tx: tokio::sync::mpsc::Sender<crate::bus::InboundMessage>,
) -> Result<ChannelManager> {
    tracing::info!("Initializing channels...");
    let channels = ChannelManager::new(config.clone(), Arc::new(inbound_tx))?;
    tracing::info!(
        "Channels initialized. Enabled: {:?}",
        channels.enabled_channels()
    );
    Ok(channels)
}

async fn start_services(cron: Arc<CronService>, heartbeat: Arc<HeartbeatService>) -> Result<()> {
    tracing::info!("Starting cron service...");
    cron.start().await?;
    tracing::info!("Cron service started");

    tracing::info!("Starting heartbeat service...");
    heartbeat.start().await?;
    tracing::info!("Heartbeat service started");
    Ok(())
}

fn start_agent_loop(agent: Arc<AgentLoop>) -> tokio::task::JoinHandle<()> {
    tracing::info!("Starting agent loop...");
    tokio::spawn(async move {
        tracing::info!("Agent loop running");
        match agent.run().await {
            Ok(_) => tracing::info!("Agent loop completed successfully"),
            Err(e) => tracing::error!("Agent loop exited with error: {}", e),
        }
    })
}

fn start_channels_loop(
    mut channels: ChannelManager,
    mut outbound_rx: tokio::sync::mpsc::Receiver<crate::bus::OutboundMessage>,
) -> tokio::task::JoinHandle<()> {
    tracing::info!("Starting all channels...");
    tokio::spawn(async move {
        match channels.start_all().await {
            Ok(_) => tracing::info!("All channels started successfully"),
            Err(e) => tracing::error!("Error starting channels: {}", e),
        }

        // Consume outbound messages and send to channels
        // Receiver already extracted to avoid lock contention
        loop {
            match outbound_rx.recv().await {
                Some(msg) => {
                    tracing::info!(
                        "Consumed outbound message: channel={}, chat_id={}, content_len={}",
                        msg.channel,
                        msg.chat_id,
                        msg.content.len()
                    );
                    if let Err(e) = channels.send(&msg).await {
                        tracing::error!("Error sending message to channels: {}", e);
                    } else {
                        tracing::info!("Successfully sent outbound message to channel manager");
                    }
                }
                None => {
                    tracing::warn!("Outbound message receiver closed");
                    break;
                }
            }
        }
        
        // Graceful shutdown - stop all channels when loop ends
        if let Err(e) = channels.stop_all().await {
            tracing::error!("Error stopping channels during shutdown: {}", e);
        }
    })
}

async fn agent(message: Option<String>, session: String) -> Result<()> {
    let config = load_config(None)?;

    // Create provider
    let provider = config.create_provider(None).await?;

    let bus = MessageBus::default();

    // Extract outbound sender for agent
    let outbound_tx = Arc::new(bus.outbound_tx.clone());
    let bus_for_agent = Arc::new(Mutex::new(bus));

    let agent = AgentLoop::new(
        bus_for_agent,
        provider,
        config.workspace_path(),
        None,
        config.agents.defaults.max_tool_iterations,
        Some(config.tools.web.search.api_key),
        config.tools.exec.timeout,
        config.tools.restrict_to_workspace,
        config.agents.defaults.compaction,
        outbound_tx,
        None, // No cron service for CLI agent mode
        Some(config.tools.google.clone()),
    )
    .await?;

    if let Some(msg) = message {
        let response = agent
            .process_direct(&msg, &session, "cli", "direct")
            .await?;
        println!("🤖 {}", response);
    } else {
        println!("🤖 Interactive mode (Ctrl+C to exit)\n");
        loop {
            use std::io::{self, BufRead, Write};
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
                .process_direct(input, &session, "cli", "direct")
                .await?;
            println!("\n🤖 {}\n", response);
        }
    }

    Ok(())
}

async fn cron_command(cmd: CronCommands) -> Result<()> {
    let _config = load_config(None)?;
    let cron_store_path = crate::utils::get_nanobot_home()?
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
                    let next_run = job
                        .state
                        .next_run_at_ms
                        .map(|ms| {
                            chrono::DateTime::from_timestamp(ms / 1000, 0)
                                .map(|dt| format!("{}", dt.format("%Y-%m-%d %H:%M:%S")))
                                .unwrap_or_else(|| "invalid timestamp".to_string())
                        })
                        .unwrap_or_else(|| "never".to_string());
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
            deliver: _deliver,
            to: _to,
            channel: _channel,
        } => {
            let schedule = if let Some(every_sec) = every {
                CronSchedule::Every {
                    every_ms: Some((every_sec * 1000) as i64),
                }
            } else if let Some(expr) = cron_expr {
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
                    deliver: false,
                    channel: None,
                    to: None,
                },
                state: CronJobState {
                    next_run_at_ms: None, // Will be computed by service
                    last_run_at_ms: None,
                    last_status: None,
                    last_error: None,
                },
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
                delete_after_run: false,
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
            deliver,
            to,
            channel,
        } => {
            let schedule = if let Some(every_sec) = every {
                Some(CronSchedule::Every {
                    every_ms: Some((every_sec * 1000) as i64),
                })
            } else if let Some(expr) = cron_expr {
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

            match cron
                .update_job(&id, name, message, schedule, deliver, channel, to)
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
        CronCommands::Run { id, force } => {
            if cron.run_job(&id, force).await? {
                println!("Job executed successfully.");
            } else {
                println!("Failed to run job {} (not found or disabled)", id);
            }
        }
    }

    Ok(())
}

#[allow(dead_code)]
fn _parse_schedule(s: &str) -> Result<CronSchedule> {
    // Try parsing as cron expression first
    if s.contains(' ') || s.contains('*') || s.contains('/') {
        return Ok(CronSchedule::Cron {
            expr: Some(s.to_string()),
            tz: None,
        });
    }

    // Try parsing as "every N seconds/minutes/hours"
    if let Some(every) = s.strip_prefix("every ") {
        let every_lower = every.to_lowercase();
        if let Some(num_str) = every_lower.strip_suffix("s") {
            if let Ok(num) = num_str.parse::<i64>() {
                return Ok(CronSchedule::Every {
                    every_ms: Some(num * 1000),
                });
            }
        }
        if let Some(num_str) = every_lower.strip_suffix("m") {
            if let Ok(num) = num_str.parse::<i64>() {
                return Ok(CronSchedule::Every {
                    every_ms: Some(num * 60 * 1000),
                });
            }
        }
        if let Some(num_str) = every_lower.strip_suffix("h") {
            if let Ok(num) = num_str.parse::<i64>() {
                return Ok(CronSchedule::Every {
                    every_ms: Some(num * 60 * 60 * 1000),
                });
            }
        }
    }

    // Try parsing as ISO timestamp
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        let ms = dt.timestamp_millis();
        return Ok(CronSchedule::At { at_ms: Some(ms) });
    }

    // Default to cron expression
    Ok(CronSchedule::Cron {
        expr: Some(s.to_string()),
        tz: None,
    })
}

async fn auth_command(cmd: AuthCommands) -> Result<()> {
    match cmd {
        AuthCommands::Google { port, headless } => {
            let config = load_config(None)?;
            let gcfg = &config.tools.google;

            if gcfg.client_id.is_empty() || gcfg.client_secret.is_empty() {
                eprintln!("Error: Google client_id and client_secret are not configured.");
                eprintln!("\nAdd them to ~/.nanobot/config.json under tools.google:");
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
            println!("  Tokens saved to ~/.nanobot/google_tokens.json");
            println!("\nMake sure tools.google.enabled is true in your config, then restart the gateway.");
        }
    }
    Ok(())
}

async fn channels_command(cmd: ChannelCommands) -> Result<()> {
    match cmd {
        ChannelCommands::Status => {
            let config = load_config(None)?;

            println!("Channel Status");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

            // WhatsApp
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
                let session_path = crate::utils::get_nanobot_home()
                    .map(|h| h.join("whatsapp").join("whatsapp.db"))
                    .unwrap_or_else(|_| PathBuf::from(".nanobot/whatsapp/whatsapp.db"));
                let session_exists = session_path.exists();
                println!(
                    "  Session: {} ({})",
                    session_path.display(),
                    if session_exists {
                        "exists"
                    } else {
                        "not paired - run 'nanobot channels login'"
                    }
                );
            }

            // Discord
            let dc = &config.channels.discord;
            println!(
                "Discord: {}",
                if dc.enabled {
                    "✓ enabled"
                } else {
                    "✗ disabled"
                }
            );
            if dc.enabled && !dc.token.is_empty() {
                let token_preview = &dc.token[..10.min(dc.token.len())];
                println!("  Token: {}...", token_preview);
            }

            // Telegram
            let tg = &config.channels.telegram;
            println!(
                "Telegram: {}",
                if tg.enabled {
                    "✓ enabled"
                } else {
                    "✗ disabled"
                }
            );
            if tg.enabled && !tg.token.is_empty() {
                let token_preview = &tg.token[..10.min(tg.token.len())];
                println!("  Token: {}...", token_preview);
            }

            // Slack
            let sl = &config.channels.slack;
            println!(
                "Slack: {}",
                if sl.enabled {
                    "✓ enabled"
                } else {
                    "✗ disabled"
                }
            );
            if sl.enabled && !sl.bot_token.is_empty() {
                let token_preview = &sl.bot_token[..10.min(sl.bot_token.len())];
                println!("  Bot Token: {}...", token_preview);
            }
        }
        ChannelCommands::Login => {
            whatsapp_login().await?;
        }
    }
    Ok(())
}

async fn status_command() -> Result<()> {
    let config = load_config(None)?;
    let config_path = crate::config::get_config_path()?;
    let workspace = config.workspace_path();

    println!("🤖 nanobot Status\n");

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
                "not authenticated (run: nanobot auth google)"
            };
            println!("Google: {}", status_str);
        } else {
            println!("Google: disabled");
        }
    }

    Ok(())
}

async fn whatsapp_login() -> Result<()> {
    use crate::utils::get_nanobot_home;
    use std::sync::Arc;
    use whatsapp_rust::bot::Bot;
    use whatsapp_rust::store::SqliteStore;
    use whatsapp_rust::types::events::Event;
    use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;
    use whatsapp_rust_ureq_http_client::UreqHttpClient;

    println!("🤖 Starting WhatsApp authentication...");
    println!("Scan the QR code that appears below to connect.\n");

    // Determine session path
    let session_path = get_nanobot_home()?.join("whatsapp");
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
