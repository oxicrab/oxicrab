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

// -----------------------------------------------------------------------
// collect_secrets
// -----------------------------------------------------------------------

#[test]
fn test_collect_secrets_skips_empty_values() {
    let config = Config::default();
    // Default config has all secrets empty
    let secrets = config.collect_secrets();
    assert!(
        secrets.is_empty(),
        "default config should have no secrets, got: {:?}",
        secrets
    );
}

#[test]
fn test_collect_secrets_returns_non_empty_and_includes_custom_headers() {
    let mut config = Config::default();
    config.providers.anthropic.api_key = "sk-ant-test".to_string();
    config.channels.telegram.token = "tg-token-123".to_string();
    config
        .providers
        .openai
        .headers
        .insert("X-Custom-Auth".to_string(), "bearer-xyz".to_string());
    // Add an empty header that should be skipped
    config
        .providers
        .gemini
        .headers
        .insert("X-Empty".to_string(), String::new());

    let secrets = config.collect_secrets();
    let names: Vec<&str> = secrets.iter().map(|(n, _)| *n).collect();
    assert!(names.contains(&"anthropic_api_key"));
    assert!(names.contains(&"telegram_token"));
    assert!(names.contains(&"provider_header"));

    // Verify actual values
    let anthropic = secrets.iter().find(|(n, _)| *n == "anthropic_api_key");
    assert_eq!(anthropic.unwrap().1, "sk-ant-test");

    let header = secrets.iter().find(|(n, _)| *n == "provider_header");
    assert_eq!(header.unwrap().1, "bearer-xyz");

    // Empty header should not appear
    let header_count = secrets
        .iter()
        .filter(|(n, _)| *n == "provider_header")
        .count();
    assert_eq!(header_count, 1);
}

// -----------------------------------------------------------------------
// should_use_prompt_guided_tools
// -----------------------------------------------------------------------

#[test]
fn test_prompt_guided_tools_explicit_ollama_provider() {
    let mut config = Config::default();
    config.agents.defaults.provider = Some("ollama".to_string());
    config.providers.ollama.prompt_guided_tools = true;
    assert!(config.should_use_prompt_guided_tools("llama3"));
}

#[test]
fn test_prompt_guided_tools_prefix_notation() {
    let mut config = Config::default();
    config.providers.ollama.prompt_guided_tools = true;
    // No explicit provider set; prefix notation should be detected
    assert!(config.should_use_prompt_guided_tools("ollama/llama3"));
}

#[test]
fn test_prompt_guided_tools_known_model_returns_false() {
    let config = Config::default();
    // claude-sonnet is inferred as anthropic, which never uses prompt-guided tools
    assert!(!config.should_use_prompt_guided_tools("claude-sonnet-4-5-20250929"));
    assert!(!config.should_use_prompt_guided_tools("gpt-4"));
    assert!(!config.should_use_prompt_guided_tools("gemini-pro"));
}

// -----------------------------------------------------------------------
// Validation: max_tokens too large
// -----------------------------------------------------------------------

#[test]
fn test_invalid_max_tokens_too_large() {
    let mut config = Config::default();
    config.agents.defaults.max_tokens = 2_000_000;
    let err = config.validate().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("unreasonably large"),
        "expected 'unreasonably large' in: {msg}"
    );
}

// -----------------------------------------------------------------------
// Validation: max_tool_iterations too large
// -----------------------------------------------------------------------

#[test]
fn test_invalid_max_tool_iterations_too_large() {
    let mut config = Config::default();
    config.agents.defaults.max_tool_iterations = 2000;
    let err = config.validate().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("unreasonably large"),
        "expected 'unreasonably large' in: {msg}"
    );
}

// -----------------------------------------------------------------------
// Validation: compaction thresholdTokens=0 when enabled
// -----------------------------------------------------------------------

#[test]
fn test_invalid_compaction_threshold_zero() {
    let mut config = Config::default();
    config.agents.defaults.compaction.enabled = true;
    config.agents.defaults.compaction.threshold_tokens = 0;
    let err = config.validate().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("thresholdTokens must be > 0"),
        "expected thresholdTokens error in: {msg}"
    );
}

// -----------------------------------------------------------------------
// Validation: compaction keepRecent=0 when enabled
// -----------------------------------------------------------------------

#[test]
fn test_invalid_compaction_keep_recent_zero() {
    let mut config = Config::default();
    config.agents.defaults.compaction.enabled = true;
    config.agents.defaults.compaction.keep_recent = 0;
    let err = config.validate().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("keepRecent must be > 0"),
        "expected keepRecent error in: {msg}"
    );
}

// -----------------------------------------------------------------------
// Validation: checkpoint interval=0 when enabled
// -----------------------------------------------------------------------

#[test]
fn test_invalid_checkpoint_interval_zero() {
    let mut config = Config::default();
    config.agents.defaults.compaction.checkpoint.enabled = true;
    config
        .agents
        .defaults
        .compaction
        .checkpoint
        .interval_iterations = 0;
    let err = config.validate().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("intervalIterations must be > 0"),
        "expected intervalIterations error in: {msg}"
    );
}

// -----------------------------------------------------------------------
// Validation: cognitive thresholds misordered
// -----------------------------------------------------------------------

#[test]
fn test_invalid_cognitive_thresholds_misordered() {
    let mut config = Config::default();
    config.agents.defaults.cognitive.enabled = true;
    config.agents.defaults.cognitive.gentle_threshold = 20;
    config.agents.defaults.cognitive.firm_threshold = 10;
    config.agents.defaults.cognitive.urgent_threshold = 30;
    let err = config.validate().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("gentle < firm < urgent"),
        "expected threshold ordering error in: {msg}"
    );
}

// -----------------------------------------------------------------------
// Validation: twilio enabled with missing fields
// -----------------------------------------------------------------------

#[test]
fn test_invalid_twilio_missing_account_sid() {
    let mut config = Config::default();
    config.channels.twilio.enabled = true;
    config.channels.twilio.account_sid = String::new();
    config.channels.twilio.auth_token = "token".into();
    config.channels.twilio.webhook_url = "https://example.com".into();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("accountSid"));
}

#[test]
fn test_invalid_twilio_missing_auth_token() {
    let mut config = Config::default();
    config.channels.twilio.enabled = true;
    config.channels.twilio.account_sid = "AC123".into();
    config.channels.twilio.auth_token = String::new();
    config.channels.twilio.webhook_url = "https://example.com".into();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("authToken"));
}

#[test]
fn test_invalid_twilio_missing_webhook_url() {
    let mut config = Config::default();
    config.channels.twilio.enabled = true;
    config.channels.twilio.account_sid = "AC123".into();
    config.channels.twilio.auth_token = "token".into();
    config.channels.twilio.webhook_url = String::new();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("webhookUrl"));
}

#[test]
fn test_invalid_twilio_webhook_port_zero() {
    let mut config = Config::default();
    config.channels.twilio.enabled = true;
    config.channels.twilio.account_sid = "AC123".into();
    config.channels.twilio.auth_token = "token".into();
    config.channels.twilio.webhook_url = "https://example.com".into();
    config.channels.twilio.webhook_port = 0;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("webhookPort"));
}

#[test]
fn test_invalid_twilio_bad_webhook_path() {
    let mut config = Config::default();
    config.channels.twilio.enabled = true;
    config.channels.twilio.account_sid = "AC123".into();
    config.channels.twilio.auth_token = "token".into();
    config.channels.twilio.webhook_url = "https://example.com".into();
    config.channels.twilio.webhook_path = "no-leading-slash".into();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("webhookPath"));
}

// -----------------------------------------------------------------------
// Validation: browser timeout=0
// -----------------------------------------------------------------------

#[test]
fn test_invalid_browser_timeout_zero() {
    let mut config = Config::default();
    config.tools.browser.timeout = 0;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("browser.timeout"));
}

// -----------------------------------------------------------------------
// Validation: obsidian enabled missing api_url, api_key, vault_name
// -----------------------------------------------------------------------

#[test]
fn test_invalid_obsidian_enabled_missing_api_url() {
    let mut config = Config::default();
    config.tools.obsidian.enabled = true;
    config.tools.obsidian.api_url = String::new();
    config.tools.obsidian.api_key = "key".into();
    config.tools.obsidian.vault_name = "vault".into();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("apiUrl"));
}

#[test]
fn test_invalid_obsidian_enabled_missing_api_key() {
    let mut config = Config::default();
    config.tools.obsidian.enabled = true;
    config.tools.obsidian.api_url = "http://localhost:27123".into();
    config.tools.obsidian.api_key = String::new();
    config.tools.obsidian.vault_name = "vault".into();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("apiKey"));
}

#[test]
fn test_invalid_obsidian_enabled_missing_vault_name() {
    let mut config = Config::default();
    config.tools.obsidian.enabled = true;
    config.tools.obsidian.api_url = "http://localhost:27123".into();
    config.tools.obsidian.api_key = "key".into();
    config.tools.obsidian.vault_name = String::new();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("vaultName"));
}

// -----------------------------------------------------------------------
// Validation: sandbox write paths too many
// -----------------------------------------------------------------------

#[test]
fn test_invalid_sandbox_write_paths_too_many() {
    let mut config = Config::default();
    config.tools.exec.sandbox.additional_write_paths =
        (0..101).map(|i| format!("/path/{i}")).collect();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("additionalWritePaths"));
}

// -----------------------------------------------------------------------
// GatewayConfig defaults
// -----------------------------------------------------------------------

#[test]
fn test_gateway_config_defaults() {
    let gw = GatewayConfig::default();
    assert!(gw.enabled);
    assert_eq!(gw.host, "127.0.0.1");
    assert_eq!(gw.port, 18790);
    assert!(gw.webhooks.is_empty());
}

// -----------------------------------------------------------------------
// WebhookConfig deserialization
// -----------------------------------------------------------------------

#[test]
fn test_webhook_config_deserialization_defaults() {
    let json = r#"{ "secret": "my-secret" }"#;
    let wh: WebhookConfig = serde_json::from_str(json).unwrap();
    assert!(wh.enabled);
    assert_eq!(wh.secret, "my-secret");
    assert_eq!(wh.template, "{{body}}");
    assert!(!wh.agent_turn);
    assert!(wh.targets.is_empty());
}

// -----------------------------------------------------------------------
// redact_debug macro — TelegramConfig
// -----------------------------------------------------------------------

#[test]
fn test_redact_debug_telegram_config() {
    let mut tg = TelegramConfig::default();
    let debug_empty = format!("{:?}", tg);
    assert!(
        debug_empty.contains("[empty]"),
        "empty token should show [empty] in debug: {debug_empty}"
    );

    tg.token = "1234567890:ABCdefGHIjklMNO".to_string();
    let debug_redacted = format!("{:?}", tg);
    assert!(
        debug_redacted.contains("[REDACTED]"),
        "non-empty token should show [REDACTED] in debug: {debug_redacted}"
    );
    assert!(
        !debug_redacted.contains("1234567890"),
        "token value must not appear in debug output"
    );
}

// -----------------------------------------------------------------------
// DmPolicy Display impl
// -----------------------------------------------------------------------

#[test]
fn test_dm_policy_display() {
    assert_eq!(format!("{}", DmPolicy::Allowlist), "allowlist");
    assert_eq!(format!("{}", DmPolicy::Pairing), "pairing");
    assert_eq!(format!("{}", DmPolicy::Open), "open");
}

// -----------------------------------------------------------------------
// PromptGuardAction Display + should_block
// -----------------------------------------------------------------------

#[test]
fn test_prompt_guard_action_display_and_should_block() {
    assert_eq!(format!("{}", PromptGuardAction::Warn), "warn");
    assert_eq!(format!("{}", PromptGuardAction::Block), "block");

    let warn_guard = PromptGuardConfig {
        enabled: true,
        action: PromptGuardAction::Warn,
    };
    assert!(!warn_guard.should_block());

    let block_guard = PromptGuardConfig {
        enabled: true,
        action: PromptGuardAction::Block,
    };
    assert!(block_guard.should_block());
}

// -----------------------------------------------------------------------
// FusionStrategy default + serde
// -----------------------------------------------------------------------

#[test]
fn test_fusion_strategy_default_and_serde() {
    assert_eq!(FusionStrategy::default(), FusionStrategy::WeightedScore);

    let ws: FusionStrategy = serde_json::from_str(r#""weighted_score""#).unwrap();
    assert_eq!(ws, FusionStrategy::WeightedScore);

    let rrf: FusionStrategy = serde_json::from_str(r#""rrf""#).unwrap();
    assert_eq!(rrf, FusionStrategy::Rrf);
}
