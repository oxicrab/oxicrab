use anyhow::Result;
use tracing::debug;

#[derive(Debug)]
enum CheckResult {
    Pass(String),
    Fail(String),
    Skip(String),
}

impl CheckResult {
    fn label(&self) -> &'static str {
        match self {
            Self::Pass(_) => "PASS",
            Self::Fail(_) => "FAIL",
            Self::Skip(_) => "SKIP",
        }
    }

    fn detail(&self) -> &str {
        match self {
            Self::Pass(s) | Self::Fail(s) | Self::Skip(s) => s,
        }
    }

    fn is_fail(&self) -> bool {
        matches!(self, Self::Fail(_))
    }
}

fn print_check(name: &str, result: &CheckResult) {
    let label = result.label();
    let detail = result.detail();
    println!("  {:<6} {:<30} {}", label, name, detail);
}

fn check_config_exists() -> CheckResult {
    match crate::config::get_config_path() {
        Ok(path) => {
            if path.exists() {
                CheckResult::Pass(format!("{}", path.display()))
            } else {
                CheckResult::Fail(format!("not found at {}", path.display()))
            }
        }
        Err(e) => CheckResult::Fail(format!("cannot determine path: {}", e)),
    }
}

fn check_config_parses() -> CheckResult {
    match crate::config::load_config(None) {
        Ok(_) => CheckResult::Pass("valid JSON".to_string()),
        Err(e) => CheckResult::Fail(format!("{}", e)),
    }
}

fn check_config_validates() -> CheckResult {
    match crate::config::load_config(None) {
        Ok(config) => match config.validate() {
            Ok(()) => CheckResult::Pass("all checks passed".to_string()),
            Err(e) => CheckResult::Fail(format!("{}", e)),
        },
        Err(e) => CheckResult::Skip(format!("config did not parse: {}", e)),
    }
}

fn check_workspace() -> CheckResult {
    match crate::config::load_config(None) {
        Ok(config) => {
            let path = config.workspace_path();
            if path.exists() {
                // Check writable
                let test_file = path.join(".doctor_test");
                match std::fs::write(&test_file, "test") {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&test_file);
                        CheckResult::Pass(format!("{} (writable)", path.display()))
                    }
                    Err(e) => {
                        CheckResult::Fail(format!("{} (not writable: {})", path.display(), e))
                    }
                }
            } else {
                CheckResult::Fail(format!("{} (does not exist)", path.display()))
            }
        }
        Err(_) => CheckResult::Skip("config not available".to_string()),
    }
}

fn check_provider_keys() -> CheckResult {
    match crate::config::load_config(None) {
        Ok(config) => {
            let mut providers = Vec::new();
            if !config.providers.anthropic.api_key.is_empty() {
                providers.push("anthropic");
            }
            if config.providers.anthropic_oauth.enabled {
                providers.push("anthropic-oauth");
            }
            if !config.providers.openai.api_key.is_empty() {
                providers.push("openai");
            }
            if !config.providers.openrouter.api_key.is_empty() {
                providers.push("openrouter");
            }
            if !config.providers.gemini.api_key.is_empty() {
                providers.push("gemini");
            }
            if !config.providers.deepseek.api_key.is_empty() {
                providers.push("deepseek");
            }
            if !config.providers.groq.api_key.is_empty() {
                providers.push("groq");
            }
            if config.providers.vllm.api_base.is_some() {
                providers.push("vllm");
            }
            if !config.providers.ollama.api_key.is_empty()
                || config.providers.ollama.api_base.is_some()
            {
                providers.push("ollama");
            }

            if providers.is_empty() {
                CheckResult::Fail("no API keys or OAuth configured".to_string())
            } else {
                CheckResult::Pass(providers.join(", "))
            }
        }
        Err(_) => CheckResult::Skip("config not available".to_string()),
    }
}

async fn check_provider_connectivity() -> CheckResult {
    match crate::config::load_config(None) {
        Ok(config) => match config.create_provider(None).await {
            Ok(provider) => {
                let start = std::time::Instant::now();
                match provider.warmup().await {
                    Ok(()) => {
                        let elapsed = start.elapsed();
                        CheckResult::Pass(format!(
                            "{} (warmup: {:.0}ms)",
                            provider.default_model(),
                            elapsed.as_secs_f64() * 1000.0
                        ))
                    }
                    Err(e) => CheckResult::Fail(format!(
                        "{} (warmup failed: {})",
                        provider.default_model(),
                        e
                    )),
                }
            }
            Err(e) => CheckResult::Fail(format!("cannot create provider: {}", e)),
        },
        Err(_) => CheckResult::Skip("config not available".to_string()),
    }
}

#[allow(unused_variables)]
fn check_channels() -> Vec<(&'static str, CheckResult)> {
    let mut results = Vec::new();

    let Ok(config) = crate::config::load_config(None) else {
        return vec![(
            "channels",
            CheckResult::Skip("config not available".to_string()),
        )];
    };

    #[cfg(feature = "channel-telegram")]
    {
        let tg = &config.channels.telegram;
        let result = if !tg.enabled {
            CheckResult::Skip("disabled".to_string())
        } else if tg.token.is_empty() {
            CheckResult::Fail("enabled but token not set".to_string())
        } else {
            CheckResult::Pass("enabled, token configured".to_string())
        };
        results.push(("telegram", result));
    }
    #[cfg(not(feature = "channel-telegram"))]
    results.push(("telegram", CheckResult::Skip("not compiled".to_string())));

    #[cfg(feature = "channel-discord")]
    {
        let dc = &config.channels.discord;
        let result = if !dc.enabled {
            CheckResult::Skip("disabled".to_string())
        } else if dc.token.is_empty() {
            CheckResult::Fail("enabled but token not set".to_string())
        } else {
            CheckResult::Pass("enabled, token configured".to_string())
        };
        results.push(("discord", result));
    }
    #[cfg(not(feature = "channel-discord"))]
    results.push(("discord", CheckResult::Skip("not compiled".to_string())));

    #[cfg(feature = "channel-slack")]
    {
        let sl = &config.channels.slack;
        let result = if !sl.enabled {
            CheckResult::Skip("disabled".to_string())
        } else if sl.bot_token.is_empty() || sl.app_token.is_empty() {
            CheckResult::Fail("enabled but tokens not set".to_string())
        } else {
            CheckResult::Pass("enabled, tokens configured".to_string())
        };
        results.push(("slack", result));
    }
    #[cfg(not(feature = "channel-slack"))]
    results.push(("slack", CheckResult::Skip("not compiled".to_string())));

    #[cfg(feature = "channel-whatsapp")]
    {
        let wa = &config.channels.whatsapp;
        let result = if wa.enabled {
            CheckResult::Pass("enabled".to_string())
        } else {
            CheckResult::Skip("disabled".to_string())
        };
        results.push(("whatsapp", result));
    }
    #[cfg(not(feature = "channel-whatsapp"))]
    results.push(("whatsapp", CheckResult::Skip("not compiled".to_string())));

    #[cfg(feature = "channel-twilio")]
    {
        let tw = &config.channels.twilio;
        let result = if !tw.enabled {
            CheckResult::Skip("disabled".to_string())
        } else if tw.account_sid.is_empty() || tw.auth_token.is_empty() {
            CheckResult::Fail("enabled but credentials not set".to_string())
        } else {
            CheckResult::Pass("enabled, credentials configured".to_string())
        };
        results.push(("twilio", result));
    }
    #[cfg(not(feature = "channel-twilio"))]
    results.push(("twilio", CheckResult::Skip("not compiled".to_string())));

    results
}

fn check_voice() -> CheckResult {
    match crate::config::load_config(None) {
        Ok(config) => {
            let tc = &config.voice.transcription;
            if !tc.enabled {
                return CheckResult::Skip("disabled".to_string());
            }
            let has_cloud = !tc.api_key.is_empty();
            let has_local = !tc.local_model_path.is_empty();
            match (has_local, has_cloud) {
                (true, true) => CheckResult::Pass("local + cloud fallback".to_string()),
                (true, false) => CheckResult::Pass("local only".to_string()),
                (false, true) => CheckResult::Pass(format!("cloud only ({})", tc.model)),
                (false, false) => {
                    CheckResult::Fail("enabled but no backend configured".to_string())
                }
            }
        }
        Err(_) => CheckResult::Skip("config not available".to_string()),
    }
}

fn check_external_command(name: &str, args: &[&str]) -> CheckResult {
    match std::process::Command::new(name).args(args).output() {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let version = stdout.lines().next().unwrap_or("").trim().to_string();
                CheckResult::Pass(version)
            } else {
                CheckResult::Fail("command failed".to_string())
            }
        }
        Err(_) => CheckResult::Fail("not found in PATH".to_string()),
    }
}

fn check_config_file_permissions() -> CheckResult {
    let Ok(path) = crate::config::get_config_path() else {
        return CheckResult::Skip("cannot determine path".to_string());
    };
    if !path.exists() {
        return CheckResult::Skip("config file not found".to_string());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode.trailing_zeros() >= 6 {
                CheckResult::Pass(format!("{:o}", mode))
            } else {
                CheckResult::Fail(format!(
                    "{:o} (world/group readable — run: chmod 600 {})",
                    mode,
                    path.display()
                ))
            }
        } else {
            CheckResult::Skip("cannot read metadata".to_string())
        }
    }

    #[cfg(not(unix))]
    CheckResult::Skip("permission check not available on this platform".to_string())
}

fn check_config_dir_permissions() -> CheckResult {
    let Ok(path) = crate::config::get_config_path() else {
        return CheckResult::Skip("cannot determine path".to_string());
    };
    let Some(parent) = path.parent() else {
        return CheckResult::Skip("config has no parent dir".to_string());
    };
    if !parent.exists() {
        return CheckResult::Skip("config directory not found".to_string());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(parent) {
            let mode = meta.permissions().mode() & 0o777;
            if mode.trailing_zeros() >= 6 {
                CheckResult::Pass(format!("{:o}", mode))
            } else {
                CheckResult::Fail(format!(
                    "{:o} (world/group accessible — run: chmod 700 {})",
                    mode,
                    parent.display()
                ))
            }
        } else {
            CheckResult::Skip("cannot read metadata".to_string())
        }
    }

    #[cfg(not(unix))]
    CheckResult::Skip("permission check not available on this platform".to_string())
}

#[allow(unused_variables, unused_mut)]
fn check_empty_allowlists() -> CheckResult {
    let Ok(config) = crate::config::load_config(None) else {
        return CheckResult::Skip("config not available".to_string());
    };

    let mut open_channels: Vec<&str> = Vec::new();

    #[cfg(feature = "channel-telegram")]
    if config.channels.telegram.enabled && config.channels.telegram.allow_from.is_empty() {
        open_channels.push("telegram");
    }

    #[cfg(feature = "channel-discord")]
    if config.channels.discord.enabled && config.channels.discord.allow_from.is_empty() {
        open_channels.push("discord");
    }

    #[cfg(feature = "channel-slack")]
    if config.channels.slack.enabled && config.channels.slack.allow_from.is_empty() {
        open_channels.push("slack");
    }

    #[cfg(feature = "channel-twilio")]
    if config.channels.twilio.enabled && config.channels.twilio.allow_from.is_empty() {
        open_channels.push("twilio");
    }

    if open_channels.is_empty() {
        CheckResult::Pass("all enabled channels have allowlists or are disabled".to_string())
    } else {
        CheckResult::Fail(format!(
            "{} (use pairing or add \"*\" to allowFrom)",
            open_channels.join(", ")
        ))
    }
}

fn check_keyring() -> CheckResult {
    #[cfg(not(feature = "keyring-store"))]
    return CheckResult::Skip("not compiled (enable 'keyring-store' feature)".to_string());

    #[cfg(feature = "keyring-store")]
    match keyring::Entry::new("oxicrab", "_doctor_probe") {
        Ok(_) => CheckResult::Pass("available".to_string()),
        Err(e) => CheckResult::Fail(format!("unavailable: {e}")),
    }
}

fn check_credential_helper() -> CheckResult {
    let Ok(config) = crate::config::load_config(None) else {
        return CheckResult::Skip("config not available".to_string());
    };

    if config.credential_helper.command.is_empty() {
        return CheckResult::Skip("not configured".to_string());
    }

    let cmd = &config.credential_helper.command;
    match which::which(cmd) {
        Ok(path) => CheckResult::Pass(format!("{} ({})", cmd, path.display())),
        Err(_) => CheckResult::Fail(format!("{cmd} not found in PATH")),
    }
}

fn check_pairing_store() -> CheckResult {
    if !crate::pairing::PairingStore::store_exists() {
        return CheckResult::Skip("not initialized (run oxicrab pairing list)".to_string());
    }

    match crate::pairing::PairingStore::new() {
        Ok(store) => {
            let paired = store.paired_count();
            let pending = store.list_pending().len();
            CheckResult::Pass(format!("{} paired sender(s), {} pending", paired, pending))
        }
        Err(e) => CheckResult::Fail(format!("cannot load: {}", e)),
    }
}

fn check_mcp_servers() -> CheckResult {
    match crate::config::load_config(None) {
        Ok(config) => {
            if config.tools.mcp.servers.is_empty() {
                return CheckResult::Skip("no servers configured".to_string());
            }
            let enabled: Vec<&str> = config
                .tools
                .mcp
                .servers
                .iter()
                .filter(|(_, s)| s.enabled)
                .map(|(name, _)| name.as_str())
                .collect();
            if enabled.is_empty() {
                CheckResult::Skip("all servers disabled".to_string())
            } else {
                CheckResult::Pass(format!(
                    "{} server(s): {}",
                    enabled.len(),
                    enabled.join(", ")
                ))
            }
        }
        Err(_) => CheckResult::Skip("config not available".to_string()),
    }
}

pub async fn doctor_command() -> Result<()> {
    println!("oxicrab doctor\n");
    println!("{}", "=".repeat(60));

    let mut pass_count = 0u32;
    let mut fail_count = 0u32;
    let mut skip_count = 0u32;

    let mut record = |name: &str, result: &CheckResult| {
        print_check(name, result);
        match result {
            CheckResult::Pass(_) => pass_count += 1,
            CheckResult::Fail(_) => fail_count += 1,
            CheckResult::Skip(_) => skip_count += 1,
        }
    };

    // Core checks
    println!("\n  Core");
    println!("  {}", "-".repeat(56));

    let r = check_config_exists();
    record("Config file", &r);

    let r = check_config_parses();
    record("Config parses", &r);

    let r = check_config_validates();
    record("Config validates", &r);

    let r = check_workspace();
    record("Workspace", &r);

    // Provider checks
    println!("\n  Provider");
    println!("  {}", "-".repeat(56));

    let r = check_provider_keys();
    record("API keys", &r);

    debug!("checking provider connectivity...");
    let r = check_provider_connectivity().await;
    record("Provider connectivity", &r);

    // Channel checks
    println!("\n  Channels");
    println!("  {}", "-".repeat(56));

    for (name, result) in check_channels() {
        record(name, &result);
    }

    // Voice
    println!("\n  Voice");
    println!("  {}", "-".repeat(56));

    let r = check_voice();
    record("Transcription", &r);

    // External tools
    println!("\n  External Tools");
    println!("  {}", "-".repeat(56));

    let r = check_external_command("ffmpeg", &["-version"]);
    record("ffmpeg", &r);

    let r = check_external_command("git", &["--version"]);
    record("git", &r);

    // Security
    println!("\n  Security");
    println!("  {}", "-".repeat(56));

    let r = check_config_file_permissions();
    record("Config file permissions", &r);

    let r = check_config_dir_permissions();
    record("Config dir permissions", &r);

    let r = check_keyring();
    record("Keyring", &r);

    let r = check_credential_helper();
    record("Credential helper", &r);

    let r = check_empty_allowlists();
    record("Empty allowlists", &r);

    let r = check_pairing_store();
    record("Pairing store", &r);

    // MCP
    println!("\n  MCP");
    println!("  {}", "-".repeat(56));

    let r = check_mcp_servers();
    record("MCP servers", &r);

    // Summary
    println!("\n{}", "=".repeat(60));
    println!(
        "  {} passed, {} failed, {} skipped",
        pass_count, fail_count, skip_count
    );

    if fail_count > 0 {
        println!("\n  Some checks failed. Review the output above.");
    } else {
        println!("\n  All checks passed!");
    }

    // Return error if any critical checks failed
    let critical_fail = {
        let config_exists = check_config_exists();
        config_exists.is_fail()
    };
    if critical_fail {
        anyhow::bail!("critical checks failed");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_result_variants() {
        let pass = CheckResult::Pass("ok".to_string());
        assert_eq!(pass.label(), "PASS");
        assert_eq!(pass.detail(), "ok");
        assert!(!pass.is_fail());

        let fail = CheckResult::Fail("bad".to_string());
        assert_eq!(fail.label(), "FAIL");
        assert!(fail.is_fail());

        let skip = CheckResult::Skip("n/a".to_string());
        assert_eq!(skip.label(), "SKIP");
        assert!(!skip.is_fail());
    }

    #[test]
    fn test_check_git_available() {
        let result = check_external_command("git", &["--version"]);
        // git should be available in dev environments
        assert!(matches!(result, CheckResult::Pass(_)));
    }

    #[test]
    fn test_check_config_default_parses() {
        // Default config should always parse (even if file doesn't exist,
        // load_config falls back to defaults)
        let result = check_config_parses();
        // This may pass or fail depending on environment, but shouldn't panic
        let _ = result;
    }
}
