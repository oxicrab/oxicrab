use crate::config::Config;
use crate::utils::{ensure_dir, get_oxicrab_home};
use anyhow::{Context, Result};
use fs2::FileExt;
use serde_json::Value as JsonValue;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

pub fn get_config_path() -> Result<PathBuf> {
    Ok(get_oxicrab_home()?.join("config.toml"))
}

pub fn load_config(config_path: Option<&Path>) -> Result<Config> {
    let default_path = get_config_path().unwrap_or_else(|_| PathBuf::from("config.toml"));
    let base_path = config_path.unwrap_or(default_path.as_path());
    let layer_paths = config_layer_paths(base_path)?;

    let mut merged = toml::Value::Table(toml::map::Map::new());
    let mut loaded_any = false;

    for path in &layer_paths {
        if !path.exists() {
            continue;
        }

        let content = read_locked_file(path)?;
        let data: toml::Value = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config TOML from {}", path.display()))?;
        merge_toml(&mut merged, data);
        loaded_any = true;
    }

    if !loaded_any {
        let mut default_config = Config::default();
        apply_runtime_overrides(&mut default_config);
        return Ok(default_config);
    }

    let migrated = migrate_config(toml_to_json(&merged)?);
    let mut config: Config =
        serde_json::from_value(migrated).with_context(|| "Failed to deserialize config")?;

    apply_runtime_overrides(&mut config);

    for path in &layer_paths {
        if path.exists() {
            check_file_permissions(path);
        }
    }

    config
        .validate()
        .with_context(|| "Configuration validation failed")?;

    Ok(config)
}

pub fn save_config(config: &Config, config_path: Option<&Path>) -> Result<()> {
    let default_path = get_config_path().unwrap_or_else(|_| PathBuf::from("config.toml"));
    let path = config_path.unwrap_or(default_path.as_path());

    ensure_dir(path.parent().context("Config path has no parent")?)?;

    // Acquire exclusive lock via separate lockfile.
    // A separate file is needed because atomic_write() uses rename(), which
    // invalidates flock on the original inode. The .lock file survives renames.
    let lock_path = path.with_extension("toml.lock");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("Failed to create lock file at {}", lock_path.display()))?;
    lock_file
        .lock_exclusive()
        .with_context(|| "Failed to acquire exclusive lock on config lock file")?;

    let content = toml::to_string_pretty(config)?;
    crate::utils::atomic_write(path, &content)
        .with_context(|| format!("Failed to write config to {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

fn config_layer_paths(base_path: &Path) -> Result<Vec<PathBuf>> {
    let mut layers = vec![base_path.to_path_buf()];

    let parent = base_path
        .parent()
        .with_context(|| format!("Config path {} has no parent", base_path.display()))?;
    let stem = base_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .with_context(|| format!("Config path {} has no file stem", base_path.display()))?;
    let extension = base_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("toml");

    layers.push(parent.join(format!("{stem}.local.{extension}")));

    let overlay_dir = parent.join(format!("{stem}.d"));
    if overlay_dir.is_dir() {
        let mut entries = fs::read_dir(&overlay_dir)
            .with_context(|| {
                format!(
                    "Failed to read config overlay dir {}",
                    overlay_dir.display()
                )
            })?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some(extension))
            .collect::<Vec<_>>();
        entries.sort();
        layers.extend(entries);
    }

    Ok(layers)
}

fn read_locked_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("Failed to open config at {}", path.display()))?;
    file.lock_shared()
        .with_context(|| "Failed to acquire shared lock on config file")?;

    let mut content = String::new();
    std::io::Read::read_to_string(&mut file, &mut content)
        .with_context(|| format!("Failed to read config from {}", path.display()))?;
    Ok(content)
}

fn apply_runtime_overrides(config: &mut Config) {
    // Resolution order: env vars > credential helper > keyring > TOML config
    crate::config::credentials::apply_env_overrides(config);
    crate::config::credentials::apply_credential_helper(config);
    #[cfg(feature = "keyring-store")]
    crate::config::credentials::apply_keyring_overrides(config);
}

fn merge_toml(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, value) in overlay_table {
                match base_table.get_mut(&key) {
                    Some(existing) => merge_toml(existing, value),
                    None => {
                        base_table.insert(key, value);
                    }
                }
            }
        }
        (base_slot, overlay_value) => *base_slot = overlay_value,
    }
}

fn toml_to_json(value: &toml::Value) -> Result<JsonValue> {
    serde_json::to_value(value).with_context(|| "Failed to convert TOML config to JSON value")
}

/// Migrate legacy config keys before deserializing into the canonical schema.
fn migrate_config(data: JsonValue) -> JsonValue {
    // Move tools.exec.restrictToWorkspace -> tools.restrictToWorkspace
    if let JsonValue::Object(mut map) = data {
        if let Some(JsonValue::Object(tools_map)) = map.get_mut("tools")
            && let Some(JsonValue::Object(exec_map)) = tools_map.get_mut("exec")
            && let Some(restrict) = exec_map.remove("restrictToWorkspace")
            && !tools_map.contains_key("restrictToWorkspace")
        {
            tools_map.insert("restrictToWorkspace".to_string(), restrict);
        }
        JsonValue::Object(map)
    } else {
        data
    }
}

/// Warn if a config file or its parent directory has overly permissive permissions.
/// Only emits warnings once per process to avoid spam when config is loaded multiple times.
#[cfg(unix)]
fn check_file_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Once;

    static WARNED: Once = Once::new();
    WARNED.call_once(|| {
        if let Ok(meta) = std::fs::metadata(path) {
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                warn!(
                    "config file {} has permissions {:o} - recommend 0600",
                    path.display(),
                    mode & 0o777
                );
            }
        }

        if let Some(parent) = path.parent()
            && let Ok(meta) = std::fs::metadata(parent)
        {
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                warn!(
                    "config directory {} has permissions {:o} - recommend 0700",
                    parent.display(),
                    mode & 0o777
                );
            }
        }
    });
}

#[cfg(not(unix))]
fn check_file_permissions(_path: &Path) {}

#[cfg(test)]
mod tests;
