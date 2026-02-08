pub mod regex;
pub mod task_tracker;

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

pub fn get_nanobot_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("NANOBOT_HOME") {
        return Ok(PathBuf::from(home));
    }
    Ok(dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".nanobot"))
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
    tmp.flush()?;
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
mod tests {
    use super::*;

    #[test]
    fn safe_filename_normal() {
        assert_eq!(safe_filename("hello_world"), "hello_world");
    }

    #[test]
    fn safe_filename_replaces_dangerous_chars() {
        assert_eq!(safe_filename("a/b\\c:d*e"), "a_b_c_d_e");
        assert_eq!(safe_filename("file<>|name"), "file___name");
    }

    #[test]
    fn safe_filename_empty() {
        assert_eq!(safe_filename(""), "");
    }

    #[test]
    fn workspace_path_tilde_slash() {
        let result = get_workspace_path("~/foo/bar");
        let home = dirs::home_dir().unwrap();
        assert_eq!(result, home.join("foo/bar"));
    }

    #[test]
    fn workspace_path_tilde_only() {
        let result = get_workspace_path("~");
        let home = dirs::home_dir().unwrap();
        assert_eq!(result, home);
    }

    #[test]
    fn workspace_path_absolute() {
        let result = get_workspace_path("/tmp/workspace");
        assert_eq!(result, PathBuf::from("/tmp/workspace"));
    }

    #[test]
    fn workspace_path_relative() {
        let result = get_workspace_path("relative/path");
        assert_eq!(result, PathBuf::from("relative/path"));
    }

    #[test]
    fn ensure_dir_creates_and_returns() {
        let tmp = tempfile::tempdir().unwrap();
        let new_dir = tmp.path().join("subdir");
        let result = ensure_dir(&new_dir).unwrap();
        assert_eq!(result, new_dir);
        assert!(new_dir.exists());
    }

    #[test]
    fn atomic_write_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.txt");
        atomic_write(&path, "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn atomic_write_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.txt");
        atomic_write(&path, "first").unwrap();
        atomic_write(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    }
}
