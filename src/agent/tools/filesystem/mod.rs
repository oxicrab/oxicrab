use crate::agent::tools::base::ExecutionContext;
use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tracing::warn;

/// Maximum file size that `read_file` will load (10 MB).
const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;

fn check_path_allowed(file_path: &Path, allowed_roots: Option<&Vec<PathBuf>>) -> Result<()> {
    if let Some(roots) = allowed_roots {
        let resolved = file_path
            .canonicalize()
            .map_err(|_| anyhow::anyhow!("Error: Cannot resolve path '{}'", file_path.display()))?;
        for root in roots {
            if let Ok(root_resolved) = root.canonicalize()
                && (resolved == root_resolved || resolved.starts_with(&root_resolved))
            {
                return Ok(());
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
/// Keeps at most `MAX_BACKUPS` copies, deleting the oldest.
fn backup_file(file_path: &Path, backup_dir: &Path) {
    if !file_path.exists() {
        return;
    }
    let Some(filename) = file_path.file_name().and_then(|f| f.to_str()) else {
        return;
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
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
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

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

        let file_path = PathBuf::from(path_str);
        let expanded = file_path.canonicalize().or_else(|_| {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
            let stripped = file_path.strip_prefix("~").unwrap_or(file_path.as_path());
            Ok::<PathBuf, anyhow::Error>(home.join(stripped))
        })?;

        if let Err(err) = check_path_allowed(&expanded, self.allowed_roots.as_ref()) {
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

        // Check file size before reading to prevent OOM on huge files
        match std::fs::metadata(&expanded) {
            Ok(meta) if meta.len() > MAX_READ_BYTES => {
                return Ok(ToolResult::error(format!(
                    "Error: file too large ({} bytes, max {}). Use shell tool to read partial content.",
                    meta.len(),
                    MAX_READ_BYTES
                )));
            }
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Error reading file metadata: {}",
                    e
                )));
            }
            _ => {}
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
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
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

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
        let content = params["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

        let file_path = PathBuf::from(path_str);
        let expanded = file_path.canonicalize().or_else(|_| {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
            let stripped = file_path.strip_prefix("~").unwrap_or(file_path.as_path());
            Ok::<PathBuf, anyhow::Error>(home.join(stripped))
        })?;

        // Check path restrictions even after fallback canonicalization
        if let Err(err) = check_path_allowed(&expanded, self.allowed_roots.as_ref()) {
            return Ok(ToolResult::error(err.to_string()));
        }

        if let Some(ref backup_dir) = self.backup_dir {
            backup_file(&expanded, backup_dir);
        }

        if let Some(parent) = expanded.parent() {
            std::fs::create_dir_all(parent)?;
        }

        match std::fs::write(&expanded, content) {
            Ok(()) => Ok(ToolResult::new(format!("File written: {}", path_str))),
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
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn description(&self) -> &'static str {
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

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
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
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
            let stripped = file_path.strip_prefix("~").unwrap_or(file_path.as_path());
            Ok::<PathBuf, anyhow::Error>(home.join(stripped))
        })?;

        if let Err(err) = check_path_allowed(&expanded, self.allowed_roots.as_ref()) {
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
                    Ok(()) => Ok(ToolResult::new(format!("Successfully edited {}", path_str))),
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
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn description(&self) -> &'static str {
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

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

        let dir_path = PathBuf::from(path_str);
        let expanded = dir_path.canonicalize().or_else(|_| {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
            let stripped = dir_path.strip_prefix("~").unwrap_or(dir_path.as_path());
            Ok::<PathBuf, anyhow::Error>(home.join(stripped))
        })?;

        if let Err(err) = check_path_allowed(&expanded, self.allowed_roots.as_ref()) {
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
mod tests;
