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
fn test_load_config_missing_explicit_file_errors() {
    let path = std::path::Path::new("/tmp/nonexistent_oxicrab_config_test.toml");
    let err = load_config(Some(path)).unwrap_err();
    assert!(err.to_string().contains("Config file not found"));
}

#[test]
fn test_load_config_minimal_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "").unwrap();
    let config = load_config(Some(&path)).unwrap();
    assert_eq!(config.agents.defaults.max_tokens, 8192);
}

#[test]
fn test_save_and_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let config = Config::default();
    save_config(&config, Some(&path)).unwrap();
    let loaded = load_config(Some(&path)).unwrap();
    assert_eq!(
        loaded.agents.defaults.model_routing.default,
        config.agents.defaults.model_routing.default
    );
    assert_eq!(
        loaded.agents.defaults.max_tokens,
        config.agents.defaults.max_tokens
    );
    assert_eq!(
        loaded.agents.defaults.temperature,
        config.agents.defaults.temperature
    );
}

#[test]
fn test_example_config_loads_and_validates() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config.example.toml");
    let config = load_config(Some(&path)).expect("config.example.toml should load");
    config
        .validate()
        .expect("config.example.toml should pass validation");
}

#[test]
fn test_layered_config_overrides_merge_before_deserialize() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("config.toml");
    let local = dir.path().join("config.local.toml");
    let overlay_dir = dir.path().join("config.d");
    std::fs::create_dir(&overlay_dir).unwrap();

    std::fs::write(
        &base,
        r#"
[providers.anthropic]
apiKey = "base-key"

[agents.defaults]
maxTokens = 4096
temperature = 0.5
"#,
    )
    .unwrap();

    std::fs::write(
        &local,
        r#"
[agents.defaults.modelRouting]
default = "openai/gpt-5"
"#,
    )
    .unwrap();

    std::fs::write(
        overlay_dir.join("10-router.toml"),
        r"
[router]
semanticThreshold = 0.61
",
    )
    .unwrap();

    std::fs::write(
        overlay_dir.join("20-agent.toml"),
        r"
[agents.defaults]
maxTokens = 2048
",
    )
    .unwrap();

    let config = load_config(Some(&base)).unwrap();
    assert_eq!(config.providers.anthropic.api_key, "base-key");
    assert_eq!(config.agents.defaults.model_routing.default, "openai/gpt-5");
    assert_eq!(config.agents.defaults.max_tokens, 2048);
    assert!((config.router.semantic_threshold - 0.61).abs() < f32::EPSILON);
}

#[test]
fn test_unknown_config_key_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        r"
[router]
semanticThreshold = 0.61
enabled = true
",
    )
    .unwrap();

    let err = load_config(Some(&path)).unwrap_err();
    assert!(err.to_string().contains("Unknown config key(s)"));
    assert!(err.to_string().contains("'router.enabled'"));
}

#[test]
fn test_env_override_applies() {
    use crate::config::credentials::apply_env_overrides;

    let mut config = Config::default();
    assert!(config.providers.anthropic.api_key.is_empty());

    unsafe { std::env::set_var("OXICRAB_ANTHROPIC_API_KEY", "test-key-from-env") };
    apply_env_overrides(&mut config);
    assert_eq!(config.providers.anthropic.api_key, "test-key-from-env");
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
    let path = dir.path().join("config.toml");
    let config = Config::default();
    save_config(&config, Some(&path)).unwrap();

    assert!(path.exists());
    let loaded = load_config(Some(&path)).unwrap();
    assert_eq!(
        loaded.agents.defaults.model_routing.default,
        config.agents.defaults.model_routing.default
    );

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
    let path = dir.path().join("config.toml");
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
fn test_load_config_with_credential_helper_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
[credentialHelper]
command = "op"
args = ["--account", "my.1password.com"]
format = "1password"
"#,
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

#[test]
fn test_concurrent_save_no_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let path_clone = path.clone();

    let config = Config::default();
    save_config(&config, Some(&path)).unwrap();

    let handles: Vec<_> = (0..10)
        .map(|i| {
            let p = path_clone.clone();
            std::thread::spawn(move || {
                let mut cfg = Config::default();
                cfg.agents.defaults.max_tokens = 1000 + i;
                save_config(&cfg, Some(&p)).unwrap();
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    let content = std::fs::read_to_string(&path).unwrap();
    let _: toml::Value =
        toml::from_str(&content).expect("config file should be valid TOML after concurrent saves");
}
