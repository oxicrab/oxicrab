pub mod cache;
pub mod client;
#[cfg(test)]
mod tests;

use cache::ObsidianCache;
use client::ObsidianApiClient;

use crate::agent::tools::base::ExecutionContext;
use crate::agent::tools::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
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
    ) -> Result<(Self, Arc<ObsidianCache>)> {
        let client = Arc::new(ObsidianApiClient::new(api_url, api_key, timeout));
        let cache = Arc::new(ObsidianCache::new(client, vault_name)?);
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
            let _ = writeln!(out, "  - {}", tag);
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
        "Read, write, search, and list notes in an Obsidian vault. Actions: read (read a note), write (create/overwrite a note, auto-generates YAML frontmatter for new notes), append (append to a note), search (full-text search), list (list notes, optionally in a folder)."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "The action to perform: read, write, append, search, or list",
                    "enum": ["read", "write", "append", "search", "list"]
                },
                "path": {
                    "type": "string",
                    "description": "Path to the note (e.g. 'Daily/2025-01-15.md'). Required for read, write, append."
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
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = params["action"].as_str().unwrap_or("");

        match action {
            "read" => {
                let path = match params["path"].as_str() {
                    Some(p) if !p.is_empty() => p,
                    _ => return Ok(ToolResult::error("'path' is required for read")),
                };
                match self.cache.read_cached(path) {
                    Some(content) => Ok(ToolResult::new(content)),
                    None => Ok(ToolResult::error(format!(
                        "Note '{}' not found in cache. It may not exist or hasn't been synced yet.",
                        path
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
                        format!("{}\n{}", fm, content)
                    } else {
                        content.to_string()
                    };
                match self.cache.write_file(path, &content).await {
                    Ok(msg) => Ok(ToolResult::new(msg)),
                    Err(e) => Ok(ToolResult::error(format!("Write failed: {}", e))),
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
                    Err(e) => Ok(ToolResult::error(format!("Append failed: {}", e))),
                }
            }
            "search" => {
                let query = match params["query"].as_str() {
                    Some(q) if !q.is_empty() => q,
                    _ => return Ok(ToolResult::error("'query' is required for search")),
                };
                let results = self.cache.search_cached(query).await;
                if results.is_empty() {
                    Ok(ToolResult::new(format!(
                        "No results found for '{}'.",
                        query
                    )))
                } else {
                    let lines: Vec<String> = results
                        .iter()
                        .map(|(path, line)| format!("{}:  {}", path, line.trim()))
                        .collect();
                    Ok(ToolResult::new(format!(
                        "Found {} matches:\n{}",
                        results.len(),
                        lines.join("\n")
                    )))
                }
            }
            "list" => {
                let folder = params["folder"].as_str();
                let files = self.cache.list_cached(folder).await;
                if files.is_empty() {
                    let msg = if let Some(f) = folder {
                        format!("No notes found in folder '{}'.", f)
                    } else {
                        "No notes in cache. The vault may not have synced yet.".to_string()
                    };
                    Ok(ToolResult::new(msg))
                } else {
                    Ok(ToolResult::new(format!(
                        "{} notes:\n{}",
                        files.len(),
                        files.join("\n")
                    )))
                }
            }
            _ => Ok(ToolResult::error(format!(
                "Unknown action '{}'. Use: read, write, append, search, or list.",
                action
            ))),
        }
    }
}
