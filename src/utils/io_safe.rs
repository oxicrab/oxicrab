use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::Path;

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
    crate::utils::atomic_write(path, &content)?;
    set_owner_only_permissions(path);
    Ok(())
}

/// Set owner-only file permissions (0600) on Unix. No-op on non-Unix.
pub fn set_owner_only_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}
