use super::schema::{Config, CredentialHelperConfig};
use anyhow::{Context, Result};
use tracing::{debug, warn};

macro_rules! define_credentials {
    ($( $name:literal, $env:literal => $($path:ident).+ );* $(;)?) => {
        /// All known credential slot names.
        pub const CREDENTIAL_NAMES: &[&str] = &[$($name),*];

        /// (slot name, env var name) pairs.
        pub const CREDENTIAL_ENV_VARS: &[(&str, &str)] = &[$(($name, $env)),*];

        /// Get a mutable reference to a credential field by slot name.
        pub fn get_credential_field<'a>(config: &'a mut Config, name: &str) -> Option<&'a mut String> {
            match name {
                $($name => Some(&mut config.$($path).+),)*
                _ => None,
            }
        }

        /// Get the current value of a credential field by slot name.
        pub fn get_credential_value<'a>(config: &'a Config, name: &str) -> Option<&'a str> {
            match name {
                $($name => Some(config.$($path).+.as_str()),)*
                _ => None,
            }
        }

        /// Apply environment variable overrides.
        ///
        /// Any `OXICRAB_*` env var that is set and non-empty will overwrite the
        /// corresponding config field, allowing secrets to be injected without
        /// touching the config file (useful for containers and CI).
        pub fn apply_env_overrides(config: &mut Config) {
            $(
                if let Ok(val) = std::env::var($env) {
                    if !val.is_empty() {
                        config.$($path).+ = val;
                    }
                }
            )*
        }
    };
}

define_credentials! {
    // Provider API keys
    "anthropic-api-key",       "OXICRAB_ANTHROPIC_API_KEY"       => providers.anthropic.api_key;
    "openai-api-key",          "OXICRAB_OPENAI_API_KEY"          => providers.openai.api_key;
    "openrouter-api-key",      "OXICRAB_OPENROUTER_API_KEY"      => providers.openrouter.api_key;
    "gemini-api-key",          "OXICRAB_GEMINI_API_KEY"          => providers.gemini.api_key;
    "deepseek-api-key",        "OXICRAB_DEEPSEEK_API_KEY"        => providers.deepseek.api_key;
    "groq-api-key",            "OXICRAB_GROQ_API_KEY"            => providers.groq.api_key;
    "moonshot-api-key",        "OXICRAB_MOONSHOT_API_KEY"        => providers.moonshot.api_key;
    "zhipu-api-key",           "OXICRAB_ZHIPU_API_KEY"           => providers.zhipu.api_key;
    "dashscope-api-key",       "OXICRAB_DASHSCOPE_API_KEY"       => providers.dashscope.api_key;
    "vllm-api-key",            "OXICRAB_VLLM_API_KEY"            => providers.vllm.api_key;
    "ollama-api-key",          "OXICRAB_OLLAMA_API_KEY"          => providers.ollama.api_key;
    // OAuth tokens
    "anthropic-oauth-access",  "OXICRAB_ANTHROPIC_OAUTH_ACCESS"  => providers.anthropic_oauth.access_token;
    "anthropic-oauth-refresh", "OXICRAB_ANTHROPIC_OAUTH_REFRESH" => providers.anthropic_oauth.refresh_token;
    // Channel tokens
    "telegram-token",          "OXICRAB_TELEGRAM_TOKEN"          => channels.telegram.token;
    "discord-token",           "OXICRAB_DISCORD_TOKEN"           => channels.discord.token;
    "slack-bot-token",         "OXICRAB_SLACK_BOT_TOKEN"         => channels.slack.bot_token;
    "slack-app-token",         "OXICRAB_SLACK_APP_TOKEN"         => channels.slack.app_token;
    "twilio-account-sid",      "OXICRAB_TWILIO_ACCOUNT_SID"      => channels.twilio.account_sid;
    "twilio-auth-token",       "OXICRAB_TWILIO_AUTH_TOKEN"       => channels.twilio.auth_token;
    // Tool credentials
    "github-token",            "OXICRAB_GITHUB_TOKEN"            => tools.github.token;
    "weather-api-key",         "OXICRAB_WEATHER_API_KEY"         => tools.weather.api_key;
    "todoist-token",           "OXICRAB_TODOIST_TOKEN"           => tools.todoist.token;
    "web-search-api-key",      "OXICRAB_WEB_SEARCH_API_KEY"      => tools.web.search.api_key;
    "google-client-secret",    "OXICRAB_GOOGLE_CLIENT_SECRET"    => tools.google.client_secret;
    "obsidian-api-key",        "OXICRAB_OBSIDIAN_API_KEY"        => tools.obsidian.api_key;
    "media-radarr-api-key",    "OXICRAB_MEDIA_RADARR_API_KEY"    => tools.media.radarr.api_key;
    "media-sonarr-api-key",    "OXICRAB_MEDIA_SONARR_API_KEY"    => tools.media.sonarr.api_key;
    // Voice
    "transcription-api-key",   "OXICRAB_TRANSCRIPTION_API_KEY"   => voice.transcription.api_key;
}

// ---------------------------------------------------------------------------
// Credential helper (P3) — external process-based credential retrieval
// ---------------------------------------------------------------------------

/// Apply credential helper overrides. Only fills fields still empty after
/// env var overrides.
pub fn apply_credential_helper(config: &mut Config) {
    let helper = config.credential_helper.clone();
    if helper.command.is_empty() {
        return;
    }

    // Collect values first to avoid borrow conflicts
    let values: Vec<(&str, String)> = CREDENTIAL_NAMES
        .iter()
        .filter_map(|&name| {
            let current = get_credential_value(config, name)?;
            if !current.is_empty() {
                return None;
            }
            match fetch_from_helper(&helper, name) {
                Ok(value) if !value.is_empty() => {
                    debug!("loaded {name} from credential helper");
                    Some((name, value))
                }
                Ok(_) => None,
                Err(e) => {
                    warn!("credential helper failed for {name}: {e}");
                    None
                }
            }
        })
        .collect();

    for (name, value) in values {
        if let Some(field) = get_credential_field(config, name) {
            *field = value;
        }
    }
}

fn fetch_from_helper(helper: &CredentialHelperConfig, key: &str) -> Result<String> {
    let format = if helper.format.is_empty() {
        "json"
    } else {
        &helper.format
    };

    match format {
        "1password" => {
            let mut args = vec!["read".to_string(), format!("op://oxicrab/{key}")];
            args.extend(helper.args.iter().cloned());
            run_helper_process("op", &args, None)
        }
        "bitwarden" => {
            let mut args = vec![
                "get".to_string(),
                "password".to_string(),
                format!("oxicrab/{key}"),
            ];
            args.extend(helper.args.iter().cloned());
            run_helper_process("bw", &args, None)
        }
        "line" => {
            let mut args = helper.args.clone();
            args.push(key.to_string());
            run_helper_process(&helper.command, &args, None)
        }
        // "json" or any unrecognized format
        _ => {
            let stdin_data = serde_json::json!({"action": "get", "key": key}).to_string();
            let output = run_helper_process(&helper.command, &helper.args, Some(&stdin_data))?;
            let parsed: serde_json::Value =
                serde_json::from_str(&output).context("credential helper returned invalid JSON")?;
            Ok(parsed["value"].as_str().unwrap_or("").to_string())
        }
    }
}

fn run_helper_process(cmd: &str, args: &[String], stdin_data: Option<&str>) -> Result<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(cmd)
        .args(args)
        .stdin(if stdin_data.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {cmd}"))?;

    if let Some(data) = stdin_data
        && let Some(mut stdin) = child.stdin.take()
    {
        stdin.write_all(data.as_bytes())?;
    }

    let output = child.wait_with_output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("exited with {}: {}", output.status, stderr.trim())
    }
}

// ---------------------------------------------------------------------------
// Keyring (P2) — OS keychain credential storage
// ---------------------------------------------------------------------------

#[cfg(feature = "keyring-store")]
pub fn apply_keyring_overrides(config: &mut Config) {
    // Collect values first to avoid borrow conflicts
    let values: Vec<(&str, String)> = CREDENTIAL_NAMES
        .iter()
        .filter_map(|&name| {
            let current = get_credential_value(config, name)?;
            if !current.is_empty() {
                return None;
            }
            let entry = match keyring::Entry::new("oxicrab", name) {
                Ok(e) => e,
                Err(e) => {
                    debug!("keyring unavailable for {name}: {e}");
                    return None;
                }
            };
            match entry.get_password() {
                Ok(secret) if !secret.is_empty() => {
                    debug!("loaded {name} from keyring");
                    Some((name, secret))
                }
                _ => None,
            }
        })
        .collect();

    for (name, value) in values {
        if let Some(field) = get_credential_field(config, name) {
            *field = value;
        }
    }
}

#[cfg(feature = "keyring-store")]
pub fn keyring_set(name: &str, value: &str) -> Result<()> {
    let entry = keyring::Entry::new("oxicrab", name).context("keyring unavailable")?;
    entry
        .set_password(value)
        .context("failed to store credential in keyring")?;
    Ok(())
}

#[cfg(feature = "keyring-store")]
pub fn keyring_delete(name: &str) -> Result<()> {
    let entry = keyring::Entry::new("oxicrab", name).context("keyring unavailable")?;
    entry
        .delete_credential()
        .context("failed to delete credential from keyring")?;
    Ok(())
}

#[cfg(feature = "keyring-store")]
pub fn keyring_has(name: &str) -> bool {
    keyring::Entry::new("oxicrab", name)
        .ok()
        .and_then(|e| e.get_password().ok())
        .is_some_and(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// Source detection — for `oxicrab credentials list`
// ---------------------------------------------------------------------------

/// Detect which backend provided a credential value.
pub fn detect_source(name: &str, config: &Config) -> &'static str {
    // Find the env var name for this credential
    let env_var = CREDENTIAL_ENV_VARS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, e)| *e);

    // 1. Check env var
    if let Some(var) = env_var
        && let Ok(val) = std::env::var(var)
        && !val.is_empty()
    {
        return "env";
    }

    // 2. Check keyring
    #[cfg(feature = "keyring-store")]
    if keyring_has(name) {
        return "keyring";
    }

    // 3. Check if loaded config has a non-empty value (from config.json or helper)
    if let Some(val) = get_credential_value(config, name)
        && !val.is_empty()
    {
        if !config.credential_helper.command.is_empty() {
            return "config/helper";
        }
        return "config";
    }

    "[empty]"
}

#[cfg(test)]
mod tests;
