use super::cli_types::{AuthCommands, PairingCommands};
use super::gateway_setup::{SetupAgentParams, setup_agent};
use crate::agent::AgentLoop;
use crate::bus::MessageBus;
use crate::config::load_config;
use anyhow::Result;
use std::sync::Arc;

pub(super) async fn agent(message: Option<String>, session: String) -> Result<()> {
    let config = load_config(None)?;
    crate::observability::init_metrics_exporter(&config);
    config.validate()?;

    let provider = crate::provider_factory::create_provider(&config, None, None)?;

    // Create shared leak detector with known secrets
    let leak_detector = {
        let mut detector = crate::safety::LeakDetector::new();
        let secrets = config.collect_secrets();
        if !secrets.is_empty() {
            detector.add_known_secrets(&secrets);
        }
        Arc::new(detector)
    };

    let bus = MessageBus::with_leak_detector(30, 60.0, 1000, 1000, leak_detector.clone());
    let outbound_tx = Arc::new(bus.outbound_tx.clone());
    let bus_for_agent = Arc::new(bus);

    let agent = setup_agent(
        SetupAgentParams {
            bus: bus_for_agent,
            provider,
            model: None,
            outbound_tx,
            cron: None,
            typing_tx: None,
            channels_config: None,
            memory_db: None,
            leak_detector: Some(leak_detector),
        },
        &config,
    )
    .await?;

    if let Some(msg) = message {
        let response = agent
            .process_direct(&msg, &session, "cli", "direct")
            .await?;
        println!("\u{1f916} {response}");
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
        println!("\n\u{1f916} {response}\n");
    }
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

            let scopes = gcfg.required_scopes();
            // Check if already authenticated
            if crate::auth::google::has_valid_credentials(
                &gcfg.client_id,
                &gcfg.client_secret,
                Some(&scopes),
                None,
            ) {
                println!("\u{2713} Already authenticated with Google.");
            }

            if headless {
                println!("\u{1f916} Starting Google OAuth2 flow (headless mode)...");
            } else {
                println!("\u{1f916} Starting Google OAuth2 flow...");
                println!(
                    "A browser window will open \u{2014} or fall back to manual mode if unavailable.\n"
                );
            }

            let db_path = config
                .workspace_path()
                .join("memory")
                .join("memory.sqlite3");
            let db = Arc::new(crate::agent::memory::memory_db::MemoryDB::new(&db_path)?);
            let oauth_store: &dyn oxicrab_core::credential_store::OAuthTokenStore = db.as_ref();
            let _creds = crate::auth::google::run_oauth_flow(
                &gcfg.client_id,
                &gcfg.client_secret,
                Some(&scopes),
                None,
                port,
                headless,
                Some(oauth_store),
            )
            .await?;

            println!("\n\u{2713} Google authentication successful!");
            println!("  Credentials saved. Restart the gateway to pick up changes.");
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
        println!("Model: {}", config.agents.defaults.model_routing.default);

        // Check API keys
        let has_openrouter = !config.providers.openrouter.api_key.is_empty();
        let has_anthropic = !config.providers.anthropic.api_key.is_empty();
        let has_openai = !config.providers.openai.api_key.is_empty();
        let has_gemini = !config.providers.gemini.api_key.is_empty();
        let has_vllm = config.providers.vllm.base.api_base.is_some();

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
            if let Some(api_base) = config.providers.vllm.base.api_base.as_ref() {
                println!("vLLM/Local: \u{2713} {api_base}");
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
        println!("Voice transcription: {voice_status}");

        // Google status
        let gcfg = &config.tools.google;
        if gcfg.is_configured() && gcfg.any_tool_enabled() {
            let scopes = gcfg.required_scopes();
            let google_authed = crate::auth::google::has_valid_credentials(
                &gcfg.client_id,
                &gcfg.client_secret,
                Some(&scopes),
                None,
            );
            let mut tools = Vec::new();
            if gcfg.gmail {
                tools.push("gmail");
            }
            if gcfg.calendar {
                tools.push("calendar");
            }
            if gcfg.tasks {
                tools.push("tasks");
            }
            let status_str = if google_authed {
                "\u{2713} authenticated"
            } else {
                "not authenticated (run: oxicrab auth google)"
            };
            println!("Google ({}): {status_str}", tools.join(", "));
        } else {
            println!("Google: not configured");
        }
    }

    Ok(())
}

pub(super) fn pairing_command(cmd: PairingCommands) -> Result<()> {
    let store = crate::pairing::PairingStore::open_default()?;

    match cmd {
        PairingCommands::List => {
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
        PairingCommands::Approve { code } => match store.approve(&code)? {
            Some((channel, sender_id)) => {
                println!("Approved: {channel}:{sender_id}");
            }
            None => {
                println!("Invalid or expired code: {code}");
            }
        },
        PairingCommands::Revoke { channel, sender_id } => {
            if store.revoke(&channel, &sender_id)? {
                println!("Revoked: {channel}:{sender_id}");
            } else {
                println!("Sender not found: {channel}:{sender_id}");
            }
        }
    }
    Ok(())
}
