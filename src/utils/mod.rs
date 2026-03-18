pub mod credential_store;
pub mod io_safe;
pub mod path_sanitize;
pub mod regex;
pub mod sandbox;
pub mod shell_ast;
pub mod subprocess;
pub mod task_tracker;
pub mod time;
pub mod transcription;

// Shared utilities re-exported from oxicrab-core
pub use oxicrab_core::utils::{get_oxicrab_home, http, media, truncate_chars, url_security};

use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn ensure_dir(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    std::fs::create_dir_all(path)
        .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    Ok(path.to_path_buf())
}

/// Resolve the path to the shared `MemoryDB` file using the default workspace
/// location (`{OXICRAB_HOME}/workspace/memory/memory.sqlite3`).
///
/// For config-aware resolution (custom workspace paths), use
/// `config.workspace_path().join("memory").join("memory.sqlite3")` instead.
pub fn get_memory_db_path() -> Result<PathBuf> {
    Ok(get_oxicrab_home()?
        .join("workspace")
        .join("memory")
        .join("memory.sqlite3"))
}

/// Write content atomically via tempfile + rename.
///
/// Guarantees the file is either fully written or untouched.
/// On crash during write, the original file remains intact.
pub fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().context("Path has no parent directory")?;
    std::fs::create_dir_all(parent)?;

    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("Failed to create temp file in {}", parent.display()))?;
    // Restrict temp file permissions BEFORE writing content, so secrets are
    // never readable by other users even briefly (closes TOCTOU window).
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

#[cfg(test)]
pub(crate) use oxicrab_core::utils::{get_workspace_path, safe_filename};

#[cfg(test)]
mod tests;
