pub mod http;
pub mod media;
pub mod url_security;

use anyhow::{Context, Result};
use std::path::PathBuf;

pub fn get_workspace_path(workspace: &str) -> PathBuf {
    if workspace.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            let stripped = workspace.strip_prefix("~/").unwrap_or(workspace);
            return home.join(stripped);
        }
    } else if workspace == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    } else if let Some(rest) = workspace.strip_prefix('~')
        && let Some(home) = dirs::home_dir()
    {
        let relative = rest.strip_prefix('/').unwrap_or(rest);
        return home.join(relative);
    }
    PathBuf::from(workspace)
}

/// Return the `~/.oxicrab/` directory (or `$OXICRAB_HOME` if set).
pub fn get_oxicrab_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("OXICRAB_HOME") {
        return Ok(PathBuf::from(home));
    }
    Ok(dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".oxicrab"))
}

/// Sanitize a string for use as a filename component.
///
/// Removes null bytes and replaces path separators and other
/// problematic characters with underscores.
pub fn safe_filename(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '\0')
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

/// Truncate a string to at most `max_chars` characters, appending `suffix`
/// (e.g. `"..."`) if truncated. Returns the original string (owned) if short enough.
/// Safe for multi-byte UTF-8.
pub fn truncate_chars(s: &str, max_chars: usize, suffix: &str) -> String {
    // Fast path: ASCII-only strings where len == char count
    if s.len() <= max_chars {
        return s.to_string();
    }
    // Find the byte index of the max_chars-th character
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => format!("{}{}", &s[..byte_idx], suffix),
        None => s.to_string(), // fewer chars than max_chars
    }
}
