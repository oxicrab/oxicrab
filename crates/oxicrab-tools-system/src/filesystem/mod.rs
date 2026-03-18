use crate::shell::lexical_normalize;
use crate::utils::path_sanitize::sanitize_error_message;
use anyhow::Result;
use async_trait::async_trait;
use oxicrab_core::require_param;
use oxicrab_core::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use oxicrab_core::tools::base::{Tool, ToolResult};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::warn;

/// Maximum file size that `read_file` will load (10 MB).
const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;

fn resolve_path(file_path: &Path) -> PathBuf {
    file_path.canonicalize().unwrap_or_else(|_| {
        if let (Some(parent), Some(file_name)) = (file_path.parent(), file_path.file_name())
            && let Ok(parent_resolved) = parent.canonicalize()
        {
            return parent_resolved.join(file_name);
        }
        lexical_normalize(file_path)
    })
}

fn check_path_allowed(file_path: &Path, allowed_roots: Option<&Vec<PathBuf>>) -> Result<()> {
    if let Some(roots) = allowed_roots {
        let resolved = resolve_path(file_path);
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

fn open_confined(target: &Path, allowed_roots: &[PathBuf]) -> Result<(cap_std::fs::Dir, PathBuf)> {
    let resolved = resolve_path(target);

    for root in allowed_roots {
        let Ok(root_resolved) = root.canonicalize() else {
            continue;
        };
        if resolved == root_resolved || resolved.starts_with(&root_resolved) {
            let relative = resolved
                .strip_prefix(&root_resolved)
                .unwrap_or(&resolved)
                .to_path_buf();
            let dir =
                cap_std::fs::Dir::open_ambient_dir(&root_resolved, cap_std::ambient_authority())
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "failed to open confined root '{}': {}",
                            root_resolved.display(),
                            e
                        )
                    })?;
            return Ok((dir, relative));
        }
    }

    anyhow::bail!(
        "Error: Path '{}' is outside the allowed directories",
        target.display()
    );
}

fn sanitize_err(msg: &str, workspace: Option<&Path>) -> String {
    sanitize_error_message(msg, workspace)
}

const MAX_BACKUPS: usize = 14;

async fn backup_file(file_path: &Path, backup_dir: &Path) {
    if tokio::fs::metadata(file_path).await.is_err() {
        return;
    }
    let Some(filename) = file_path.file_name().and_then(|f| f.to_str()) else {
        return;
    };
    if let Err(e) = tokio::fs::create_dir_all(backup_dir).await {
        warn!(
            "Failed to create backup dir {}: {}",
            backup_dir.display(),
            e
        );
        return;
    }
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup_name = format!("{filename}.{timestamp}");
    let backup_path = backup_dir.join(&backup_name);
    if let Err(e) = tokio::fs::copy(file_path, &backup_path).await {
        warn!(
            "Failed to backup {} → {}: {}",
            file_path.display(),
            backup_path.display(),
            e
        );
        return;
    }

    let prefix = format!("{filename}.");
    let mut backups: Vec<PathBuf> = Vec::new();
    if let Ok(mut rd) = tokio::fs::read_dir(backup_dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if name.starts_with(&prefix)
                && tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_file())
            {
                backups.push(path);
            }
        }
    }

    if backups.len() > MAX_BACKUPS {
        backups.sort();
        for old in &backups[..backups.len() - MAX_BACKUPS] {
            let _ = tokio::fs::remove_file(old).await;
        }
    }
}

/// Trait for workspace manager integration (optional).
pub trait WorkspaceFileTracker: Send + Sync {
    fn is_managed_path(&self, path: &Path) -> bool;
    fn touch_file(&self, path: &Path) -> Result<()>;
    fn register_file(
        &self,
        path: &Path,
        source_tool: Option<&str>,
        session_key: Option<&str>,
    ) -> Result<()>;
}

pub struct ReadFileTool {
    allowed_roots: Option<Vec<PathBuf>>,
    workspace: Option<PathBuf>,
    workspace_manager: Option<Arc<dyn WorkspaceFileTracker>>,
}

impl ReadFileTool {
    pub fn new(allowed_roots: Option<Vec<PathBuf>>, workspace: Option<PathBuf>) -> Self {
        Self {
            allowed_roots,
            workspace,
            workspace_manager: None,
        }
    }

    #[must_use]
    pub fn with_workspace_manager(mut self, mgr: Arc<dyn WorkspaceFileTracker>) -> Self {
        self.workspace_manager = Some(mgr);
        self
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

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            subagent_access: SubagentAccess::Full,
            ..Default::default()
        }
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
        let path_str = require_param!(params, "path");

        let file_path = PathBuf::from(path_str);
        let expanded = tokio::fs::canonicalize(&file_path).await.or_else(|_| {
            if file_path.starts_with("~") {
                let home = dirs::home_dir()
                    .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
                let stripped = file_path.strip_prefix("~").unwrap_or(file_path.as_path());
                Ok::<PathBuf, anyhow::Error>(home.join(stripped))
            } else {
                Ok(lexical_normalize(&file_path))
            }
        })?;

        let ws = self.workspace.as_deref();

        let result = if let Some(ref roots) = self.allowed_roots {
            let (dir, relative) = match open_confined(&expanded, roots) {
                Ok(v) => v,
                Err(e) => return Ok(ToolResult::error(sanitize_err(&e.to_string(), ws))),
            };

            let path_str_owned = path_str.to_string();
            let ws_owned = ws.map(Path::to_path_buf);
            tokio::task::spawn_blocking(move || {
                let ws_ref = ws_owned.as_deref();
                let target = if relative.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    relative
                };
                let Ok(meta) = dir.metadata(&target) else {
                    return Ok(ToolResult::error(format!(
                        "file not found: {path_str_owned}"
                    )));
                };
                if meta.is_dir() {
                    return Ok(ToolResult::error(format!(
                        "not a file (path is a directory): {path_str_owned}. Use list_dir to list directory contents, or read_file with a file path"
                    )));
                }
                if meta.len() > MAX_READ_BYTES {
                    return Ok(ToolResult::error(format!(
                        "file too large ({} bytes, max {}). Use shell tool to read partial content",
                        meta.len(),
                        MAX_READ_BYTES
                    )));
                }
                match dir.read_to_string(&target) {
                    Ok(content) => Ok(ToolResult::new(content)),
                    Err(e) => Ok(ToolResult::error(sanitize_err(
                        &format!("error reading file: {e}"),
                        ws_ref,
                    ))),
                }
            })
            .await?
        } else {
            if let Err(err) = check_path_allowed(&expanded, self.allowed_roots.as_ref()) {
                return Ok(ToolResult::error(sanitize_err(&err.to_string(), ws)));
            }

            match tokio::fs::metadata(&expanded).await {
                Ok(meta) if meta.is_dir() => {
                    return Ok(ToolResult::error(format!(
                        "not a file (path is a directory): {path_str}. Use list_dir to list directory contents, or read_file with a file path"
                    )));
                }
                Ok(meta) if meta.len() > MAX_READ_BYTES => {
                    return Ok(ToolResult::error(format!(
                        "file too large ({} bytes, max {}). Use shell tool to read partial content",
                        meta.len(),
                        MAX_READ_BYTES
                    )));
                }
                Err(_) => {
                    return Ok(ToolResult::error(format!("file not found: {path_str}")));
                }
                _ => {}
            }

            match tokio::fs::read_to_string(&expanded).await {
                Ok(content) => Ok(ToolResult::new(content)),
                Err(e) => Ok(ToolResult::error(sanitize_err(
                    &format!("error reading file: {e}"),
                    ws,
                ))),
            }
        };

        if let Ok(ref r) = result
            && !r.is_error
            && let Some(ref mgr) = self.workspace_manager
            && mgr.is_managed_path(&expanded)
        {
            let _ = mgr.touch_file(&expanded);
        }

        result
    }
}

pub struct WriteFileTool {
    allowed_roots: Option<Vec<PathBuf>>,
    backup_dir: Option<PathBuf>,
    workspace: Option<PathBuf>,
    workspace_manager: Option<Arc<dyn WorkspaceFileTracker>>,
}

impl WriteFileTool {
    pub fn new(
        allowed_roots: Option<Vec<PathBuf>>,
        backup_dir: Option<PathBuf>,
        workspace: Option<PathBuf>,
    ) -> Self {
        Self {
            allowed_roots,
            backup_dir,
            workspace,
            workspace_manager: None,
        }
    }

    #[must_use]
    pub fn with_workspace_manager(mut self, mgr: Arc<dyn WorkspaceFileTracker>) -> Self {
        self.workspace_manager = Some(mgr);
        self
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

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            subagent_access: SubagentAccess::Full,
            ..Default::default()
        }
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
        let path_str = require_param!(params, "path");
        let content = require_param!(params, "content");

        let file_path = PathBuf::from(path_str);
        let expanded = tokio::fs::canonicalize(&file_path).await.or_else(|_| {
            if file_path.starts_with("~") {
                let home = dirs::home_dir()
                    .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
                let stripped = file_path.strip_prefix("~").unwrap_or(file_path.as_path());
                Ok::<PathBuf, anyhow::Error>(home.join(stripped))
            } else {
                Ok(lexical_normalize(&file_path))
            }
        })?;

        let ws = self.workspace.as_deref();

        let result = if let Some(ref roots) = self.allowed_roots {
            let (dir, relative) = match open_confined(&expanded, roots) {
                Ok(v) => v,
                Err(e) => return Ok(ToolResult::error(sanitize_err(&e.to_string(), ws))),
            };

            if let Some(ref backup_dir) = self.backup_dir {
                backup_file(&expanded, backup_dir).await;
            }

            let content_owned = content.to_string();
            let path_str_owned = path_str.to_string();
            let ws_owned = ws.map(Path::to_path_buf);
            tokio::task::spawn_blocking(move || {
                let ws_ref = ws_owned.as_deref();
                if let Some(parent) = relative.parent()
                    && !parent.as_os_str().is_empty()
                {
                    dir.create_dir_all(parent)
                        .map_err(|e| anyhow::anyhow!("failed to create parent directories: {e}"))?;
                }
                match dir.write(&relative, &content_owned) {
                    Ok(()) => Ok(ToolResult::new(format!("File written: {path_str_owned}"))),
                    Err(e) => Ok(ToolResult::error(sanitize_err(
                        &format!("error writing file: {e}"),
                        ws_ref,
                    ))),
                }
            })
            .await?
        } else {
            if let Err(err) = check_path_allowed(&expanded, self.allowed_roots.as_ref()) {
                return Ok(ToolResult::error(sanitize_err(&err.to_string(), ws)));
            }

            if let Some(ref backup_dir) = self.backup_dir {
                backup_file(&expanded, backup_dir).await;
            }

            if let Some(parent) = expanded.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }

            match tokio::fs::write(&expanded, content).await {
                Ok(()) => Ok(ToolResult::new(format!("File written: {path_str}"))),
                Err(e) => Ok(ToolResult::error(sanitize_err(
                    &format!("error writing file: {e}"),
                    ws,
                ))),
            }
        };

        if let Ok(ref r) = result
            && !r.is_error
            && let Some(ref mgr) = self.workspace_manager
            && mgr.is_managed_path(&expanded)
            && let Err(e) = mgr.register_file(&expanded, Some("write_file"), None)
        {
            warn!("failed to register workspace file: {e}");
        }

        result
    }
}

pub struct EditFileTool {
    allowed_roots: Option<Vec<PathBuf>>,
    backup_dir: Option<PathBuf>,
    workspace: Option<PathBuf>,
}

impl EditFileTool {
    pub fn new(
        allowed_roots: Option<Vec<PathBuf>>,
        backup_dir: Option<PathBuf>,
        workspace: Option<PathBuf>,
    ) -> Self {
        Self {
            allowed_roots,
            backup_dir,
            workspace,
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

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            subagent_access: SubagentAccess::Full,
            ..Default::default()
        }
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
        let path_str = require_param!(params, "path");
        let old_text = require_param!(params, "old_text");
        let new_text = require_param!(params, "new_text");

        let file_path = PathBuf::from(path_str);
        let expanded = tokio::fs::canonicalize(&file_path).await.or_else(|_| {
            if file_path.starts_with("~") {
                let home = dirs::home_dir()
                    .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
                let stripped = file_path.strip_prefix("~").unwrap_or(file_path.as_path());
                Ok::<PathBuf, anyhow::Error>(home.join(stripped))
            } else {
                Ok(lexical_normalize(&file_path))
            }
        })?;

        let ws = self.workspace.as_deref();

        if let Some(ref roots) = self.allowed_roots {
            let (dir, relative) = match open_confined(&expanded, roots) {
                Ok(v) => v,
                Err(e) => return Ok(ToolResult::error(sanitize_err(&e.to_string(), ws))),
            };

            if let Some(ref backup_dir) = self.backup_dir {
                backup_file(&expanded, backup_dir).await;
            }

            let old_text_owned = old_text.to_string();
            let new_text_owned = new_text.to_string();
            let path_str_owned = path_str.to_string();
            let ws_owned = ws.map(Path::to_path_buf);
            return tokio::task::spawn_blocking(move || {
                let ws_ref = ws_owned.as_deref();
                match dir.metadata(&relative) {
                    Ok(meta) if meta.len() > MAX_READ_BYTES => {
                        return Ok(ToolResult::error(format!(
                            "file too large for edit ({} bytes, max {})",
                            meta.len(),
                            MAX_READ_BYTES
                        )));
                    }
                    Err(_) => {
                        return Ok(ToolResult::error(format!(
                            "file not found: {path_str_owned}"
                        )));
                    }
                    _ => {}
                }
                let Ok(content) = dir.read_to_string(&relative) else {
                    return Ok(ToolResult::error(format!(
                        "file not found: {path_str_owned}"
                    )));
                };

                if !content.contains(&*old_text_owned) {
                    return Ok(ToolResult::error(
                        "old_text not found in file. Make sure it matches exactly".to_string(),
                    ));
                }

                let count = content.matches(&*old_text_owned).count();
                if count > 1 {
                    return Ok(ToolResult::error(format!(
                        "old_text appears {count} times. Please provide more context to make it unique"
                    )));
                }

                let new_content = content.replacen(&*old_text_owned, &new_text_owned, 1);
                match dir.write(&relative, &new_content) {
                    Ok(()) => Ok(ToolResult::new(format!(
                        "Successfully edited {path_str_owned}"
                    ))),
                    Err(e) => Ok(ToolResult::error(sanitize_err(
                        &format!("error writing file: {e}"),
                        ws_ref,
                    ))),
                }
            })
            .await?;
        }

        if let Err(err) = check_path_allowed(&expanded, self.allowed_roots.as_ref()) {
            return Ok(ToolResult::error(sanitize_err(&err.to_string(), ws)));
        }

        match tokio::fs::metadata(&expanded).await {
            Ok(meta) if meta.len() > MAX_READ_BYTES => {
                return Ok(ToolResult::error(format!(
                    "file too large for edit ({} bytes, max {})",
                    meta.len(),
                    MAX_READ_BYTES
                )));
            }
            Err(_) => {
                return Ok(ToolResult::error(format!("file not found: {path_str}")));
            }
            _ => {}
        }

        match tokio::fs::read_to_string(&expanded).await {
            Ok(content) => {
                if !content.contains(old_text) {
                    return Ok(ToolResult::error(
                        "old_text not found in file. Make sure it matches exactly".to_string(),
                    ));
                }

                let count = content.matches(old_text).count();
                if count > 1 {
                    return Ok(ToolResult::error(format!(
                        "old_text appears {count} times. Please provide more context to make it unique"
                    )));
                }

                if let Some(ref backup_dir) = self.backup_dir {
                    backup_file(&expanded, backup_dir).await;
                }

                let new_content = content.replacen(old_text, new_text, 1);
                match tokio::fs::write(&expanded, new_content).await {
                    Ok(()) => Ok(ToolResult::new(format!("Successfully edited {path_str}"))),
                    Err(e) => Ok(ToolResult::error(sanitize_err(
                        &format!("error writing file: {e}"),
                        ws,
                    ))),
                }
            }
            Err(e) => Ok(ToolResult::error(sanitize_err(
                &format!("error reading file: {e}"),
                ws,
            ))),
        }
    }
}

pub struct ListDirTool {
    allowed_roots: Option<Vec<PathBuf>>,
    workspace: Option<PathBuf>,
}

impl ListDirTool {
    pub fn new(allowed_roots: Option<Vec<PathBuf>>, workspace: Option<PathBuf>) -> Self {
        Self {
            allowed_roots,
            workspace,
        }
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

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            subagent_access: SubagentAccess::Full,
            ..Default::default()
        }
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
        let path_str = require_param!(params, "path");

        let dir_path = PathBuf::from(path_str);
        let expanded = tokio::fs::canonicalize(&dir_path).await.or_else(|_| {
            if dir_path.starts_with("~") {
                let home = dirs::home_dir()
                    .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
                let stripped = dir_path.strip_prefix("~").unwrap_or(dir_path.as_path());
                Ok::<PathBuf, anyhow::Error>(home.join(stripped))
            } else {
                Ok(lexical_normalize(&dir_path))
            }
        })?;

        let ws = self.workspace.as_deref();

        if let Some(ref roots) = self.allowed_roots {
            let (dir, relative) = match open_confined(&expanded, roots) {
                Ok(v) => v,
                Err(e) => return Ok(ToolResult::error(sanitize_err(&e.to_string(), ws))),
            };

            let path_str_owned = path_str.to_string();
            let ws_owned = ws.map(Path::to_path_buf);
            return tokio::task::spawn_blocking(move || {
                let ws_ref = ws_owned.as_deref();
                let target = if relative.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    relative
                };
                let Ok(meta) = dir.metadata(&target) else {
                    return Ok(ToolResult::error(format!(
                        "directory not found: {path_str_owned}"
                    )));
                };
                if !meta.is_dir() {
                    return Ok(ToolResult::error(format!(
                        "not a directory: {path_str_owned}"
                    )));
                }
                let Ok(subdir) = dir.open_dir(&target) else {
                    return Ok(ToolResult::error("error reading directory".to_string()));
                };
                let mut entries = Vec::new();
                match subdir.entries() {
                    Ok(iter) => {
                        for entry in iter {
                            let Ok(entry) = entry else { continue };
                            let name = entry.file_name().to_string_lossy().to_string();
                            let is_dir = entry.metadata().is_ok_and(|m| m.is_dir());
                            entries.push(format!("{}{}", name, if is_dir { "/" } else { "" }));
                        }
                        entries.sort();
                        Ok(ToolResult::new(entries.join("\n")))
                    }
                    Err(e) => Ok(ToolResult::error(sanitize_err(
                        &format!("error reading directory: {e}"),
                        ws_ref,
                    ))),
                }
            })
            .await?;
        }

        if let Err(err) = check_path_allowed(&expanded, self.allowed_roots.as_ref()) {
            return Ok(ToolResult::error(sanitize_err(&err.to_string(), ws)));
        }

        match tokio::fs::metadata(&expanded).await {
            Err(_) => {
                return Ok(ToolResult::error(format!(
                    "directory not found: {path_str}"
                )));
            }
            Ok(meta) if !meta.is_dir() => {
                return Ok(ToolResult::error(format!("not a directory: {path_str}")));
            }
            _ => {}
        }

        let mut entries = Vec::new();
        match tokio::fs::read_dir(&expanded).await {
            Ok(mut rd) => {
                while let Ok(Some(entry)) = rd.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.metadata().await.is_ok_and(|m| m.is_dir());
                    entries.push(format!("{}{}", name, if is_dir { "/" } else { "" }));
                }
                entries.sort();
                Ok(ToolResult::new(entries.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(sanitize_err(
                &format!("error reading directory: {e}"),
                ws,
            ))),
        }
    }
}

#[cfg(test)]
mod tests;
