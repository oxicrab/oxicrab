use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tracing::warn;

fn check_path_allowed(file_path: &Path, allowed_roots: &Option<Vec<PathBuf>>) -> Result<()> {
    if let Some(roots) = allowed_roots {
        let resolved = file_path
            .canonicalize()
            .map_err(|_| anyhow::anyhow!("Error: Cannot resolve path '{}'", file_path.display()))?;
        for root in roots {
            if let Ok(root_resolved) = root.canonicalize() {
                if resolved == root_resolved || resolved.starts_with(&root_resolved) {
                    return Ok(());
                }
            }
        }
        let roots_str = roots
            .iter()
            .map(|r| r.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!(
            "Error: Path '{}' is outside the allowed directories ({})",
            file_path.display(),
            roots_str
        );
    }
    Ok(())
}

const MAX_BACKUPS: usize = 14;

/// Create a timestamped backup of a file before overwriting it.
/// Backups are stored in `backup_dir/{filename}.{timestamp}`.
/// Keeps at most MAX_BACKUPS copies, deleting the oldest.
fn backup_file(file_path: &Path, backup_dir: &Path) {
    if !file_path.exists() {
        return;
    }
    let filename = match file_path.file_name().and_then(|f| f.to_str()) {
        Some(f) => f,
        None => return,
    };
    if let Err(e) = std::fs::create_dir_all(backup_dir) {
        warn!(
            "Failed to create backup dir {}: {}",
            backup_dir.display(),
            e
        );
        return;
    }
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup_name = format!("{}.{}", filename, timestamp);
    let backup_path = backup_dir.join(&backup_name);
    if let Err(e) = std::fs::copy(file_path, &backup_path) {
        warn!(
            "Failed to backup {} → {}: {}",
            file_path.display(),
            backup_path.display(),
            e
        );
        return;
    }

    // Prune old backups: list all files matching "{filename}.*", sort, remove oldest
    let prefix = format!("{}.", filename);
    let mut backups: Vec<PathBuf> = std::fs::read_dir(backup_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&prefix) && entry.path().is_file() {
                Some(entry.path())
            } else {
                None
            }
        })
        .collect();

    if backups.len() > MAX_BACKUPS {
        backups.sort();
        for old in &backups[..backups.len() - MAX_BACKUPS] {
            let _ = std::fs::remove_file(old);
        }
    }
}

pub struct ReadFileTool {
    allowed_roots: Option<Vec<PathBuf>>,
}

impl ReadFileTool {
    pub fn new(allowed_roots: Option<Vec<PathBuf>>) -> Self {
        Self { allowed_roots }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file at the given path."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
    }

    fn cacheable(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to read"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

        let file_path = PathBuf::from(path_str);
        let expanded = file_path.canonicalize().or_else(|_| {
            let home = dirs::home_dir().unwrap_or_default();
            let stripped = file_path.strip_prefix("~").unwrap_or(file_path.as_path());
            Ok::<PathBuf, anyhow::Error>(home.join(stripped))
        })?;

        if let Err(err) = check_path_allowed(&expanded, &self.allowed_roots) {
            return Ok(ToolResult::error(err.to_string()));
        }

        if !expanded.exists() {
            return Ok(ToolResult::error(format!(
                "Error: File not found: {}",
                path_str
            )));
        }

        if !expanded.is_file() {
            return Ok(ToolResult::error(format!(
                "Error: Not a file (path is a directory): {}. Use list_dir to list directory contents, or read_file with a file path.",
                path_str
            )));
        }

        match std::fs::read_to_string(&expanded) {
            Ok(content) => Ok(ToolResult::new(content)),
            Err(e) => Ok(ToolResult::error(format!("Error reading file: {}", e))),
        }
    }
}

pub struct WriteFileTool {
    allowed_roots: Option<Vec<PathBuf>>,
    backup_dir: Option<PathBuf>,
}

impl WriteFileTool {
    pub fn new(allowed_roots: Option<Vec<PathBuf>>, backup_dir: Option<PathBuf>) -> Self {
        Self {
            allowed_roots,
            backup_dir,
        }
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file at the given path. Creates parent directories if needed."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to write to"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
        let content = params["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

        let file_path = PathBuf::from(path_str);
        let expanded = file_path.canonicalize().or_else(|_| {
            let home = dirs::home_dir().unwrap_or_default();
            let stripped = file_path.strip_prefix("~").unwrap_or(file_path.as_path());
            Ok::<PathBuf, anyhow::Error>(home.join(stripped))
        })?;

        // Check path restrictions even after fallback canonicalization
        if let Err(err) = check_path_allowed(&expanded, &self.allowed_roots) {
            return Ok(ToolResult::error(err.to_string()));
        }

        if let Some(ref backup_dir) = self.backup_dir {
            backup_file(&expanded, backup_dir);
        }

        if let Some(parent) = expanded.parent() {
            std::fs::create_dir_all(parent)?;
        }

        match std::fs::write(&expanded, content) {
            Ok(_) => Ok(ToolResult::new(format!("File written: {}", path_str))),
            Err(e) => Ok(ToolResult::error(format!("Error writing file: {}", e))),
        }
    }
}

pub struct EditFileTool {
    allowed_roots: Option<Vec<PathBuf>>,
    backup_dir: Option<PathBuf>,
}

impl EditFileTool {
    pub fn new(allowed_roots: Option<Vec<PathBuf>>, backup_dir: Option<PathBuf>) -> Self {
        Self {
            allowed_roots,
            backup_dir,
        }
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing old_text with new_text. The old_text must exist exactly in the file."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to edit"
                },
                "old_text": {
                    "type": "string",
                    "description": "The exact text to find and replace"
                },
                "new_text": {
                    "type": "string",
                    "description": "The text to replace with"
                }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
        let old_text = params["old_text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'old_text' parameter"))?;
        let new_text = params["new_text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'new_text' parameter"))?;

        let file_path = PathBuf::from(path_str);
        let expanded = file_path.canonicalize().or_else(|_| {
            let home = dirs::home_dir().unwrap_or_default();
            let stripped = file_path.strip_prefix("~").unwrap_or(file_path.as_path());
            Ok::<PathBuf, anyhow::Error>(home.join(stripped))
        })?;

        if let Err(err) = check_path_allowed(&expanded, &self.allowed_roots) {
            return Ok(ToolResult::error(err.to_string()));
        }

        if !expanded.exists() {
            return Ok(ToolResult::error(format!(
                "Error: File not found: {}",
                path_str
            )));
        }

        match std::fs::read_to_string(&expanded) {
            Ok(content) => {
                if !content.contains(old_text) {
                    return Ok(ToolResult::error(
                        "Error: old_text not found in file. Make sure it matches exactly."
                            .to_string(),
                    ));
                }

                let count = content.matches(old_text).count();
                if count > 1 {
                    return Ok(ToolResult::error(format!(
                        "Warning: old_text appears {} times. Please provide more context to make it unique.",
                        count
                    )));
                }

                if let Some(ref backup_dir) = self.backup_dir {
                    backup_file(&expanded, backup_dir);
                }

                let new_content = content.replacen(old_text, new_text, 1);
                match std::fs::write(&expanded, new_content) {
                    Ok(_) => Ok(ToolResult::new(format!("Successfully edited {}", path_str))),
                    Err(e) => Ok(ToolResult::error(format!("Error writing file: {}", e))),
                }
            }
            Err(e) => Ok(ToolResult::error(format!("Error reading file: {}", e))),
        }
    }
}

pub struct ListDirTool {
    allowed_roots: Option<Vec<PathBuf>>,
}

impl ListDirTool {
    pub fn new(allowed_roots: Option<Vec<PathBuf>>) -> Self {
        Self { allowed_roots }
    }
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "List the contents of a directory."
    }

    fn cacheable(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The directory path to list"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

        let dir_path = PathBuf::from(path_str);
        let expanded = dir_path.canonicalize().or_else(|_| {
            let home = dirs::home_dir().unwrap_or_default();
            let stripped = dir_path.strip_prefix("~").unwrap_or(dir_path.as_path());
            Ok::<PathBuf, anyhow::Error>(home.join(stripped))
        })?;

        if let Err(err) = check_path_allowed(&expanded, &self.allowed_roots) {
            return Ok(ToolResult::error(err.to_string()));
        }

        if !expanded.exists() {
            return Ok(ToolResult::error(format!(
                "Error: Directory not found: {}",
                path_str
            )));
        }

        if !expanded.is_dir() {
            return Ok(ToolResult::error(format!(
                "Error: Not a directory: {}",
                path_str
            )));
        }

        let mut entries = Vec::new();
        match std::fs::read_dir(&expanded) {
            Ok(rd) => {
                for entry in rd.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let path = entry.path();
                    let is_dir = path.is_dir();
                    entries.push(format!("{}{}", name, if is_dir { "/" } else { "" }));
                }
                entries.sort();
                Ok(ToolResult::new(entries.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!("Error reading directory: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // --- check_path_allowed ---

    #[test]
    fn test_check_path_allowed_none_allows_all() {
        let tmp = std::env::temp_dir();
        assert!(check_path_allowed(&tmp, &None).is_ok());
    }

    #[test]
    fn test_check_path_allowed_within_root() {
        let tmp = std::env::temp_dir();
        let roots = Some(vec![tmp.clone()]);
        assert!(check_path_allowed(&tmp, &roots).is_ok());
    }

    #[test]
    fn test_check_path_allowed_outside_root() {
        let roots = Some(vec![PathBuf::from("/tmp/nanobot_test_nonexistent_root")]);
        let result = check_path_allowed(&std::env::temp_dir(), &roots);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("outside the allowed directories"));
    }

    #[test]
    fn test_check_path_allowed_nonexistent_path() {
        let roots = Some(vec![std::env::temp_dir()]);
        let result = check_path_allowed(Path::new("/tmp/does_not_exist_12345"), &roots);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Cannot resolve path"));
    }

    // --- ReadFileTool ---

    #[tokio::test]
    async fn test_read_file_success() {
        let dir = std::env::temp_dir().join("nanobot_test_read");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.txt");
        fs::write(&file, "hello world").unwrap();

        let tool = ReadFileTool::new(None);
        let result = tool
            .execute(serde_json::json!({"path": file.to_str().unwrap()}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content, "hello world");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let tool = ReadFileTool::new(None);
        let result = tool
            .execute(serde_json::json!({"path": "/tmp/nanobot_nonexistent_file_12345.txt"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found") || result.content.contains("Cannot resolve"));
    }

    #[tokio::test]
    async fn test_read_file_missing_param() {
        let tool = ReadFileTool::new(None);
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_file_not_a_file() {
        let tool = ReadFileTool::new(None);
        let result = tool
            .execute(serde_json::json!({"path": "/tmp"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Not a file (path is a directory)"));
    }

    #[tokio::test]
    async fn test_read_file_path_restriction() {
        let dir = std::env::temp_dir().join("nanobot_test_read_restricted");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("secret.txt");
        fs::write(&file, "secret").unwrap();

        // Allow only a different root
        let other = std::env::temp_dir().join("nanobot_test_other_root");
        fs::create_dir_all(&other).unwrap();
        let tool = ReadFileTool::new(Some(vec![other.clone()]));
        let result = tool
            .execute(serde_json::json!({"path": file.to_str().unwrap()}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("outside the allowed directories"));

        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&other).unwrap();
    }

    // --- WriteFileTool ---

    #[tokio::test]
    async fn test_write_file_success() {
        let dir = std::env::temp_dir().join("nanobot_test_write");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("output.txt");

        let tool = WriteFileTool::new(None, None);
        let result = tool
            .execute(serde_json::json!({"path": file.to_str().unwrap(), "content": "test content"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("File written"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "test content");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn test_write_file_creates_parent_dirs() {
        let dir = std::env::temp_dir().join("nanobot_test_write_nested/a/b/c");
        let file = dir.join("deep.txt");

        let tool = WriteFileTool::new(None, None);
        let result = tool
            .execute(serde_json::json!({"path": file.to_str().unwrap(), "content": "deep"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(fs::read_to_string(&file).unwrap(), "deep");

        fs::remove_dir_all(std::env::temp_dir().join("nanobot_test_write_nested")).unwrap();
    }

    // --- EditFileTool ---

    #[tokio::test]
    async fn test_edit_file_success() {
        let dir = std::env::temp_dir().join("nanobot_test_edit");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("edit.txt");
        fs::write(&file, "hello world").unwrap();

        let tool = EditFileTool::new(None, None);
        let result = tool
            .execute(serde_json::json!({
                "path": file.to_str().unwrap(),
                "old_text": "hello",
                "new_text": "goodbye"
            }))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(fs::read_to_string(&file).unwrap(), "goodbye world");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn test_edit_file_old_text_not_found() {
        let dir = std::env::temp_dir().join("nanobot_test_edit_nf");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("edit.txt");
        fs::write(&file, "hello world").unwrap();

        let tool = EditFileTool::new(None, None);
        let result = tool
            .execute(serde_json::json!({
                "path": file.to_str().unwrap(),
                "old_text": "missing text",
                "new_text": "replacement"
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("old_text not found"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn test_edit_file_ambiguous_match() {
        let dir = std::env::temp_dir().join("nanobot_test_edit_ambig");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("edit.txt");
        fs::write(&file, "foo bar foo baz").unwrap();

        let tool = EditFileTool::new(None, None);
        let result = tool
            .execute(serde_json::json!({
                "path": file.to_str().unwrap(),
                "old_text": "foo",
                "new_text": "qux"
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("appears 2 times"));

        fs::remove_dir_all(&dir).unwrap();
    }

    // --- ListDirTool ---

    #[tokio::test]
    async fn test_list_dir_success() {
        let dir = std::env::temp_dir().join("nanobot_test_listdir");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "").unwrap();
        fs::write(dir.join("b.txt"), "").unwrap();
        fs::create_dir_all(dir.join("subdir")).unwrap();

        let tool = ListDirTool::new(None);
        let result = tool
            .execute(serde_json::json!({"path": dir.to_str().unwrap()}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("a.txt"));
        assert!(result.content.contains("b.txt"));
        assert!(result.content.contains("subdir/"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn test_list_dir_not_found() {
        let tool = ListDirTool::new(None);
        let result = tool
            .execute(serde_json::json!({"path": "/tmp/nanobot_nonexistent_dir_12345"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found") || result.content.contains("Cannot resolve"));
    }

    #[tokio::test]
    async fn test_list_dir_not_a_directory() {
        let dir = std::env::temp_dir().join("nanobot_test_listdir_file");
        fs::create_dir_all(dir.parent().unwrap()).unwrap();
        fs::write(&dir, "not a dir").unwrap();

        let tool = ListDirTool::new(None);
        let result = tool
            .execute(serde_json::json!({"path": dir.to_str().unwrap()}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Not a directory"));

        fs::remove_file(&dir).unwrap();
    }

    // --- backup_file ---

    #[test]
    fn test_backup_creates_copy() {
        let dir = std::env::temp_dir().join("nanobot_test_backup_basic");
        let backup_dir = dir.join("backups");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.md");
        fs::write(&file, "original content").unwrap();

        backup_file(&file, &backup_dir);

        assert!(backup_dir.exists());
        let backups: Vec<_> = fs::read_dir(&backup_dir).unwrap().flatten().collect();
        assert_eq!(backups.len(), 1);
        let backup_content = fs::read_to_string(backups[0].path()).unwrap();
        assert_eq!(backup_content, "original content");
        let name = backups[0].file_name().to_string_lossy().to_string();
        assert!(
            name.starts_with("test.md."),
            "backup name should be prefixed: {}",
            name
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_backup_skips_nonexistent_file() {
        let dir = std::env::temp_dir().join("nanobot_test_backup_skip");
        let backup_dir = dir.join("backups");
        let _ = fs::remove_dir_all(&dir);

        backup_file(&dir.join("nope.md"), &backup_dir);

        assert!(!backup_dir.exists());
    }

    #[test]
    fn test_backup_prunes_old_copies() {
        let dir = std::env::temp_dir().join("nanobot_test_backup_prune");
        let backup_dir = dir.join("backups");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&backup_dir).unwrap();

        let file = dir.join("data.md");
        fs::write(&file, "content").unwrap();

        // Create 16 fake old backups (exceed MAX_BACKUPS of 14)
        for i in 0..16 {
            let name = format!("data.md.20250101-{:06}", i);
            fs::write(backup_dir.join(&name), format!("v{}", i)).unwrap();
        }

        // Trigger backup which should prune to 14
        backup_file(&file, &backup_dir);

        let count = fs::read_dir(&backup_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("data.md."))
            .count();
        assert_eq!(
            count, MAX_BACKUPS,
            "should keep exactly {} backups",
            MAX_BACKUPS
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn test_write_file_creates_backup() {
        let dir = std::env::temp_dir().join("nanobot_test_write_backup");
        let backup_dir = dir.join("backups");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let file = dir.join("target.md");
        fs::write(&file, "before").unwrap();

        let tool = WriteFileTool::new(None, Some(backup_dir.clone()));
        let result = tool
            .execute(serde_json::json!({"path": file.to_str().unwrap(), "content": "after"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(fs::read_to_string(&file).unwrap(), "after");

        let backups: Vec<_> = fs::read_dir(&backup_dir).unwrap().flatten().collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read_to_string(backups[0].path()).unwrap(), "before");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn test_edit_file_creates_backup() {
        let dir = std::env::temp_dir().join("nanobot_test_edit_backup");
        let backup_dir = dir.join("backups");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let file = dir.join("target.md");
        fs::write(&file, "hello world").unwrap();

        let tool = EditFileTool::new(None, Some(backup_dir.clone()));
        let result = tool
            .execute(serde_json::json!({
                "path": file.to_str().unwrap(),
                "old_text": "hello",
                "new_text": "goodbye"
            }))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(fs::read_to_string(&file).unwrap(), "goodbye world");

        let backups: Vec<_> = fs::read_dir(&backup_dir).unwrap().flatten().collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read_to_string(backups[0].path()).unwrap(),
            "hello world"
        );

        fs::remove_dir_all(&dir).unwrap();
    }
}
