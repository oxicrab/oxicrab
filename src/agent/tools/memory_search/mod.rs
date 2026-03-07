use crate::actions;
use crate::agent::memory::MemoryStore;
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct MemorySearchTool {
    memory: Arc<MemoryStore>,
}

impl MemorySearchTool {
    pub fn new(memory: Arc<MemoryStore>) -> Self {
        Self { memory }
    }
}

impl MemorySearchTool {
    fn action_explain_last(&self) -> Result<ToolResult> {
        let details = self.memory.db().get_last_search_details()?;
        let Some(d) = details else {
            return Ok(ToolResult::new(
                "No memory searches recorded yet.".to_string(),
            ));
        };

        let score_str = d
            .top_score
            .map_or_else(|| "n/a".to_string(), |s| format!("{s:.3}"));
        let sources = if d.source_keys.is_empty() {
            "(none)".to_string()
        } else {
            d.source_keys.join(", ")
        };

        Ok(ToolResult::new(format!(
            "Last memory search:\n\
             - Query: \"{}\"\n\
             - Method: {}\n\
             - Results: {}\n\
             - Top score: {}\n\
             - Sources: {}\n\
             - Time: {}",
            d.query, d.search_type, d.result_count, score_str, sources, d.timestamp
        )))
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &'static str {
        "memory_search"
    }

    fn description(&self) -> &'static str {
        "Search long-term memory and daily notes. Actions: 'search' (default) finds relevant memories; 'explain_last' shows provenance details of the most recent search (query, method, scores, sources)."
    }

    fn cacheable(&self) -> bool {
        true
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: actions![search: ro, explain_last: ro],
            ..Default::default()
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "explain_last"],
                    "description": "Action to perform. 'search' (default) retrieves memories; 'explain_last' returns provenance of the most recent search."
                },
                "query": {
                    "type": "string",
                    "description": "Search query to find relevant memories. Required when action is 'search' (the default)."
                }
            }
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = params["action"].as_str().unwrap_or("search");

        if action == "explain_last" {
            return self.action_explain_last();
        }

        let query = match params["query"].as_str() {
            Some(q) if !q.trim().is_empty() => q,
            _ => {
                return Ok(ToolResult::error(
                    "missing or empty 'query' parameter".to_string(),
                ));
            }
        };

        // Use hybrid search when embeddings are available
        #[cfg(feature = "embeddings")]
        if self.memory.has_embeddings() {
            match self.memory.hybrid_search(query, 8, None) {
                Ok(hits) if !hits.is_empty() => {
                    let chunks: Vec<String> = hits
                        .iter()
                        .map(|h| format!("**{}**: {}", h.source_key, h.content))
                        .collect();
                    return Ok(ToolResult::new(chunks.join("\n\n---\n\n")));
                }
                Ok(_) => {} // empty, fall through to keyword search
                Err(e) => {
                    tracing::warn!("hybrid search failed, falling back to keyword: {}", e);
                }
            }
        }

        // Fallback to keyword-only search
        match self.memory.get_memory_context(Some(query)) {
            Ok(context) => {
                if context.trim().is_empty() {
                    Ok(ToolResult::new(
                        "No relevant memories found for this query.".to_string(),
                    ))
                } else {
                    Ok(ToolResult::new(context))
                }
            }
            Err(e) => Ok(ToolResult::error(format!("memory search error: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests;
