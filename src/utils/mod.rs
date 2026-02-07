use anyhow::{Context, Result};
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

pub fn get_nanobot_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("NANOBOT_HOME") {
        return Ok(PathBuf::from(home));
    }
    Ok(dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".nanobot"))
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
    } else if workspace.starts_with("~") {
        // Handle "~something" (without slash) - treat as "~/something"
        if let Some(home) = dirs::home_dir() {
            let stripped = workspace.strip_prefix("~").unwrap_or(workspace);
            // If stripped starts with /, it's an absolute path - strip that too
            let relative = if stripped.starts_with('/') {
                &stripped[1..]
            } else {
                stripped
            };
            return home.join(relative);
        }
    }
    PathBuf::from(workspace)
}
