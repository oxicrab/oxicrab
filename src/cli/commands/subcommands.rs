use super::{
    AgentLoop, AuthCommands, ChannelCommands, Context, CredentialCommands, CronCommands, CronJob,
    CronJobState, CronPayload, CronSchedule, CronService, MessageBus, PairingCommands,
    SetupAgentParams, StatsCommands, SystemTime, UNIX_EPOCH, setup_agent,
};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::load_config;

pub(super) async fn agent(
    message: Option<String>,
    session: String,
    provider: Option<String>,
) -> Result<()> {
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
        println!("\u{1f916} {}", response);
    } else {
        interactive_repl(&agent, &session).await?;
    }

    Ok(())
}

async fn interactive_repl(agent: &AgentLoop, session: &str) -> Result<()> {
    use std::io::{self, BufRead, Write};

    println!("\u{1f916} Interactive mode (Ctrl+C to exit)\n");
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
        println!("\n\u{1f916} {}\n", response);
    }
}

#[allow(clippy::too_many_lines)]
pub(super) async fn cron_command(cmd: CronCommands) -> Result<()> {
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
                    origin_metadata: std::collections::HashMap::new(),
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

pub(super) async fn auth_command(cmd: AuthCommands) -> Result<()> {
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
                println!("\u{2713} Already authenticated with Google.");
                // In real implementation, would prompt for re-auth
            }

            if headless {
                println!("\u{1f916} Starting Google OAuth2 flow (headless mode)...");
            } else {
                println!("\u{1f916} Starting Google OAuth2 flow...");
                println!(
                    "A browser window will open \u{2014} or fall back to manual mode if unavailable.\n"
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

            println!("\n\u{2713} Google authentication successful!");
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
pub(super) async fn channels_command(cmd: ChannelCommands) -> Result<()> {
    match cmd {
        ChannelCommands::Status => {
            let config = load_config(None)?;

            println!("Channel Status");
            println!(
                "\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}"
            );

            // WhatsApp
            #[cfg(feature = "channel-whatsapp")]
            {
                let wa = &config.channels.whatsapp;
                println!(
                    "WhatsApp: {}",
                    if wa.enabled {
                        "\u{2713} enabled"
                    } else {
                        "\u{2717} disabled"
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
                        "\u{2713} enabled"
                    } else {
                        "\u{2717} disabled"
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
                        "\u{2713} enabled"
                    } else {
                        "\u{2717} disabled"
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
                        "\u{2713} enabled"
                    } else {
                        "\u{2717} disabled"
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

pub(super) fn status_command() -> Result<()> {
    let config = load_config(None)?;
    let config_path = crate::config::get_config_path()?;
    let workspace = config.workspace_path();

    println!("\u{1f916} oxicrab Status\n");

    println!(
        "Config: {} {}",
        config_path.display(),
        if config_path.exists() {
            "\u{2713}"
        } else {
            "\u{2717}"
        }
    );
    println!(
        "Workspace: {} {}",
        workspace.display(),
        if workspace.exists() {
            "\u{2713}"
        } else {
            "\u{2717}"
        }
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
            if has_openrouter {
                "\u{2713}"
            } else {
                "not set"
            }
        );
        println!(
            "Anthropic API: {}",
            if has_anthropic { "\u{2713}" } else { "not set" }
        );
        println!(
            "OpenAI API: {}",
            if has_openai { "\u{2713}" } else { "not set" }
        );
        println!(
            "Gemini API: {}",
            if has_gemini { "\u{2713}" } else { "not set" }
        );
        if has_vllm {
            if let Some(api_base) = config.providers.vllm.api_base.as_ref() {
                println!("vLLM/Local: \u{2713} {}", api_base);
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
            (true, true) => "\u{2713} local + cloud fallback".to_string(),
            (true, false) => "\u{2713} local only".to_string(),
            (false, true) => format!("\u{2713} cloud only ({})", config.voice.transcription.model),
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
                "\u{2713} authenticated"
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

pub(super) fn pairing_command(cmd: PairingCommands) -> Result<()> {
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

pub(super) fn credentials_command(cmd: CredentialCommands) -> Result<()> {
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

pub(super) fn stats_command(cmd: &StatsCommands) -> Result<()> {
    let config = load_config(None)?;
    let workspace = config.workspace_path();
    let db_path = workspace.join("memory").join("memory.sqlite3");

    if !db_path.exists() {
        anyhow::bail!(
            "memory database not found at {}. Run the agent first to initialize it.",
            db_path.display()
        );
    }

    let db = crate::agent::memory::MemoryDB::new(&db_path)?;

    match cmd {
        StatsCommands::Today => {
            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
            let daily = db.get_daily_cost(&today)?;
            println!(
                "Cost today ({}): {:.2} cents (${:.4})",
                today,
                daily,
                daily / 100.0
            );
        }
        StatsCommands::Costs { days } => {
            let since = (chrono::Utc::now().date_naive()
                - chrono::Duration::days(i64::from(*days)))
            .format("%Y-%m-%d")
            .to_string();
            let summary = db.get_cost_summary(&since)?;

            if summary.is_empty() {
                println!("No cost data in the last {} days.", days);
                return Ok(());
            }

            println!(
                "{:<12} {:<30} {:>8} {:>10} {:>10} {:>6}",
                "Date", "Model", "Cents", "Input", "Output", "Calls"
            );
            println!("{}", "\u{2500}".repeat(80));

            let mut total_cents = 0.0;
            let mut total_calls = 0i64;
            for row in &summary {
                println!(
                    "{:<12} {:<30} {:>8.2} {:>10} {:>10} {:>6}",
                    row.date,
                    row.model,
                    row.total_cents,
                    row.total_input_tokens,
                    row.total_output_tokens,
                    row.call_count,
                );
                total_cents += row.total_cents;
                total_calls += row.call_count;
            }

            println!("{}", "\u{2500}".repeat(80));
            println!(
                "Total: {:.2} cents (${:.4}) across {} calls",
                total_cents,
                total_cents / 100.0,
                total_calls
            );
        }
        StatsCommands::Search => {
            let stats = db.get_search_stats()?;
            println!("Memory Search Statistics");
            println!("{}", "\u{2500}".repeat(40));
            println!("Total searches:       {}", stats.total_searches);
            println!("Total hits:           {}", stats.total_hits);
            println!("Avg results/search:   {:.1}", stats.avg_results_per_search);

            let top = db.get_top_sources(10)?;
            if !top.is_empty() {
                println!("\nTop Sources by Hit Count:");
                for (key, count) in &top {
                    println!("  {:<30} {} hits", key, count);
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

    println!("\u{1f916} Starting WhatsApp authentication...");
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
                    println!("\n\u{1f916} WhatsApp QR Code:");
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
                    println!("\n\u{1f916} WhatsApp Pairing Code: {}", code);
                    println!("Enter this code on your phone.");
                    println!("Code expires in: {:?}\n", timeout);
                }
                Event::PairSuccess(_pair_success) => {
                    println!("\n\u{2705} WhatsApp connected successfully!");
                    println!("You can now close this window. The session is saved.\n");
                }
                Event::PairError(pair_error) => {
                    eprintln!("\n\u{274c} WhatsApp pairing failed: {:?}", pair_error);
                }
                Event::Connected(_connected) => {
                    println!("\n\u{2705} WhatsApp connected!\n");
                }
                Event::Disconnected(_disconnected) => {
                    eprintln!("\n\u{26a0}\u{fe0f}  WhatsApp disconnected");
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
