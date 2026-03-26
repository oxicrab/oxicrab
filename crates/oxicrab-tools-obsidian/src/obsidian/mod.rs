pub mod cache;
pub mod client;
#[cfg(test)]
mod tests;

use cache::ObsidianCache;
use client::ObsidianApiClient;

use anyhow::Result;
use async_trait::async_trait;
use oxicrab_core::actions;
use oxicrab_core::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory};
use oxicrab_core::tools::base::{Tool, ToolResult};
use oxicrab_memory::memory_db::MemoryDB;
use serde_json::Value;
use std::sync::Arc;

pub use cache::ObsidianSyncService;

pub struct ObsidianTool {
    pub(crate) cache: Arc<ObsidianCache>,
}

impl ObsidianTool {
    /// Create a new `ObsidianTool` and return the shared cache for the sync service.
    pub fn new(
        api_url: &str,
        api_key: &str,
        vault_name: &str,
        timeout: u64,
        db: Option<Arc<MemoryDB>>,
    ) -> Result<(Self, Arc<ObsidianCache>)> {
        let client = Arc::new(ObsidianApiClient::new(api_url, api_key, timeout));
        let cache = Arc::new(ObsidianCache::new(client, vault_name, db)?);
        Ok((
            Self {
                cache: cache.clone(),
            },
            cache,
        ))
    }
}

fn generate_frontmatter(params: Option<&Value>) -> String {
    let now = chrono::Local::now().format("%Y-%m-%d, %H:%M:%S");
    let get = |field: &str| -> String {
        params
            .and_then(|p| p.get(field))
            .and_then(|v| v.as_str())
            .unwrap_or(match field {
                "type" => "note",
                _ => "",
            })
            .to_string()
    };

    // Format tags as a YAML list (Obsidian expects `tags:\n  - tag1\n  - tag2`)
    let tags_yaml = format_tags_yaml(params);

    format!(
        "---\ncreate-date: {}\ntype: {}\n{}link: {}\nstatus: {}\norder: {}\nparent: {}\n---\n",
        now,
        get("type"),
        tags_yaml,
        get("link"),
        get("status"),
        get("order"),
        get("parent")
    )
}

/// Parse tags from frontmatter params and format as a YAML list.
/// Accepts either a JSON array `["tag1", "tag2"]` or a comma/space-separated string.
fn format_tags_yaml(params: Option<&Value>) -> String {
    let tags_val = params.and_then(|p| p.get("tags"));
    let tags: Vec<String> = match tags_val {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) if !s.is_empty() => s
            .split([',', ' '])
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        _ => vec![],
    };
    if tags.is_empty() {
        "tags:\n".to_string()
    } else {
        use std::fmt::Write as _;
        let mut out = "tags:\n".to_string();
        for tag in &tags {
            let _ = writeln!(out, "  - {tag}");
        }
        out
    }
}

#[async_trait]
impl Tool for ObsidianTool {
    fn name(&self) -> &'static str {
        "obsidian"
    }

    fn description(&self) -> &'static str {
        "Read, write, search, and list notes in an Obsidian vault. Actions: read, write, append, \
         search, list, delete, rename."
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: actions![
                read: ro,
                write,
                append,
                search: ro,
                list: ro,
                delete,
                rename,
            ],
            category: ToolCategory::Productivity,
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "The action to perform. 'search' finds notes by content \
                     (full-text). 'list' browses notes by folder. 'read' reads a specific note \
                     by path. 'write' creates or overwrites. 'append' adds to end of a note. \
                     'delete' removes a note. 'rename' moves/renames a note (requires path and new_path).",
                    "enum": ["read", "write", "append", "search", "list", "delete", "rename"]
                },
                "path": {
                    "type": "string",
                    "description": "Path to the note (e.g. 'Daily/2025-01-15.md'). Required for read, write, append, delete, rename."
                },
                "new_path": {
                    "type": "string",
                    "description": "New path for the note (for rename action)"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write or append. Required for write and append."
                },
                "query": {
                    "type": "string",
                    "description": "Search query for full-text search. Required for search."
                },
                "folder": {
                    "type": "string",
                    "description": "Optional folder prefix to filter when listing notes."
                },
                "frontmatter": {
                    "type": "object",
                    "description": "Optional frontmatter fields for new notes. Supported: type, tags, link, status, order, parent. create-date is auto-filled. Only used on write when the note doesn't already exist.",
                    "properties": {
                        "type": { "type": "string" },
                        "tags": { "description": "Tags as an array [\"tag1\", \"tag2\"] or comma/space-separated string" },
                        "link": { "type": "string" },
                        "status": { "type": "string" },
                        "order": { "type": "string" },
                        "parent": { "type": "string" }
                    }
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results to return (for list, default 100; for search, default 50)",
                    "minimum": 1,
                    "maximum": 500
                },
                "offset": {
                    "type": "integer",
                    "description": "Number of results to skip (for search and list, default 0)",
                    "minimum": 0
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = params["action"].as_str().unwrap_or_default();

        match action {
            "read" => {
                let path = match params["path"].as_str() {
                    Some(p) if !p.is_empty() => p,
                    _ => return Ok(ToolResult::error("'path' is required for read")),
                };
                match self.cache.read_cached(path) {
                    Some(content) => Ok(ToolResult::new(content)),
                    None => Ok(ToolResult::error(format!(
                        "Note '{path}' not found in cache. It may not exist or hasn't been synced yet."
                    ))),
                }
            }
            "write" => {
                let path = match params["path"].as_str() {
                    Some(p) if !p.is_empty() => p,
                    _ => return Ok(ToolResult::error("'path' is required for write")),
                };
                let Some(content) = params["content"].as_str() else {
                    return Ok(ToolResult::error("'content' is required for write"));
                };
                let content =
                    if self.cache.read_cached(path).is_none() && !content.starts_with("---") {
                        let fm = generate_frontmatter(params.get("frontmatter"));
                        format!("{fm}\n{content}")
                    } else {
                        content.to_string()
                    };
                match self.cache.write_file(path, &content).await {
                    Ok(msg) => Ok(ToolResult::new(msg)),
                    Err(e) => Ok(ToolResult::error(format!("write failed: {e}"))),
                }
            }
            "append" => {
                let path = match params["path"].as_str() {
                    Some(p) if !p.is_empty() => p,
                    _ => return Ok(ToolResult::error("'path' is required for append")),
                };
                let Some(content) = params["content"].as_str() else {
                    return Ok(ToolResult::error("'content' is required for append"));
                };
                match self.cache.append_file(path, content).await {
                    Ok(msg) => Ok(ToolResult::new(msg)),
                    Err(e) => Ok(ToolResult::error(format!("append failed: {e}"))),
                }
            }
            "search" => {
                let query = match params["query"].as_str() {
                    Some(q) if !q.is_empty() => q,
                    _ => return Ok(ToolResult::error("'query' is required for search")),
                };
                let offset = params["offset"].as_u64().unwrap_or(0) as usize;
                let limit = params["limit"].as_u64().unwrap_or(50).clamp(1, 500) as usize;
                let all_results = self.cache.search_cached(query).await;
                let total = all_results.len();
                if total == 0 {
                    Ok(ToolResult::new(format!("No results found for '{query}'.")))
                } else {
                    let results: Vec<_> =
                        all_results.into_iter().skip(offset).take(limit).collect();
                    let lines: Vec<String> = results
                        .iter()
                        .map(|(path, line)| format!("{}:  {}", path, line.trim()))
                        .collect();
                    let end = offset + results.len();
                    let range_note = if total > limit || offset > 0 {
                        format!(" (showing {}-{} of {})", offset + 1, end, total)
                    } else {
                        String::new()
                    };
                    Ok(ToolResult::new(format!(
                        "Found {} matches{}:\n{}",
                        total,
                        range_note,
                        lines.join("\n")
                    )))
                }
            }
            "list" => {
                let folder = params["folder"].as_str();
                let offset = params["offset"].as_u64().unwrap_or(0) as usize;
                let limit = params["limit"].as_u64().unwrap_or(100).clamp(1, 500) as usize;
                let all_files = self.cache.list_cached(folder).await;
                let total = all_files.len();
                if total == 0 {
                    let msg = if let Some(f) = folder {
                        format!("No notes found in folder '{f}'.")
                    } else {
                        "No notes in cache. The vault may not have synced yet.".to_string()
                    };
                    Ok(ToolResult::new(msg))
                } else {
                    let files: Vec<_> = all_files.into_iter().skip(offset).take(limit).collect();
                    let end = offset + files.len();
                    let range_note = if total > limit || offset > 0 {
                        format!(" (showing {}-{} of {})", offset + 1, end, total)
                    } else {
                        String::new()
                    };
                    Ok(ToolResult::new(format!(
                        "{} notes{}:\n{}",
                        total,
                        range_note,
                        files.join("\n")
                    )))
                }
            }
            "delete" => {
                let path = match params["path"].as_str() {
                    Some(p) if !p.is_empty() => p,
                    _ => return Ok(ToolResult::error("'path' is required for delete")),
                };
                if path.contains("..") {
                    return Ok(ToolResult::error("path must not contain '..'".to_string()));
                }
                match self.cache.delete_file(path).await {
                    Ok(msg) => Ok(ToolResult::new(msg)),
                    Err(e) => Ok(ToolResult::error(format!("delete failed: {e}"))),
                }
            }
            "rename" => {
                let path = match params["path"].as_str() {
                    Some(p) if !p.is_empty() => p,
                    _ => return Ok(ToolResult::error("'path' is required for rename")),
                };
                let new_path = match params["new_path"].as_str() {
                    Some(p) if !p.is_empty() => p,
                    _ => return Ok(ToolResult::error("'new_path' is required for rename")),
                };
                if path.contains("..") || new_path.contains("..") {
                    return Ok(ToolResult::error("paths must not contain '..'".to_string()));
                }
                match self.cache.rename_file(path, new_path).await {
                    Ok(msg) => Ok(ToolResult::new(msg)),
                    Err(e) => Ok(ToolResult::error(format!("rename failed: {e}"))),
                }
            }
            _ => Ok(ToolResult::error(format!(
                "unknown action '{action}'. Use: read, write, append, search, list, delete, or rename"
            ))),
        }
    }
}
