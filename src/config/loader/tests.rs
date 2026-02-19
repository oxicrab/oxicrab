use super::*;

#[test]
fn test_migrate_config_moves_restrict_to_workspace() {
    let input = serde_json::json!({
        "tools": {
            "exec": {
                "timeout": 60,
                "restrictToWorkspace": true
            }
        }
    });
    let result = migrate_config(input);
    let tools = result.get("tools").unwrap();
    assert_eq!(
        tools.get("restrictToWorkspace"),
        Some(&serde_json::json!(true))
    );
    // Should be removed from exec
    let exec = tools.get("exec").unwrap();
    assert!(exec.get("restrictToWorkspace").is_none());
}

#[test]
fn test_migrate_config_no_overwrite_existing() {
    let input = serde_json::json!({
        "tools": {
            "restrictToWorkspace": false,
            "exec": {
                "restrictToWorkspace": true
            }
        }
    });
    let result = migrate_config(input);
    let tools = result.get("tools").unwrap();
    // Should keep the existing top-level value (false), not overwrite
    assert_eq!(
        tools.get("restrictToWorkspace"),
        Some(&serde_json::json!(false))
    );
}

#[test]
fn test_migrate_config_no_tools_key() {
    let input = serde_json::json!({"agents": {}});
    let result = migrate_config(input.clone());
    assert_eq!(result, input);
}

#[test]
fn test_load_config_missing_file_returns_default() {
    let path = std::path::Path::new("/tmp/nonexistent_oxicrab_config_test.json");
    let config = load_config(Some(path)).unwrap();
    assert_eq!(config.agents.defaults.model, "claude-sonnet-4-5-20250929");
}

#[test]
fn test_load_config_minimal_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, "{}").unwrap();
    let config = load_config(Some(&path)).unwrap();
    assert_eq!(config.agents.defaults.max_tokens, 8192);
}

#[test]
fn test_save_and_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let config = Config::default();
    save_config(&config, Some(&path)).unwrap();
    let loaded = load_config(Some(&path)).unwrap();
    assert_eq!(loaded.agents.defaults.model, config.agents.defaults.model);
    assert_eq!(
        loaded.agents.defaults.max_tokens,
        config.agents.defaults.max_tokens
    );
    assert!(
        (loaded.agents.defaults.temperature - config.agents.defaults.temperature).abs()
            < f32::EPSILON
    );
}

#[test]
fn test_load_config_with_local_model() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{"agents": {"defaults": {"localModel": "ollama/qwen3-coder:30b"}}}"#,
    )
    .unwrap();
    let config = load_config(Some(&path)).unwrap();
    assert_eq!(
        config.agents.defaults.local_model.as_deref(),
        Some("ollama/qwen3-coder:30b")
    );
}

#[test]
fn test_example_config_loads_and_validates() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config.example.json");
    let config = load_config(Some(&path)).expect("config.example.json should load");
    config
        .validate()
        .expect("config.example.json should pass validation");
}

#[test]
fn test_env_override_applies() {
    use crate::config::credentials::apply_env_overrides;

    let mut config = Config::default();
    assert!(config.providers.anthropic.api_key.is_empty());

    // Set env var and apply
    unsafe { std::env::set_var("OXICRAB_ANTHROPIC_API_KEY", "test-key-from-env") };
    apply_env_overrides(&mut config);
    assert_eq!(config.providers.anthropic.api_key, "test-key-from-env");

    // Clean up
    unsafe { std::env::remove_var("OXICRAB_ANTHROPIC_API_KEY") };
}

#[test]
fn test_env_override_empty_string_ignored() {
    use crate::config::credentials::apply_env_overrides;

    let mut config = Config::default();
    config.providers.openai.api_key = "original-key".to_string();

    unsafe { std::env::set_var("OXICRAB_OPENAI_API_KEY", "") };
    apply_env_overrides(&mut config);
    assert_eq!(config.providers.openai.api_key, "original-key");

    unsafe { std::env::remove_var("OXICRAB_OPENAI_API_KEY") };
}

#[test]
fn test_env_override_channel_tokens() {
    use crate::config::credentials::apply_env_overrides;

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
fn test_save_config_atomic_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let config = Config::default();
    save_config(&config, Some(&path)).unwrap();

    // Verify file exists and can be loaded
    assert!(path.exists());
    let loaded = load_config(Some(&path)).unwrap();
    assert_eq!(loaded.agents.defaults.model, config.agents.defaults.model);

    // On unix, check permissions are 0600
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn test_credential_helper_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut config = Config::default();
    config.credential_helper.command = "my-helper".to_string();
    config.credential_helper.args = vec!["--vault".to_string(), "test".to_string()];
    config.credential_helper.format = "line".to_string();
    save_config(&config, Some(&path)).unwrap();
    let loaded = load_config(Some(&path)).unwrap();
    assert_eq!(loaded.credential_helper.command, "my-helper");
    assert_eq!(
        loaded.credential_helper.args,
        vec!["--vault".to_string(), "test".to_string()]
    );
    assert_eq!(loaded.credential_helper.format, "line");
}

#[test]
fn test_load_config_with_credential_helper_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{
            "credentialHelper": {
                "command": "op",
                "args": ["--account", "my.1password.com"],
                "format": "1password"
            }
        }"#,
    )
    .unwrap();
    let config = load_config(Some(&path)).unwrap();
    assert_eq!(config.credential_helper.command, "op");
    assert_eq!(config.credential_helper.format, "1password");
}

#[test]
fn test_env_override_new_vars() {
    use crate::config::credentials::apply_env_overrides;

    let mut config = Config::default();

    unsafe { std::env::set_var("OXICRAB_MOONSHOT_API_KEY", "moonshot-key") };
    unsafe { std::env::set_var("OXICRAB_TODOIST_TOKEN", "todoist-tok") };
    unsafe { std::env::set_var("OXICRAB_TRANSCRIPTION_API_KEY", "transcription-key") };
    apply_env_overrides(&mut config);

    assert_eq!(config.providers.moonshot.api_key, "moonshot-key");
    assert_eq!(config.tools.todoist.token, "todoist-tok");
    assert_eq!(config.voice.transcription.api_key, "transcription-key");

    unsafe { std::env::remove_var("OXICRAB_MOONSHOT_API_KEY") };
    unsafe { std::env::remove_var("OXICRAB_TODOIST_TOKEN") };
    unsafe { std::env::remove_var("OXICRAB_TRANSCRIPTION_API_KEY") };
}
