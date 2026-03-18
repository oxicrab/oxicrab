pub(crate) mod http;
pub(crate) mod media;

use std::path::PathBuf;

use anyhow::{Context, Result};

pub(crate) fn get_oxicrab_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("OXICRAB_HOME") {
        return Ok(PathBuf::from(home));
    }
    Ok(dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".oxicrab"))
}

pub(crate) fn safe_filename(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '\0')
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

pub(crate) fn truncate_chars(s: &str, max_chars: usize, suffix: &str) -> String {
    if s.len() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}{suffix}")
}

pub(crate) fn sanitize_path(
    path: &std::path::Path,
    _workspace: Option<&std::path::Path>,
) -> String {
    // Simplified version -- just use tilde collapse
    let path_str = path.to_string_lossy();
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if path_str.starts_with(home_str.as_ref()) {
            return format!("~{}", &path_str[home_str.len()..]);
        }
    }
    path_str.to_string()
}
