//! Internal utility functions used by provider implementations.
//!
//! Kept private to this crate — not exported.

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Return the oxicrab home directory (`$OXICRAB_HOME` or `~/.oxicrab`).
pub fn get_oxicrab_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("OXICRAB_HOME") {
        return Ok(PathBuf::from(home));
    }
    Ok(dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".oxicrab"))
}

/// Read JSON from disk with a shared lock on a sibling `.json.lock` file.
pub fn read_json_locked<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }

    // Best-effort lock acquisition to avoid hard failures when lock files are unavailable.
    let lock_path = path.with_extension("json.lock");
    let _lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .ok()
        .and_then(|f| fs2::FileExt::lock_shared(&f).ok().map(|()| f));

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read JSON file: {}", path.display()))?;
    let value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse JSON file: {}", path.display()))?;
    Ok(Some(value))
}

/// Write JSON to disk atomically with an exclusive lock on a sibling `.json.lock` file.
pub fn write_json_locked<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    use fs2::FileExt;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let lock_path = path.with_extension("json.lock");
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;
    lock_file.lock_exclusive()?;

    let content = serde_json::to_string_pretty(value)?;
    atomic_write(path, &content)?;
    set_owner_only_permissions(path)?;
    Ok(())
}

/// Write content atomically via tempfile + rename.
fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().context("Path has no parent directory")?;
    std::fs::create_dir_all(parent)?;

    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("Failed to create temp file in {}", parent.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tmp
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    tmp.write_all(content.as_bytes())
        .with_context(|| "Failed to write to temp file")?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .with_context(|| format!("Failed to atomically rename to {}", path.display()))?;
    Ok(())
}

/// Set owner-only file permissions (0600) on Unix. No-op on non-Unix.
fn set_owner_only_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).with_context(
            || format!("Failed to set owner-only permissions on {}", path.display()),
        )?;
    }
    Ok(())
}
