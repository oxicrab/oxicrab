use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use crate::agent::workspace::WorkspaceManager;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::warn;

/// Maximum file size that `read_file` will load (10 MB).
const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;

fn resolve_path(file_path: &Path) -> PathBuf {
    // Use canonicalize if the path exists (resolves symlinks).
    // For non-existent paths (e.g. write to new file), try to canonicalize
    // the parent directory (to resolve symlinks like /var → /private/var on
    // macOS) and append the filename. Final fallback: lexical normalization.
    file_path.canonicalize().unwrap_or_else(|_| {
        if let (Some(parent), Some(file_name)) = (file_path.parent(), file_path.file_name())
            && let Ok(parent_resolved) = parent.canonicalize()
        {
            return parent_resolved.join(file_name);
        }
        crate::agent::tools::shell::lexical_normalize(file_path)
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

/// Open a capability-based directory handle for confined path operations.
///
/// Uses `cap_std::fs::Dir` (backed by `openat()`) to eliminate TOCTOU race
/// conditions between path validation and file operations. Returns the Dir
/// handle and the relative path from root to target.
fn open_confined(target: &Path, allowed_roots: &[PathBuf]) -> Result<(cap_std::fs::Dir, PathBuf)> {
    // Resolve the target path: canonicalize for existing paths, resolve parent for new files
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
            // Open the root as a capability-based Dir (uses openat under the hood)
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

/// Sanitize a path in an error message if workspace is available.
fn sanitize_err(msg: &str, workspace: Option<&Path>) -> String {
    crate::utils::path_sanitize::sanitize_error_message(msg, workspace)
}

const MAX_BACKUPS: usize = 14;

/// Create a timestamped backup of a file before overwriting it.
/// Backups are stored in `backup_dir/{filename}.{timestamp}`.
/// Keeps at most `MAX_BACKUPS` copies, deleting the oldest.
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
    let backup_name = format!("{}.{}", filename, timestamp);
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

    // Prune old backups: list all files matching "{filename}.*", sort, remove oldest
    let prefix = format!("{}.", filename);
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

pub struct ReadFileTool {
    allowed_roots: Option<Vec<PathBuf>>,
    workspace: Option<PathBuf>,
    workspace_manager: Option<Arc<WorkspaceManager>>,
}

impl ReadFileTool {
    pub fn new(allowed_roots: Option<Vec<PathBuf>>, workspace: Option<PathBuf>) -> Self {
        Self {
            allowed_roots,
            workspace,
            workspace_manager: None,
        }
    }

    /// Set the workspace manager for `accessed_at` tracking.
    #[must_use]
    pub fn with_workspace_manager(mut self, mgr: Arc<WorkspaceManager>) -> Self {
        self.workspace_manager = Some(mgr);
        self
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

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: false,
            subagent_access: SubagentAccess::Full,
            actions: vec![],
        }
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
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
        let expanded = tokio::fs::canonicalize(&file_path).await.or_else(|_| {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
            let stripped = file_path.strip_prefix("~").unwrap_or(file_path.as_path());
            Ok::<PathBuf, anyhow::Error>(home.join(stripped))
        })?;

        let ws = self.workspace.as_deref();

        // When allowed_roots is set, use cap-std for TOCTOU-safe confined reads
        let result = if let Some(ref roots) = self.allowed_roots {
            let (dir, relative) = match open_confined(&expanded, roots) {
                Ok(v) => v,
                Err(e) => return Ok(ToolResult::error(sanitize_err(&e.to_string(), ws))),
            };

            // All operations below use the confined Dir handle (openat)
            let path_str_owned = path_str.to_string();
            let ws_owned = ws.map(Path::to_path_buf);
            tokio::task::spawn_blocking(move || {
                let ws_ref = ws_owned.as_deref();
                // For root-relative access (e.g. open_confined returns "" for the root itself)
                let target = if relative.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    relative
                };
                let Ok(meta) = dir.metadata(&target) else {
                    return Ok(ToolResult::error(format!(
                        "file not found: {}",
                        path_str_owned
                    )));
                };
                if meta.is_dir() {
                    return Ok(ToolResult::error(format!(
                        "not a file (path is a directory): {}. Use list_dir to list directory contents, or read_file with a file path",
                        path_str_owned
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
                        &format!("error reading file: {}", e),
                        ws_ref,
                    ))),
                }
            })
            .await?
        } else {
            // Unrestricted mode: use direct tokio::fs operations
            if let Err(err) = check_path_allowed(&expanded, self.allowed_roots.as_ref()) {
                return Ok(ToolResult::error(sanitize_err(&err.to_string(), ws)));
            }

            match tokio::fs::metadata(&expanded).await {
                Ok(meta) if meta.is_dir() => {
                    return Ok(ToolResult::error(format!(
                        "not a file (path is a directory): {}. Use list_dir to list directory contents, or read_file with a file path",
                        path_str
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
                    return Ok(ToolResult::error(format!("file not found: {}", path_str)));
                }
                _ => {}
            }

            match tokio::fs::read_to_string(&expanded).await {
                Ok(content) => Ok(ToolResult::new(content)),
                Err(e) => Ok(ToolResult::error(sanitize_err(
                    &format!("error reading file: {}", e),
                    ws,
                ))),
            }
        };

        // Track accessed_at for managed workspace files (fire-and-forget)
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
    workspace_manager: Option<Arc<WorkspaceManager>>,
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

    /// Set the workspace manager for manifest registration.
    #[must_use]
    pub fn with_workspace_manager(mut self, mgr: Arc<WorkspaceManager>) -> Self {
        self.workspace_manager = Some(mgr);
        self
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

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: false,
            subagent_access: SubagentAccess::Full,
            actions: vec![],
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
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
        let content = params["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

        let file_path = PathBuf::from(path_str);
        let expanded = tokio::fs::canonicalize(&file_path).await.or_else(|_| {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
            let stripped = file_path.strip_prefix("~").unwrap_or(file_path.as_path());
            Ok::<PathBuf, anyhow::Error>(home.join(stripped))
        })?;

        let ws = self.workspace.as_deref();

        // Confined write when allowed_roots is set
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
                // Create parent dirs within the confined root
                if let Some(parent) = relative.parent()
                    && !parent.as_os_str().is_empty()
                {
                    dir.create_dir_all(parent).map_err(|e| {
                        anyhow::anyhow!("failed to create parent directories: {}", e)
                    })?;
                }
                match dir.write(&relative, &content_owned) {
                    Ok(()) => Ok(ToolResult::new(format!("File written: {}", path_str_owned))),
                    Err(e) => Ok(ToolResult::error(sanitize_err(
                        &format!("error writing file: {}", e),
                        ws_ref,
                    ))),
                }
            })
            .await?
        } else {
            // Unrestricted mode
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
                Ok(()) => Ok(ToolResult::new(format!("File written: {}", path_str))),
                Err(e) => Ok(ToolResult::error(sanitize_err(
                    &format!("error writing file: {}", e),
                    ws,
                ))),
            }
        };

        // Register in workspace manifest for managed files (fire-and-forget)
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
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn description(&self) -> &'static str {
        "Edit a file by replacing old_text with new_text. The old_text must exist exactly in the file."
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: false,
            subagent_access: SubagentAccess::Denied,
            actions: vec![],
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
        let expanded = tokio::fs::canonicalize(&file_path).await.or_else(|_| {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
            let stripped = file_path.strip_prefix("~").unwrap_or(file_path.as_path());
            Ok::<PathBuf, anyhow::Error>(home.join(stripped))
        })?;

        let ws = self.workspace.as_deref();

        // Confined edit when allowed_roots is set
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
                let Ok(content) = dir.read_to_string(&relative) else {
                    return Ok(ToolResult::error(format!(
                        "file not found: {}",
                        path_str_owned
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
                        "old_text appears {} times. Please provide more context to make it unique",
                        count
                    )));
                }

                let new_content = content.replacen(&*old_text_owned, &new_text_owned, 1);
                match dir.write(&relative, &new_content) {
                    Ok(()) => Ok(ToolResult::new(format!(
                        "Successfully edited {}",
                        path_str_owned
                    ))),
                    Err(e) => Ok(ToolResult::error(sanitize_err(
                        &format!("error writing file: {}", e),
                        ws_ref,
                    ))),
                }
            })
            .await?;
        }

        // Unrestricted mode
        if let Err(err) = check_path_allowed(&expanded, self.allowed_roots.as_ref()) {
            return Ok(ToolResult::error(sanitize_err(&err.to_string(), ws)));
        }

        if tokio::fs::metadata(&expanded).await.is_err() {
            return Ok(ToolResult::error(format!("file not found: {}", path_str)));
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
                        "old_text appears {} times. Please provide more context to make it unique",
                        count
                    )));
                }

                if let Some(ref backup_dir) = self.backup_dir {
                    backup_file(&expanded, backup_dir).await;
                }

                let new_content = content.replacen(old_text, new_text, 1);
                match tokio::fs::write(&expanded, new_content).await {
                    Ok(()) => Ok(ToolResult::new(format!("Successfully edited {}", path_str))),
                    Err(e) => Ok(ToolResult::error(sanitize_err(
                        &format!("error writing file: {}", e),
                        ws,
                    ))),
                }
            }
            Err(e) => Ok(ToolResult::error(sanitize_err(
                &format!("error reading file: {}", e),
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
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn description(&self) -> &'static str {
        "List the contents of a directory."
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: false,
            subagent_access: SubagentAccess::Full,
            actions: vec![],
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
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

        let dir_path = PathBuf::from(path_str);
        let expanded = tokio::fs::canonicalize(&dir_path).await.or_else(|_| {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
            let stripped = dir_path.strip_prefix("~").unwrap_or(dir_path.as_path());
            Ok::<PathBuf, anyhow::Error>(home.join(stripped))
        })?;

        let ws = self.workspace.as_deref();

        // Confined listing when allowed_roots is set
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
                        "directory not found: {}",
                        path_str_owned
                    )));
                };
                if !meta.is_dir() {
                    return Ok(ToolResult::error(format!(
                        "not a directory: {}",
                        path_str_owned
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
                        &format!("error reading directory: {}", e),
                        ws_ref,
                    ))),
                }
            })
            .await?;
        }

        // Unrestricted mode
        if let Err(err) = check_path_allowed(&expanded, self.allowed_roots.as_ref()) {
            return Ok(ToolResult::error(sanitize_err(&err.to_string(), ws)));
        }

        match tokio::fs::metadata(&expanded).await {
            Err(_) => {
                return Ok(ToolResult::error(format!(
                    "directory not found: {}",
                    path_str
                )));
            }
            Ok(meta) if !meta.is_dir() => {
                return Ok(ToolResult::error(format!("not a directory: {}", path_str)));
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
                &format!("error reading directory: {}", e),
                ws,
            ))),
        }
    }
}

#[cfg(test)]
mod tests;
