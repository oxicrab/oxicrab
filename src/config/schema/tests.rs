use super::*;

#[test]
fn test_default_config_validates() {
    let config = Config::default();
    assert!(config.validate().is_ok());
}

#[test]
fn test_invalid_zero_max_tokens() {
    let mut config = Config::default();
    config.agents.defaults.max_tokens = 0;
    assert!(config.validate().is_err());
}

#[test]
fn test_invalid_temperature_negative() {
    let mut config = Config::default();
    config.agents.defaults.temperature = -1.0;
    assert!(config.validate().is_err());
}

#[test]
fn test_invalid_temperature_too_high() {
    let mut config = Config::default();
    config.agents.defaults.temperature = 3.0;
    assert!(config.validate().is_err());
}

#[test]
fn test_invalid_zero_max_tool_iterations() {
    let mut config = Config::default();
    config.agents.defaults.max_tool_iterations = 0;
    assert!(config.validate().is_err());
}

#[test]
fn test_invalid_zero_port() {
    let mut config = Config::default();
    config.gateway.port = 0;
    assert!(config.validate().is_err());
}

#[test]
fn test_invalid_zero_exec_timeout() {
    let mut config = Config::default();
    config.tools.exec.timeout = 0;
    assert!(config.validate().is_err());
}

#[test]
fn test_invalid_zero_max_results() {
    let mut config = Config::default();
    config.tools.web.search.max_results = 0;
    assert!(config.validate().is_err());
}

#[test]
fn test_invalid_memory_purge_before_archive() {
    let mut config = Config::default();
    config.agents.defaults.memory.archive_after_days = 30;
    config.agents.defaults.memory.purge_after_days = 10; // less than archive
    assert!(config.validate().is_err());
}

#[test]
fn test_invalid_nan_temperature() {
    let mut config = Config::default();
    config.agents.defaults.temperature = f32::NAN;
    assert!(config.validate().is_err());
}

#[test]
fn test_invalid_infinity_temperature() {
    let mut config = Config::default();
    config.agents.defaults.temperature = f32::INFINITY;
    assert!(config.validate().is_err());
}

#[test]
fn test_invalid_nan_hybrid_weight() {
    let mut config = Config::default();
    config.agents.defaults.memory.hybrid_weight = f32::NAN;
    assert!(config.validate().is_err());
}

#[test]
fn test_invalid_model_cost_negative() {
    let mut config = Config::default();
    config.agents.defaults.cost_guard.model_costs.insert(
        "test-model".to_string(),
        ModelCost {
            input_per_million: -1.0,
            output_per_million: 3.0,
        },
    );
    assert!(config.validate().is_err());
}

#[test]
fn test_invalid_model_cost_nan() {
    let mut config = Config::default();
    config.agents.defaults.cost_guard.model_costs.insert(
        "test-model".to_string(),
        ModelCost {
            input_per_million: f64::NAN,
            output_per_million: 3.0,
        },
    );
    assert!(config.validate().is_err());
}

#[test]
fn test_telegram_enabled_without_token() {
    let mut config = Config::default();
    config.channels.telegram.enabled = true;
    config.channels.telegram.token = String::new();
    assert!(config.validate().is_err());
}

#[test]
fn test_discord_enabled_without_token() {
    let mut config = Config::default();
    config.channels.discord.enabled = true;
    config.channels.discord.token = String::new();
    assert!(config.validate().is_err());
}

#[test]
fn test_slack_enabled_without_tokens() {
    let mut config = Config::default();
    config.channels.slack.enabled = true;
    config.channels.slack.bot_token = String::new();
    assert!(config.validate().is_err());
}

#[test]
fn test_obsidian_enabled_zero_timeout() {
    let mut config = Config::default();
    config.tools.obsidian.enabled = true;
    config.tools.obsidian.api_url = "http://localhost:27123".into();
    config.tools.obsidian.api_key = "test-key".into();
    config.tools.obsidian.vault_name = "test".into();
    config.tools.obsidian.timeout = 0;
    assert!(config.validate().is_err());
}

#[test]
fn test_sandbox_paths_limit() {
    let mut config = Config::default();
    config.tools.exec.sandbox.additional_read_paths =
        (0..101).map(|i| format!("/path/{i}")).collect();
    assert!(config.validate().is_err());
}

#[test]
fn test_get_api_key_with_anthropic_model() {
    let mut config = Config::default();
    config.providers.anthropic.api_key = "test-anthropic-key".to_string();
    let api_key = config.get_api_key(Some("claude-sonnet-4-5-20250929"));
    assert_eq!(api_key, Some("test-anthropic-key"));
}

#[test]
fn test_get_api_key_with_openai_model() {
    let mut config = Config::default();
    config.providers.openai.api_key = "test-openai-key".to_string();
    let api_key = config.get_api_key(Some("gpt-4"));
    assert_eq!(api_key, Some("test-openai-key"));
}

#[test]
fn test_get_api_key_fallback_order() {
    let mut config = Config::default();
    config.providers.anthropic.api_key = "test-anthropic-key".to_string();
    // Call with no model parameter and no match - should fall back to first available
    let api_key = config.get_api_key(Some("unknown-model"));
    assert_eq!(api_key, Some("test-anthropic-key"));
}

#[test]
fn test_valid_dm_policy_values() {
    for policy in &[DmPolicy::Allowlist, DmPolicy::Pairing, DmPolicy::Open] {
        let mut config = Config::default();
        config.channels.telegram.dm_policy = policy.clone();
        assert!(
            config.validate().is_ok(),
            "policy '{:?}' should be valid",
            policy
        );
    }
}

#[test]
fn test_invalid_dm_policy_rejected() {
    // Invalid dm_policy values are now rejected at deserialization time (serde enum)
    let json = r#"{ "channels": { "telegram": { "enabled": false, "dmPolicy": "invalid" } } }"#;
    let result: Result<Config, _> = serde_json::from_str(json);
    assert!(
        result.is_err(),
        "invalid dmPolicy should be rejected by serde"
    );
}

#[test]
fn test_dm_policy_default_is_allowlist() {
    let config = Config::default();
    assert_eq!(config.channels.telegram.dm_policy, DmPolicy::Allowlist);
    assert_eq!(config.channels.discord.dm_policy, DmPolicy::Allowlist);
    assert_eq!(config.channels.slack.dm_policy, DmPolicy::Allowlist);
    assert_eq!(config.channels.whatsapp.dm_policy, DmPolicy::Allowlist);
    assert_eq!(config.channels.twilio.dm_policy, DmPolicy::Allowlist);
}

#[test]
fn test_dm_policy_deserializes_from_json() {
    let json = r#"{
        "channels": {
            "telegram": { "enabled": false, "dmPolicy": "pairing" },
            "discord": { "enabled": false, "dmPolicy": "open" }
        }
    }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(config.channels.telegram.dm_policy, DmPolicy::Pairing);
    assert_eq!(config.channels.discord.dm_policy, DmPolicy::Open);
    // Others default to Allowlist
    assert_eq!(config.channels.slack.dm_policy, DmPolicy::Allowlist);
}

#[test]
fn test_credential_helper_config_default() {
    let config = Config::default();
    assert!(config.credential_helper.command.is_empty());
    assert!(config.credential_helper.args.is_empty());
    assert!(config.credential_helper.format.is_empty());
}

#[test]
fn test_credential_helper_config_deserializes() {
    let json = r#"{
        "credentialHelper": {
            "command": "op",
            "args": ["--vault", "oxicrab"],
            "format": "1password"
        }
    }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(config.credential_helper.command, "op");
    assert_eq!(config.credential_helper.args, vec!["--vault", "oxicrab"]);
    assert_eq!(config.credential_helper.format, "1password");
}

#[test]
fn test_credential_helper_config_missing_is_default() {
    let json = r"{}";
    let config: Config = serde_json::from_str(json).unwrap();
    assert!(config.credential_helper.command.is_empty());
}
