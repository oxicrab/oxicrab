use crate::agent::memory::MemoryStore;
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
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
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

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let query = match params["query"].as_str() {
            Some(q) if !q.trim().is_empty() => q,
            _ => {
                return Ok(ToolResult::error(
                    "Missing or empty 'query' parameter".to_string(),
                ))
            }
        };

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
mod tests {
    use super::*;

    fn create_tool() -> MemorySearchTool {
        let tmp = tempfile::TempDir::new().unwrap();
        let memory = Arc::new(MemoryStore::new(tmp.path()).unwrap());
        MemorySearchTool::new(memory)
    }

    #[tokio::test]
    async fn test_memory_search_missing_query() {
        let tool = create_tool();
        let result = tool.execute(serde_json::json!({})).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("query"));
    }

    #[tokio::test]
    async fn test_memory_search_empty_query() {
        let tool = create_tool();
        let result = tool
            .execute(serde_json::json!({"query": "  "}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("query"));
    }

    #[tokio::test]
    async fn test_memory_search_empty_result() {
        let tool = create_tool();
        let result = tool
            .execute(serde_json::json!({"query": "nonexistent topic xyz"}))
            .await
            .unwrap();
        // Should not error - either returns results or a friendly message
        assert!(!result.is_error);
    }

    #[test]
    fn test_tool_metadata() {
        let tool = create_tool();
        assert_eq!(tool.name(), "memory_search");
        assert!(tool.cacheable());
        assert!(tool.description().contains("memory"));
    }
}
