use crate::config::Config;
use crate::utils::{ensure_dir, get_oxicrab_home};
use anyhow::{Context, Result};
use fs2::FileExt;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[allow(unused_imports)]
use tracing::warn;

pub fn get_config_path() -> Result<PathBuf> {
    Ok(get_oxicrab_home()?.join("config.json"))
}

pub fn load_config(config_path: Option<&Path>) -> Result<Config> {
    let default_path = get_config_path().unwrap_or_else(|_| PathBuf::from("config.json"));
    let path = config_path.unwrap_or(default_path.as_path());

    if path.exists() {
        // Acquire shared (read) lock — allows concurrent readers, blocks during writes
        let file = fs::File::open(path)
            .with_context(|| format!("Failed to open config at {}", path.display()))?;
        file.lock_shared()
            .with_context(|| "Failed to acquire shared lock on config file")?;

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;
        // Lock released when `file` drops at end of scope

        let mut data: Value = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse config JSON from {}", path.display()))?;

        // Migrate old config formats
        data = migrate_config(data);

        // Note: We don't convert keys here because serde's `rename` attributes
        // expect the original camelCase keys from JSON. The conversion was causing
        // fields with `rename` attributes to not be deserialized correctly.

        let mut config: Config =
            serde_json::from_value(data).with_context(|| "Failed to deserialize config")?;

        // Apply credential overrides (env > helper > keyring > config.json)
        crate::config::credentials::apply_env_overrides(&mut config);
        crate::config::credentials::apply_credential_helper(&mut config);
        #[cfg(feature = "keyring-store")]
        crate::config::credentials::apply_keyring_overrides(&mut config);

        // Check file permissions (unix only, warn-only)
        check_file_permissions(path);

        // Validate configuration
        config
            .validate()
            .with_context(|| "Configuration validation failed")?;

        return Ok(config);
    }

    let mut default_config = Config::default();
    // Apply credential overrides even with default config
    crate::config::credentials::apply_env_overrides(&mut default_config);
    crate::config::credentials::apply_credential_helper(&mut default_config);
    #[cfg(feature = "keyring-store")]
    crate::config::credentials::apply_keyring_overrides(&mut default_config);
    // Validate default config too (should always pass, but good practice)
    default_config
        .validate()
        .with_context(|| "Default configuration validation failed")?;
    Ok(default_config)
}

/// Warn if the config file or its parent directory has overly permissive permissions.
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
                    "config file {} has permissions {:o} — recommend 0600",
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
                    "config directory {} has permissions {:o} — recommend 0700",
                    parent.display(),
                    mode & 0o777
                );
            }
        }
    });
}

#[cfg(not(unix))]
fn check_file_permissions(_path: &Path) {
    // Permission checks only apply on unix systems
}

fn migrate_config(data: Value) -> Value {
    // Move tools.exec.restrictToWorkspace → tools.restrictToWorkspace
    if let Value::Object(mut map) = data {
        if let Some(Value::Object(tools_map)) = map.get_mut("tools")
            && let Some(Value::Object(exec_map)) = tools_map.get_mut("exec")
            && let Some(restrict) = exec_map.remove("restrictToWorkspace")
            && !tools_map.contains_key("restrictToWorkspace")
        {
            tools_map.insert("restrictToWorkspace".to_string(), restrict);
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

    // Acquire exclusive lock via separate lockfile.
    // A separate file is needed because atomic_write() uses rename(), which
    // invalidates flock on the original inode. The .lock file survives renames.
    let lock_path = path.with_extension("json.lock");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&lock_path)
        .with_context(|| format!("Failed to create lock file at {}", lock_path.display()))?;
    lock_file
        .lock_exclusive()
        .with_context(|| "Failed to acquire exclusive lock on config lock file")?;

    // serde `rename` attributes already produce camelCase keys during
    // serialization, so no post-processing is needed. A prior convert_to_camel
    // pass was removed because it corrupted HashMap keys (MCP server names,
    // env vars, custom headers, model cost prefixes) that contain underscores.
    let data = serde_json::to_value(config)?;

    let content = serde_json::to_string_pretty(&data)?;
    crate::utils::atomic_write(path, &content)
        .with_context(|| format!("Failed to write config to {}", path.display()))?;

    // Restrict permissions (best-effort, may fail on Windows)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }

    // Lock released when lock_file drops
    Ok(())
}

#[cfg(test)]
mod tests;
