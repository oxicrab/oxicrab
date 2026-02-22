use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

/// System paths that are safe to expose in error messages.
const SYSTEM_PREFIXES: &[&str] = &[
    "/usr", "/etc", "/lib", "/lib64", "/bin", "/sbin", "/dev", "/proc", "/tmp", "/var",
];

/// Sanitize a path for inclusion in error messages sent to the LLM.
///
/// - Paths under `~/workspace` → `~/workspace/...` (tilde-collapsed)
/// - Paths under home but outside workspace → `<redacted>/filename`
/// - System paths (`/usr`, `/etc`, etc.) → unchanged
/// - Other absolute paths (outside home) → unchanged
pub fn sanitize_path(path: &Path, workspace: Option<&Path>) -> String {
    let path_str = path.to_string_lossy();

    let Some(home) = dirs::home_dir() else {
        return path_str.to_string();
    };
    let home_str = home.to_string_lossy();

    // Not under home → return unchanged
    if !path_str.starts_with(home_str.as_ref()) {
        return path_str.to_string();
    }

    // Check system prefixes first (in case home is under /var or similar)
    for prefix in SYSTEM_PREFIXES {
        if path_str.starts_with(prefix) && !path_str.starts_with(home_str.as_ref()) {
            return path_str.to_string();
        }
    }

    // Under home — check if it's under workspace.
    // We already verified path_str starts with home_str above, so using
    // home_str.len() as the offset is safe (workspace must also be under home).
    if let Some(ws) = workspace {
        let ws_str = ws.to_string_lossy();
        if path_str.starts_with(ws_str.as_ref()) && path_str.is_char_boundary(home_str.len()) {
            // Collapse home to ~
            let rest = &path_str[home_str.len()..];
            return format!("~{rest}");
        }
    }

    // Under home + inside ~/.oxicrab → collapse to ~
    let oxicrab_dir = home.join(".oxicrab");
    let oxicrab_str = oxicrab_dir.to_string_lossy();
    if path_str.starts_with(oxicrab_str.as_ref()) && path_str.is_char_boundary(home_str.len()) {
        let rest = &path_str[home_str.len()..];
        return format!("~{rest}");
    }

    // Under home but outside workspace → redact, show only filename
    let filename = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    format!("<redacted>/{filename}")
}

static PATH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:/[\w._-]+){2,}").expect("path regex"));

/// Sanitize an error message by finding and replacing absolute paths
/// that contain the home directory.
pub fn sanitize_error_message(msg: &str, workspace: Option<&Path>) -> String {
    let Some(home) = dirs::home_dir() else {
        return msg.to_string();
    };
    let home_str = home.to_string_lossy();

    PATH_RE
        .replace_all(msg, |caps: &regex::Captures| {
            let matched = &caps[0];
            if matched.starts_with(home_str.as_ref()) {
                sanitize_path(Path::new(matched), workspace)
            } else {
                matched.to_string()
            }
        })
        .to_string()
}

#[cfg(test)]
mod tests;
