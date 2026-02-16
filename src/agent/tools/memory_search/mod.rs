use crate::agent::memory::MemoryStore;
use crate::agent::tools::base::ExecutionContext;
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

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &'static str {
        "memory_search"
    }

    fn description(&self) -> &'static str {
        "Search long-term memory and daily notes. Use to recall user preferences, past conversations, important facts."
    }

    fn cacheable(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query to find relevant memories"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let query = match params["query"].as_str() {
            Some(q) if !q.trim().is_empty() => q,
            _ => {
                return Ok(ToolResult::error(
                    "Missing or empty 'query' parameter".to_string(),
                ))
            }
        };

        // Use hybrid search when embeddings are available
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
            Err(e) => Ok(ToolResult::error(format!("Memory search error: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests;
