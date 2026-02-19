pub mod http;
pub mod media;
pub mod regex;
pub mod subprocess;
pub mod task_tracker;
pub mod transcription;
pub mod url_security;

use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn ensure_dir(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    std::fs::create_dir_all(path)
        .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    Ok(path.to_path_buf())
}

pub fn safe_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

pub fn get_oxicrab_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("OXICRAB_HOME") {
        return Ok(PathBuf::from(home));
    }
    Ok(dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".oxicrab"))
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
    tmp.write_all(content.as_bytes())
        .with_context(|| "Failed to write to temp file")?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .with_context(|| format!("Failed to atomically rename to {}", path.display()))?;
    Ok(())
}

pub fn get_workspace_path(workspace: &str) -> PathBuf {
    if workspace.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            // Strip "~/" prefix to get relative path from home
            let stripped = workspace.strip_prefix("~/").unwrap_or(workspace);
            return home.join(stripped);
        }
    } else if workspace == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    } else if let Some(rest) = workspace.strip_prefix('~') {
        // Handle "~something" (without slash) - treat as "~/something"
        if let Some(home) = dirs::home_dir() {
            let relative = rest.strip_prefix('/').unwrap_or(rest);
            return home.join(relative);
        }
    }
    PathBuf::from(workspace)
}

#[cfg(test)]
mod tests;
