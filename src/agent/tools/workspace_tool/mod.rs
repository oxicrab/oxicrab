use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use crate::agent::workspace::{FileCategory, WorkspaceManager};
use crate::config::schema::WorkspaceTtlConfig;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use walkdir::WalkDir;

#[cfg(test)]
mod tests;

pub struct WorkspaceTool {
    manager: Arc<WorkspaceManager>,
    workspace_ttl: WorkspaceTtlConfig,
}

impl WorkspaceTool {
    pub fn new(manager: Arc<WorkspaceManager>, workspace_ttl: WorkspaceTtlConfig) -> Self {
        Self {
            manager,
            workspace_ttl,
        }
    }

    /// Resolve a path parameter to an absolute path.
    ///
    /// Handles absolute paths, ~ expansion, and relative paths from workspace root.
    fn resolve_tool_path(&self, path_str: &str) -> PathBuf {
        let path = PathBuf::from(path_str);
        if path.is_absolute() {
            path
        } else if path_str.starts_with('~') {
            dirs::home_dir()
                .map(|h| h.join(path_str.strip_prefix("~/").unwrap_or(path_str)))
                .unwrap_or(path)
        } else {
            self.manager.workspace_root().join(path_str)
        }
    }

    /// Format a human-readable file size.
    fn format_size(bytes: i64) -> String {
        if bytes < 1024 {
            format!("{} B", bytes)
        } else if bytes < 1024 * 1024 {
            format!("{:.1} KB", bytes as f64 / 1024.0)
        } else {
            format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
        }
    }

    /// Format a list of workspace file entries as a readable table.
    fn format_file_table(entries: &[crate::agent::workspace::WorkspaceFileEntry]) -> String {
        if entries.is_empty() {
            return "No files found.".to_string();
        }

        let mut lines = vec![format!("Found {} file(s):\n", entries.len())];
        lines.push(format!(
            "  {:<40} | {:<10} | {:<8} | {}",
            "path", "category", "size", "created"
        ));
        lines.push(format!("  {}", "-".repeat(80)));

        for entry in entries {
            let date = entry
                .created_at
                .split(' ')
                .next()
                .unwrap_or(&entry.created_at);
            lines.push(format!(
                "  {:<40} | {:<10} | {:<8} | {}",
                entry.path,
                entry.category,
                Self::format_size(entry.size_bytes),
                date
            ));
        }

        lines.join("\n")
    }

    fn action_list(&self, params: &Value) -> Result<String> {
        let category = params["category"]
            .as_str()
            .map(FileCategory::from_str)
            .transpose()
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let date = params["date"].as_str();
        let tags = params["tags"].as_str();

        let entries = self.manager.list_files(category, date, tags)?;
        Ok(Self::format_file_table(&entries))
    }

    fn action_search(&self, params: &Value) -> Result<String> {
        let query = params["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;

        let entries = self.manager.search_files(query)?;
        Ok(Self::format_file_table(&entries))
    }

    fn action_info(&self, params: &Value) -> Result<String> {
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'path' parameter"))?;
        let abs_path = self.resolve_tool_path(path_str);

        // Search manifest by the filename or relative path
        let search_term = abs_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path_str);

        let entries = self.manager.search_files(search_term)?;
        let rel = abs_path
            .strip_prefix(self.manager.workspace_root())
            .ok()
            .and_then(|p| p.to_str());

        let entry = entries
            .iter()
            .find(|e| {
                // Match by relative path or by absolute path ending
                if let Some(r) = rel {
                    e.path == r
                } else {
                    e.path.ends_with(search_term)
                }
            })
            .or_else(|| entries.first());

        let Some(entry) = entry else {
            return Ok(format!("No file found matching '{}'", path_str));
        };

        let mut info = String::new();
        let _ = writeln!(info, "path: {}", entry.path);
        let _ = writeln!(info, "category: {}", entry.category);
        if let Some(ref name) = entry.original_name {
            let _ = writeln!(info, "original_name: {}", name);
        }
        let _ = writeln!(
            info,
            "size: {} ({} bytes)",
            Self::format_size(entry.size_bytes),
            entry.size_bytes
        );
        if let Some(ref tool) = entry.source_tool {
            let _ = writeln!(info, "source_tool: {}", tool);
        }
        if !entry.tags.is_empty() {
            let _ = writeln!(info, "tags: {}", entry.tags);
        }
        let _ = writeln!(info, "created_at: {}", entry.created_at);
        if let Some(ref accessed) = entry.accessed_at {
            let _ = writeln!(info, "accessed_at: {}", accessed);
        }
        if let Some(ref session) = entry.session_key {
            let _ = writeln!(info, "session_key: {}", session);
        }

        Ok(info)
    }

    fn action_tree(&self) -> String {
        let root = self.manager.workspace_root();
        let mut tree = String::from("workspace/\n");

        for cat in FileCategory::ALL {
            let cat_dir = root.join(cat.as_str());
            if !cat_dir.is_dir() {
                continue;
            }

            let _ = writeln!(tree, "  {}/", cat.as_str());

            // Walk subdirectories (typically date dirs)
            let mut subdirs: Vec<(String, usize)> = Vec::new();
            for entry in WalkDir::new(&cat_dir)
                .min_depth(1)
                .max_depth(1)
                .sort_by_file_name()
                .into_iter()
                .filter_map(Result::ok)
            {
                if entry.file_type().is_dir() {
                    let name = entry.file_name().to_str().unwrap_or("?").to_string();
                    // Count files in this subdirectory
                    let file_count = WalkDir::new(entry.path())
                        .min_depth(1)
                        .max_depth(3)
                        .into_iter()
                        .filter_map(Result::ok)
                        .filter(|e| e.file_type().is_file())
                        .count();
                    subdirs.push((name, file_count));
                } else if entry.file_type().is_file() {
                    // Files directly in category dir (no date subdir)
                    subdirs.push((
                        entry.file_name().to_str().unwrap_or("?").to_string(),
                        0, // marker for direct file
                    ));
                }
            }

            if subdirs.is_empty() {
                tree.push_str("    (empty)\n");
            } else {
                for (name, count) in &subdirs {
                    if *count == 0 {
                        // Direct file in category dir
                        let _ = writeln!(tree, "    {}", name);
                    } else {
                        let label = if *count == 1 { "file" } else { "files" };
                        let _ = writeln!(tree, "    {}/ ({} {})", name, count, label);
                    }
                }
            }
        }

        if tree == "workspace/\n" {
            tree.push_str("  (no category directories)\n");
        }

        tree
    }

    fn action_move(&self, params: &Value) -> Result<String> {
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'path' parameter"))?;
        let category_str = params["category"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'category' parameter"))?;

        let abs_path = self.resolve_tool_path(path_str);
        let category =
            FileCategory::from_str(category_str).map_err(|e| anyhow::anyhow!("{}", e))?;

        let new_path = self.manager.move_file(&abs_path, category)?;
        let display = new_path
            .strip_prefix(self.manager.workspace_root())
            .unwrap_or(&new_path);
        Ok(format!("Moved to {}", display.display()))
    }

    fn action_delete(&self, params: &Value) -> Result<String> {
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'path' parameter"))?;

        let abs_path = self.resolve_tool_path(path_str);
        self.manager.remove_file(&abs_path)?;
        Ok(format!("Deleted {}", path_str))
    }

    fn action_tag(&self, params: &Value) -> Result<String> {
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'path' parameter"))?;
        let tags = params["tags"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'tags' parameter"))?;

        let abs_path = self.resolve_tool_path(path_str);
        self.manager.tag_file(&abs_path, tags)?;
        Ok(format!("Tagged '{}' with: {}", path_str, tags))
    }

    fn action_cleanup(&self) -> Result<String> {
        let ttl_map = self.workspace_ttl.to_map();
        let removed = self.manager.cleanup_expired(&ttl_map)?;
        let (stale, discovered) = self.manager.sync_manifest()?;

        let mut report = format!("Cleanup complete: {} expired file(s) removed.", removed);
        if stale > 0 || discovered > 0 {
            let _ = write!(
                report,
                "\nManifest sync: {} stale entries removed, {} new files discovered.",
                stale, discovered
            );
        }
        Ok(report)
    }
}

#[async_trait]
impl Tool for WorkspaceTool {
    fn name(&self) -> &'static str {
        "workspace"
    }

    fn description(&self) -> &'static str {
        "Manage workspace files: list, search, organize, and clean up files in the workspace."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(0, 1, 0)
    }

    fn capabilities(&self) -> ToolCapabilities {
        use crate::agent::tools::base::ActionDescriptor;

        ToolCapabilities {
            built_in: true,
            network_outbound: false,
            subagent_access: SubagentAccess::ReadOnly,
            actions: vec![
                ActionDescriptor {
                    name: "list",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "search",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "info",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "tree",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "move",
                    read_only: false,
                },
                ActionDescriptor {
                    name: "delete",
                    read_only: false,
                },
                ActionDescriptor {
                    name: "tag",
                    read_only: false,
                },
                ActionDescriptor {
                    name: "cleanup",
                    read_only: false,
                },
            ],
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "search", "info", "tree", "move", "delete", "tag", "cleanup"],
                    "description": "The workspace management action to perform"
                },
                "category": {
                    "type": "string",
                    "enum": ["code", "documents", "data", "images", "downloads", "temp"],
                    "description": "File category filter (for list, move)"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (for search)"
                },
                "path": {
                    "type": "string",
                    "description": "File path (for info, move, delete, tag)"
                },
                "date": {
                    "type": "string",
                    "description": "Date filter YYYY-MM-DD (for list)"
                },
                "tags": {
                    "type": "string",
                    "description": "Comma-separated tags (for tag action, or filter for list)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'action' parameter"))?;

        let result = match action {
            "list" => self.action_list(&params),
            "search" => self.action_search(&params),
            "info" => self.action_info(&params),
            "tree" => Ok(self.action_tree()),
            "move" => self.action_move(&params),
            "delete" => self.action_delete(&params),
            "tag" => self.action_tag(&params),
            "cleanup" => self.action_cleanup(),
            _ => return Ok(ToolResult::error(format!("unknown action: '{}'", action))),
        };

        Ok(ToolResult::from_result(result, "workspace"))
    }
}
