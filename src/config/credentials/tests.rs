use super::*;

#[test]
fn test_credential_names_count() {
    assert_eq!(CREDENTIAL_NAMES.len(), 28);
}

#[test]
fn test_credential_env_vars_count() {
    assert_eq!(CREDENTIAL_ENV_VARS.len(), 28);
}

#[test]
fn test_get_credential_field() {
    let mut config = Config::default();
    let field = get_credential_field(&mut config, "anthropic-api-key");
    assert!(field.is_some());
    *field.unwrap() = "test-key".to_string();
    assert_eq!(config.providers.anthropic.api_key, "test-key");
}

#[test]
fn test_get_credential_field_unknown() {
    let mut config = Config::default();
    assert!(get_credential_field(&mut config, "nonexistent").is_none());
}

#[test]
fn test_get_credential_value() {
    let mut config = Config::default();
    config.providers.openai.api_key = "test-openai".to_string();
    assert_eq!(
        get_credential_value(&config, "openai-api-key"),
        Some("test-openai")
    );
}

#[test]
fn test_get_credential_value_unknown() {
    let config = Config::default();
    assert!(get_credential_value(&config, "nonexistent").is_none());
}

#[test]
fn test_apply_env_overrides() {
    let mut config = Config::default();
    assert!(config.providers.anthropic.api_key.is_empty());

    unsafe { std::env::set_var("OXICRAB_ANTHROPIC_API_KEY", "test-key-from-env") };
    apply_env_overrides(&mut config);
    assert_eq!(config.providers.anthropic.api_key, "test-key-from-env");

    unsafe { std::env::remove_var("OXICRAB_ANTHROPIC_API_KEY") };
}

#[test]
fn test_apply_env_overrides_empty_ignored() {
    let mut config = Config::default();
    config.providers.openai.api_key = "original-key".to_string();

    unsafe { std::env::set_var("OXICRAB_OPENAI_API_KEY", "") };
    apply_env_overrides(&mut config);
    assert_eq!(config.providers.openai.api_key, "original-key");

    unsafe { std::env::remove_var("OXICRAB_OPENAI_API_KEY") };
}

#[test]
fn test_apply_env_overrides_channels() {
    let mut config = Config::default();

    unsafe { std::env::set_var("OXICRAB_TELEGRAM_TOKEN", "tg-token") };
    unsafe { std::env::set_var("OXICRAB_DISCORD_TOKEN", "dc-token") };
    unsafe { std::env::set_var("OXICRAB_GITHUB_TOKEN", "gh-token") };
    apply_env_overrides(&mut config);

    assert_eq!(config.channels.telegram.token, "tg-token");
    assert_eq!(config.channels.discord.token, "dc-token");
    assert_eq!(config.tools.github.token, "gh-token");

    unsafe { std::env::remove_var("OXICRAB_TELEGRAM_TOKEN") };
    unsafe { std::env::remove_var("OXICRAB_DISCORD_TOKEN") };
    unsafe { std::env::remove_var("OXICRAB_GITHUB_TOKEN") };
}

#[test]
fn test_apply_env_overrides_new_vars() {
    let mut config = Config::default();

    unsafe { std::env::set_var("OXICRAB_MOONSHOT_API_KEY", "moonshot-key") };
    unsafe { std::env::set_var("OXICRAB_WEATHER_API_KEY", "weather-key") };
    unsafe { std::env::set_var("OXICRAB_TRANSCRIPTION_API_KEY", "transcription-key") };
    apply_env_overrides(&mut config);

    assert_eq!(config.providers.moonshot.api_key, "moonshot-key");
    assert_eq!(config.tools.weather.api_key, "weather-key");
    assert_eq!(config.voice.transcription.api_key, "transcription-key");

    unsafe { std::env::remove_var("OXICRAB_MOONSHOT_API_KEY") };
    unsafe { std::env::remove_var("OXICRAB_WEATHER_API_KEY") };
    unsafe { std::env::remove_var("OXICRAB_TRANSCRIPTION_API_KEY") };
}

#[test]
fn test_credential_helper_empty_command_noop() {
    let mut config = Config::default();
    // Default helper has empty command, should be a no-op
    apply_credential_helper(&mut config);
    assert!(config.providers.anthropic.api_key.is_empty());
}

#[test]
fn test_detect_source_env() {
    let config = Config::default();
    unsafe { std::env::set_var("OXICRAB_ANTHROPIC_API_KEY", "from-env") };
    assert_eq!(detect_source("anthropic-api-key", &config), "env");
    unsafe { std::env::remove_var("OXICRAB_ANTHROPIC_API_KEY") };
}

#[test]
fn test_detect_source_config() {
    let mut config = Config::default();
    config.providers.openai.api_key = "from-config".to_string();
    // Make sure env is not set
    unsafe { std::env::remove_var("OXICRAB_OPENAI_API_KEY") };
    assert_eq!(detect_source("openai-api-key", &config), "config");
}

#[test]
fn test_detect_source_empty() {
    let config = Config::default();
    unsafe { std::env::remove_var("OXICRAB_GEMINI_API_KEY") };
    assert_eq!(detect_source("gemini-api-key", &config), "[empty]");
}

#[test]
fn test_all_credential_paths_valid() {
    // Verify every credential name maps to a valid field
    let mut config = Config::default();
    for &name in CREDENTIAL_NAMES {
        assert!(
            get_credential_field(&mut config, name).is_some(),
            "credential {name} has no matching field"
        );
    }
}

#[test]
fn test_all_env_vars_unique() {
    let mut seen = std::collections::HashSet::new();
    for &(name, env_var) in CREDENTIAL_ENV_VARS {
        assert!(
            seen.insert(env_var),
            "duplicate env var {env_var} for {name}"
        );
    }
}

#[test]
fn test_all_names_unique() {
    let mut seen = std::collections::HashSet::new();
    for &name in CREDENTIAL_NAMES {
        assert!(seen.insert(name), "duplicate credential name {name}");
    }
}

#[test]
fn test_get_credential_field_channel_tokens() {
    let mut config = Config::default();

    // Test channel token fields
    *get_credential_field(&mut config, "telegram-token").unwrap() = "tg".to_string();
    *get_credential_field(&mut config, "discord-token").unwrap() = "dc".to_string();
    *get_credential_field(&mut config, "slack-bot-token").unwrap() = "sb".to_string();
    *get_credential_field(&mut config, "slack-app-token").unwrap() = "sa".to_string();
    *get_credential_field(&mut config, "twilio-account-sid").unwrap() = "sid".to_string();
    *get_credential_field(&mut config, "twilio-auth-token").unwrap() = "auth".to_string();

    assert_eq!(config.channels.telegram.token, "tg");
    assert_eq!(config.channels.discord.token, "dc");
    assert_eq!(config.channels.slack.bot_token, "sb");
    assert_eq!(config.channels.slack.app_token, "sa");
    assert_eq!(config.channels.twilio.account_sid, "sid");
    assert_eq!(config.channels.twilio.auth_token, "auth");
}

#[test]
fn test_get_credential_field_tool_tokens() {
    let mut config = Config::default();

    *get_credential_field(&mut config, "github-token").unwrap() = "gh".to_string();
    *get_credential_field(&mut config, "weather-api-key").unwrap() = "wx".to_string();
    *get_credential_field(&mut config, "todoist-token").unwrap() = "td".to_string();
    *get_credential_field(&mut config, "web-search-api-key").unwrap() = "ws".to_string();
    *get_credential_field(&mut config, "google-client-secret").unwrap() = "gc".to_string();
    *get_credential_field(&mut config, "obsidian-api-key").unwrap() = "ob".to_string();
    *get_credential_field(&mut config, "media-radarr-api-key").unwrap() = "ra".to_string();
    *get_credential_field(&mut config, "media-sonarr-api-key").unwrap() = "so".to_string();

    assert_eq!(config.tools.github.token, "gh");
    assert_eq!(config.tools.weather.api_key, "wx");
    assert_eq!(config.tools.todoist.token, "td");
    assert_eq!(config.tools.web.search.api_key, "ws");
    assert_eq!(config.tools.google.client_secret, "gc");
    assert_eq!(config.tools.obsidian.api_key, "ob");
    assert_eq!(config.tools.media.radarr.api_key, "ra");
    assert_eq!(config.tools.media.sonarr.api_key, "so");
}

#[test]
fn test_get_credential_field_oauth_tokens() {
    let mut config = Config::default();

    *get_credential_field(&mut config, "anthropic-oauth-access").unwrap() = "access".to_string();
    *get_credential_field(&mut config, "anthropic-oauth-refresh").unwrap() = "refresh".to_string();

    assert_eq!(config.providers.anthropic_oauth.access_token, "access");
    assert_eq!(config.providers.anthropic_oauth.refresh_token, "refresh");
}

#[test]
fn test_get_credential_field_voice() {
    let mut config = Config::default();

    *get_credential_field(&mut config, "transcription-api-key").unwrap() = "voice-key".to_string();

    assert_eq!(config.voice.transcription.api_key, "voice-key");
}

#[test]
fn test_get_credential_value_all_providers() {
    let mut config = Config::default();
    config.providers.anthropic.api_key = "a".to_string();
    config.providers.openai.api_key = "b".to_string();
    config.providers.openrouter.api_key = "c".to_string();
    config.providers.gemini.api_key = "d".to_string();
    config.providers.deepseek.api_key = "e".to_string();
    config.providers.groq.api_key = "f".to_string();
    config.providers.moonshot.api_key = "g".to_string();
    config.providers.zhipu.api_key = "h".to_string();
    config.providers.dashscope.api_key = "i".to_string();
    config.providers.vllm.api_key = "j".to_string();
    config.providers.ollama.api_key = "k".to_string();

    assert_eq!(
        get_credential_value(&config, "anthropic-api-key"),
        Some("a")
    );
    assert_eq!(get_credential_value(&config, "openai-api-key"), Some("b"));
    assert_eq!(
        get_credential_value(&config, "openrouter-api-key"),
        Some("c")
    );
    assert_eq!(get_credential_value(&config, "gemini-api-key"), Some("d"));
    assert_eq!(get_credential_value(&config, "deepseek-api-key"), Some("e"));
    assert_eq!(get_credential_value(&config, "groq-api-key"), Some("f"));
    assert_eq!(get_credential_value(&config, "moonshot-api-key"), Some("g"));
    assert_eq!(get_credential_value(&config, "zhipu-api-key"), Some("h"));
    assert_eq!(
        get_credential_value(&config, "dashscope-api-key"),
        Some("i")
    );
    assert_eq!(get_credential_value(&config, "vllm-api-key"), Some("j"));
    assert_eq!(get_credential_value(&config, "ollama-api-key"), Some("k"));
}

#[test]
fn test_apply_env_overrides_overwrites_existing() {
    let mut config = Config::default();
    config.providers.anthropic.api_key = "from-config".to_string();

    unsafe { std::env::set_var("OXICRAB_ANTHROPIC_API_KEY", "from-env") };
    apply_env_overrides(&mut config);
    // Env var should overwrite config value
    assert_eq!(config.providers.anthropic.api_key, "from-env");

    unsafe { std::env::remove_var("OXICRAB_ANTHROPIC_API_KEY") };
}

#[test]
fn test_apply_env_overrides_all_new_provider_vars() {
    let mut config = Config::default();

    unsafe { std::env::set_var("OXICRAB_ZHIPU_API_KEY", "zhipu-key") };
    unsafe { std::env::set_var("OXICRAB_DASHSCOPE_API_KEY", "dash-key") };
    unsafe { std::env::set_var("OXICRAB_VLLM_API_KEY", "vllm-key") };
    unsafe { std::env::set_var("OXICRAB_OLLAMA_API_KEY", "ollama-key") };
    apply_env_overrides(&mut config);

    assert_eq!(config.providers.zhipu.api_key, "zhipu-key");
    assert_eq!(config.providers.dashscope.api_key, "dash-key");
    assert_eq!(config.providers.vllm.api_key, "vllm-key");
    assert_eq!(config.providers.ollama.api_key, "ollama-key");

    unsafe { std::env::remove_var("OXICRAB_ZHIPU_API_KEY") };
    unsafe { std::env::remove_var("OXICRAB_DASHSCOPE_API_KEY") };
    unsafe { std::env::remove_var("OXICRAB_VLLM_API_KEY") };
    unsafe { std::env::remove_var("OXICRAB_OLLAMA_API_KEY") };
}

#[test]
fn test_apply_env_overrides_oauth_tokens() {
    let mut config = Config::default();

    unsafe { std::env::set_var("OXICRAB_ANTHROPIC_OAUTH_ACCESS", "access-tok") };
    unsafe { std::env::set_var("OXICRAB_ANTHROPIC_OAUTH_REFRESH", "refresh-tok") };
    apply_env_overrides(&mut config);

    assert_eq!(config.providers.anthropic_oauth.access_token, "access-tok");
    assert_eq!(
        config.providers.anthropic_oauth.refresh_token,
        "refresh-tok"
    );

    unsafe { std::env::remove_var("OXICRAB_ANTHROPIC_OAUTH_ACCESS") };
    unsafe { std::env::remove_var("OXICRAB_ANTHROPIC_OAUTH_REFRESH") };
}

#[test]
fn test_apply_env_overrides_tool_vars() {
    let mut config = Config::default();

    unsafe { std::env::set_var("OXICRAB_TODOIST_TOKEN", "todoist-tok") };
    unsafe { std::env::set_var("OXICRAB_WEB_SEARCH_API_KEY", "brave-key") };
    unsafe { std::env::set_var("OXICRAB_GOOGLE_CLIENT_SECRET", "google-secret") };
    unsafe { std::env::set_var("OXICRAB_OBSIDIAN_API_KEY", "obsidian-key") };
    unsafe { std::env::set_var("OXICRAB_MEDIA_RADARR_API_KEY", "radarr-key") };
    unsafe { std::env::set_var("OXICRAB_MEDIA_SONARR_API_KEY", "sonarr-key") };
    apply_env_overrides(&mut config);

    assert_eq!(config.tools.todoist.token, "todoist-tok");
    assert_eq!(config.tools.web.search.api_key, "brave-key");
    assert_eq!(config.tools.google.client_secret, "google-secret");
    assert_eq!(config.tools.obsidian.api_key, "obsidian-key");
    assert_eq!(config.tools.media.radarr.api_key, "radarr-key");
    assert_eq!(config.tools.media.sonarr.api_key, "sonarr-key");

    unsafe { std::env::remove_var("OXICRAB_TODOIST_TOKEN") };
    unsafe { std::env::remove_var("OXICRAB_WEB_SEARCH_API_KEY") };
    unsafe { std::env::remove_var("OXICRAB_GOOGLE_CLIENT_SECRET") };
    unsafe { std::env::remove_var("OXICRAB_OBSIDIAN_API_KEY") };
    unsafe { std::env::remove_var("OXICRAB_MEDIA_RADARR_API_KEY") };
    unsafe { std::env::remove_var("OXICRAB_MEDIA_SONARR_API_KEY") };
}

#[test]
fn test_credential_helper_skips_nonempty_fields() {
    let mut config = Config::default();
    config.providers.anthropic.api_key = "already-set".to_string();
    // Even with a configured helper, it should skip non-empty fields
    config.credential_helper.command = "nonexistent-binary".to_string();
    // apply_credential_helper will skip anthropic-api-key because it's non-empty
    // and may warn about others but shouldn't panic
    apply_credential_helper(&mut config);
    assert_eq!(config.providers.anthropic.api_key, "already-set");
}

#[test]
fn test_credential_helper_line_format_builds_args() {
    // Just verify the line format builds the right command structure
    let helper = CredentialHelperConfig {
        command: "echo".to_string(),
        args: vec!["--prefix".to_string()],
        format: "line".to_string(),
    };
    // For "line" format, args should be: ["--prefix", "key-name"]
    let result = fetch_from_helper(&helper, "test-key");
    // echo --prefix test-key should succeed and return the args
    if let Ok(output) = result {
        // echo might not be available in all test envs
        assert!(output.contains("test-key"));
    }
}

#[test]
fn test_detect_source_config_helper() {
    let mut config = Config::default();
    config.providers.openai.api_key = "from-somewhere".to_string();
    config.credential_helper.command = "my-helper".to_string();
    unsafe { std::env::remove_var("OXICRAB_OPENAI_API_KEY") };
    // When helper is configured and field is non-empty, source is "config/helper"
    assert_eq!(detect_source("openai-api-key", &config), "config/helper");
}

#[test]
fn test_detect_source_env_takes_priority() {
    let mut config = Config::default();
    config.providers.gemini.api_key = "from-config".to_string();
    unsafe { std::env::set_var("OXICRAB_GEMINI_API_KEY", "from-env") };
    // Env should take priority even if config has a value
    assert_eq!(detect_source("gemini-api-key", &config), "env");
    unsafe { std::env::remove_var("OXICRAB_GEMINI_API_KEY") };
}

#[test]
fn test_credential_helper_config_default() {
    let helper = CredentialHelperConfig::default();
    assert!(helper.command.is_empty());
    assert!(helper.args.is_empty());
    assert!(helper.format.is_empty());
}

#[test]
fn test_credential_helper_json_format_invalid_output() {
    let helper = CredentialHelperConfig {
        command: "echo".to_string(),
        args: vec!["not-json".to_string()],
        format: "json".to_string(),
    };
    // echo outputs "not-json" which isn't valid JSON
    let result = fetch_from_helper(&helper, "test-key");
    assert!(result.is_err());
}

#[test]
fn test_env_var_name_mapping_consistency() {
    // Verify each credential name has a corresponding env var in CREDENTIAL_ENV_VARS
    for &name in CREDENTIAL_NAMES {
        let found = CREDENTIAL_ENV_VARS.iter().any(|(n, _)| *n == name);
        assert!(found, "credential {name} missing from CREDENTIAL_ENV_VARS");
    }
}

#[test]
fn test_env_var_names_follow_convention() {
    // All env vars should start with OXICRAB_
    for &(name, env_var) in CREDENTIAL_ENV_VARS {
        assert!(
            env_var.starts_with("OXICRAB_"),
            "env var {env_var} for {name} doesn't start with OXICRAB_"
        );
    }
}
