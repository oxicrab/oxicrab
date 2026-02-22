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
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_sanitize_path_workspace_relative() {
        let home = dirs::home_dir().unwrap();
        let workspace = home.join("projects/myapp");
        let path = home.join("projects/myapp/src/main.rs");
        let result = sanitize_path(&path, Some(&workspace));
        assert_eq!(result, "~/projects/myapp/src/main.rs");
    }

    #[test]
    fn test_sanitize_path_oxicrab_dir() {
        let home = dirs::home_dir().unwrap();
        let path = home.join(".oxicrab/config.json");
        let result = sanitize_path(&path, None);
        assert_eq!(result, "~/.oxicrab/config.json");
    }

    #[test]
    fn test_sanitize_path_outside_workspace_under_home() {
        let home = dirs::home_dir().unwrap();
        let workspace = home.join("projects/myapp");
        let path = home.join("secrets/key.pem");
        let result = sanitize_path(&path, Some(&workspace));
        assert_eq!(result, "<redacted>/key.pem");
    }

    #[test]
    fn test_sanitize_path_system_path_unchanged() {
        let path = PathBuf::from("/etc/passwd");
        let result = sanitize_path(&path, None);
        assert_eq!(result, "/etc/passwd");
    }

    #[test]
    fn test_sanitize_path_system_usr_unchanged() {
        let path = PathBuf::from("/usr/lib/libfoo.so");
        let result = sanitize_path(&path, None);
        assert_eq!(result, "/usr/lib/libfoo.so");
    }

    #[test]
    fn test_sanitize_error_message_with_embedded_paths() {
        let home = dirs::home_dir().unwrap();
        let workspace = home.join("projects/myapp");
        let home_str = home.to_string_lossy();
        let msg = format!(
            "Error: file not found: {}/secrets/key.pem and /etc/hosts",
            home_str
        );
        let result = sanitize_error_message(&msg, Some(&workspace));
        assert!(result.contains("<redacted>/key.pem"));
        assert!(result.contains("/etc/hosts"));
        assert!(!result.contains(&*home_str));
    }

    #[test]
    fn test_sanitize_path_no_home_dir_unchanged() {
        // When path is not under home at all
        let workspace = PathBuf::from("/opt/app");
        let path = PathBuf::from("/opt/app/data/file.txt");
        let result = sanitize_path(&path, Some(&workspace));
        // Not under home → unchanged
        assert_eq!(result, "/opt/app/data/file.txt");
    }

    #[test]
    fn test_sanitize_error_message_no_paths() {
        let result = sanitize_error_message("simple error with no paths", None);
        assert_eq!(result, "simple error with no paths");
    }
}
