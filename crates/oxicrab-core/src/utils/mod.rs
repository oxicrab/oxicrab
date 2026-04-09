pub mod http;
pub mod media;
pub mod url_params;
pub mod url_security;

use anyhow::{Context, Result};
use std::path::PathBuf;

pub fn get_workspace_path(workspace: &str) -> PathBuf {
    if workspace.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            let stripped = workspace.strip_prefix("~/").unwrap_or(workspace);
            return home.join(stripped);
        }
    } else if workspace == "~"
        && let Some(home) = dirs::home_dir()
    {
        return home;
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

/// Build a button label by prepending `prefix` to `text`, truncating `text` to
/// at most `max_text_chars` characters (with `...` suffix) if it exceeds the budget.
/// Safe for multi-byte UTF-8.
///
/// Example: `truncate_label("View: ", "long title", 10)` → `"View: long title"`
/// if ≤10 chars, or `"View: long ti..."` if over.
pub fn truncate_label(prefix: &str, text: &str, max_text_chars: usize) -> String {
    let boundary = text.floor_char_boundary(max_text_chars);
    if boundary >= text.len() {
        format!("{prefix}{text}")
    } else {
        let trim_boundary = text.floor_char_boundary(max_text_chars.saturating_sub(3));
        format!("{prefix}{}...", &text[..trim_boundary])
    }
}

/// Truncate a string to at most `max_chars` characters, appending `suffix`
/// (e.g. `"..."`) if truncated. Returns the original string (owned) if short enough.
/// Safe for multi-byte UTF-8.
pub fn truncate_chars(s: &str, max_chars: usize, suffix: &str) -> String {
    let boundary = s.floor_char_boundary(max_chars);
    if boundary >= s.len() {
        s.to_string()
    } else {
        format!("{}{suffix}", &s[..boundary])
    }
}
