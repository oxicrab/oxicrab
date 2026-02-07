use crate::config::Config;
use crate::utils::{ensure_dir, get_nanobot_home};
use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub fn get_config_path() -> Result<PathBuf> {
    Ok(get_nanobot_home()?.join("config.json"))
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
        
        let config: Config = serde_json::from_value(data)
            .with_context(|| "Failed to deserialize config")?;
        return Ok(config);
    }

    Ok(Config::default())
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
        Value::Array(arr) => {
            Value::Array(arr.into_iter().map(convert_to_camel).collect())
        }
        _ => value,
    }
}

fn snake_to_camel(name: &str) -> String {
    let parts: Vec<&str> = name.split('_').collect();
    if parts.is_empty() {
        return String::new();
    }
    parts[0].to_string() + &parts[1..]
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
