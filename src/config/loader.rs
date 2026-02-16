use crate::config::Config;
use crate::utils::{ensure_dir, get_oxicrab_home};
use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub fn get_config_path() -> Result<PathBuf> {
    Ok(get_oxicrab_home()?.join("config.json"))
}

pub fn load_config(config_path: Option<&Path>) -> Result<Config> {
    let default_path = get_config_path().unwrap_or_else(|_| PathBuf::from("config.json"));
    let path = config_path.unwrap_or(default_path.as_path());

    if path.exists() {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;
        let mut data: Value = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse config JSON from {}", path.display()))?;

        // Migrate old config formats
        data = migrate_config(data);

        // Note: We don't convert keys here because serde's `rename` attributes
        // expect the original camelCase keys from JSON. The conversion was causing
        // fields with `rename` attributes to not be deserialized correctly.

        let config: Config =
            serde_json::from_value(data).with_context(|| "Failed to deserialize config")?;

        // Validate configuration
        config
            .validate()
            .with_context(|| "Configuration validation failed")?;

        return Ok(config);
    }

    let default_config = Config::default();
    // Validate default config too (should always pass, but good practice)
    default_config
        .validate()
        .with_context(|| "Default configuration validation failed")?;
    Ok(default_config)
}

fn migrate_config(data: Value) -> Value {
    // Move tools.exec.restrictToWorkspace → tools.restrictToWorkspace
    if let Value::Object(mut map) = data {
        if let Some(Value::Object(ref mut tools_map)) = map.get_mut("tools") {
            if let Some(Value::Object(ref mut exec_map)) = tools_map.get_mut("exec") {
                if let Some(restrict) = exec_map.remove("restrictToWorkspace") {
                    if !tools_map.contains_key("restrictToWorkspace") {
                        tools_map.insert("restrictToWorkspace".to_string(), restrict);
                    }
                }
            }
        }
        Value::Object(map)
    } else {
        data
    }
}

pub fn save_config(config: &Config, config_path: Option<&Path>) -> Result<()> {
    let default_path = get_config_path().unwrap_or_else(|_| PathBuf::from("config.json"));
    let path = config_path.unwrap_or(default_path.as_path());

    ensure_dir(path.parent().context("Config path has no parent")?)?;

    // Convert to camelCase
    let mut data = serde_json::to_value(config)?;
    data = convert_to_camel(data);

    let content = serde_json::to_string_pretty(&data)?;
    fs::write(path, content)
        .with_context(|| format!("Failed to write config to {}", path.display()))?;

    // Restrict permissions (best-effort, may fail on Windows)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

fn convert_to_camel(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                let new_key = snake_to_camel(&k);
                new_map.insert(new_key, convert_to_camel(v));
            }
            Value::Object(new_map)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(convert_to_camel).collect()),
        _ => value,
    }
}

fn snake_to_camel(name: &str) -> String {
    let parts: Vec<&str> = name.split('_').collect();
    if parts.is_empty() {
        return String::new();
    }
    parts[0].to_string()
        + &parts[1..]
            .iter()
            .map(|s| {
                let mut chars = s.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snake_to_camel_simple() {
        assert_eq!(snake_to_camel("hello_world"), "helloWorld");
    }

    #[test]
    fn test_snake_to_camel_single_word() {
        assert_eq!(snake_to_camel("hello"), "hello");
    }

    #[test]
    fn test_snake_to_camel_multiple_underscores() {
        assert_eq!(snake_to_camel("max_tool_iterations"), "maxToolIterations");
    }

    #[test]
    fn test_snake_to_camel_empty() {
        assert_eq!(snake_to_camel(""), "");
    }

    #[test]
    fn test_snake_to_camel_already_camel() {
        assert_eq!(snake_to_camel("alreadyCamel"), "alreadyCamel");
    }

    #[test]
    fn test_convert_to_camel_nested() {
        let input = serde_json::json!({
            "max_tokens": 8192,
            "api_key": "test"
        });
        let result = convert_to_camel(input);
        assert!(result.get("maxTokens").is_some());
        assert!(result.get("apiKey").is_some());
        assert!(result.get("max_tokens").is_none());
    }

    #[test]
    fn test_convert_to_camel_array() {
        let input = serde_json::json!([{"some_key": 1}, {"another_key": 2}]);
        let result = convert_to_camel(input);
        let arr = result.as_array().unwrap();
        assert!(arr[0].get("someKey").is_some());
        assert!(arr[1].get("anotherKey").is_some());
    }

    #[test]
    fn test_convert_to_camel_scalar_passthrough() {
        assert_eq!(
            convert_to_camel(serde_json::json!(42)),
            serde_json::json!(42)
        );
        assert_eq!(
            convert_to_camel(serde_json::json!("hello")),
            serde_json::json!("hello")
        );
        assert_eq!(
            convert_to_camel(serde_json::json!(true)),
            serde_json::json!(true)
        );
        assert_eq!(
            convert_to_camel(serde_json::json!(null)),
            serde_json::json!(null)
        );
    }

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
}
