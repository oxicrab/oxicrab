pub mod cache;
pub mod client;
#[cfg(test)]
mod tests;

use cache::ObsidianCache;
use client::ObsidianApiClient;

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
    /// Create a new ObsidianTool and return the shared cache for the sync service.
    pub async fn new(
        api_url: &str,
        api_key: &str,
        vault_name: &str,
        timeout: u64,
    ) -> Result<(Self, Arc<ObsidianCache>)> {
        let client = Arc::new(ObsidianApiClient::new(api_url, api_key, timeout));
        let cache = Arc::new(ObsidianCache::new(client, vault_name).await?);
        Ok((
            Self {
                cache: cache.clone(),
            },
            cache,
        ))
    }
}

#[async_trait]
impl Tool for ObsidianTool {
    fn name(&self) -> &str {
        "obsidian"
    }

    fn description(&self) -> &str {
        "Read, write, search, and list notes in an Obsidian vault. Actions: read (read a note), write (create/overwrite a note), append (append to a note), search (full-text search), list (list notes, optionally in a folder)."
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
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let action = params["action"].as_str().unwrap_or("");

        match action {
            "read" => {
                let path = match params["path"].as_str() {
                    Some(p) if !p.is_empty() => p,
                    _ => return Ok(ToolResult::error("'path' is required for read".into())),
                };
                match self.cache.read_cached(path).await {
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
                    _ => return Ok(ToolResult::error("'path' is required for write".into())),
                };
                let content = match params["content"].as_str() {
                    Some(c) => c,
                    None => return Ok(ToolResult::error("'content' is required for write".into())),
                };
                match self.cache.write_file(path, content).await {
                    Ok(msg) => Ok(ToolResult::new(msg)),
                    Err(e) => Ok(ToolResult::error(format!("Write failed: {}", e))),
                }
            }
            "append" => {
                let path = match params["path"].as_str() {
                    Some(p) if !p.is_empty() => p,
                    _ => return Ok(ToolResult::error("'path' is required for append".into())),
                };
                let content = match params["content"].as_str() {
                    Some(c) => c,
                    None => {
                        return Ok(ToolResult::error("'content' is required for append".into()))
                    }
                };
                match self.cache.append_file(path, content).await {
                    Ok(msg) => Ok(ToolResult::new(msg)),
                    Err(e) => Ok(ToolResult::error(format!("Append failed: {}", e))),
                }
            }
            "search" => {
                let query = match params["query"].as_str() {
                    Some(q) if !q.is_empty() => q,
                    _ => return Ok(ToolResult::error("'query' is required for search".into())),
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
