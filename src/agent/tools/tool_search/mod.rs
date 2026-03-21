use crate::actions;
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory};
use crate::agent::tools::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

const REQUEST_ID_META_KEY: &str = "request_id";

/// A tool name + description pair for the search index.
#[derive(Clone)]
pub struct ToolIndexEntry {
    pub name: String,
    pub description: String,
    pub deferred: bool,
}

/// Meta-tool that lets the LLM discover available tools by keyword search.
///
/// When MCP servers register many tools, sending all schemas in every request
/// wastes tokens. Deferred tools have their schemas omitted from the LLM
/// request until the LLM discovers them via `tool_search`. Once discovered,
/// their schemas are included in subsequent iterations of the same agent run.
pub struct ToolSearchTool {
    index: Vec<ToolIndexEntry>,
    /// Request-scoped set of tool names activated during the current agent run.
    /// The agent loop reads this to dynamically expand tool definitions.
    activated: ActivatedTools,
}

impl ToolSearchTool {
    pub fn new(index: Vec<ToolIndexEntry>, activated: ActivatedTools) -> Self {
        Self { index, activated }
    }
}

#[derive(Clone, Default)]
pub struct ActivatedTools {
    inner: Arc<Mutex<HashMap<String, HashSet<String>>>>,
}

impl ActivatedTools {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn clear(&self, request_id: &str) {
        self.inner.lock().await.remove(request_id);
    }

    pub async fn snapshot(&self, request_id: &str) -> HashSet<String> {
        self.inner
            .lock()
            .await
            .get(request_id)
            .cloned()
            .unwrap_or_default()
    }

    async fn activate(&self, request_id: &str, names: impl IntoIterator<Item = String>) {
        let mut guard = self.inner.lock().await;
        let entry = guard.entry(request_id.to_string()).or_default();
        entry.extend(names);
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &'static str {
        "tool_search"
    }

    fn description(&self) -> &'static str {
        "Search for available tools by keyword. Returns matching tool names and descriptions. \
         Use this when you need a tool that isn't in your current tool list."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keyword(s) to search for in tool names and descriptions"
                }
            },
            "required": ["query"]
        })
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            actions: actions![search: ro],
            category: ToolCategory::System,
            subagent_access: SubagentAccess::Denied,
            ..Default::default()
        }
    }

    async fn execute(&self, params: Value, ctx: &ExecutionContext) -> Result<ToolResult> {
        let query = params["query"].as_str().unwrap_or("").to_lowercase();

        if query.is_empty() {
            // List all tools (names only)
            let mut lines: Vec<String> = self
                .index
                .iter()
                .map(|e| format!("- {}: {}", e.name, e.description))
                .collect();
            lines.sort();
            return Ok(ToolResult::new(format!(
                "Available tools ({}):\n{}",
                self.index.len(),
                lines.join("\n")
            )));
        }

        let keywords: Vec<&str> = query.split_whitespace().collect();
        let mut matches: Vec<&ToolIndexEntry> = self
            .index
            .iter()
            .filter(|e| {
                let haystack = format!("{} {}", e.name, e.description).to_lowercase();
                keywords.iter().any(|kw| haystack.contains(kw))
            })
            .collect();

        matches.sort_by(|a, b| a.name.cmp(&b.name));

        if matches.is_empty() {
            return Ok(ToolResult::new(format!(
                "No tools found matching '{query}'. Try different keywords or an empty query to list all."
            )));
        }

        // Activate any deferred tools that matched
        if let Some(request_id) = ctx
            .metadata
            .get(REQUEST_ID_META_KEY)
            .and_then(Value::as_str)
        {
            self.activated
                .activate(
                    request_id,
                    matches
                        .iter()
                        .filter(|m| m.deferred)
                        .map(|m| m.name.clone()),
                )
                .await;
        }

        let lines: Vec<String> = matches
            .iter()
            .map(|e| format!("- {}: {}", e.name, e.description))
            .collect();

        Ok(ToolResult::new(format!(
            "Found {} tool(s) matching '{query}':\n{}\n\nThese tools are now available for use.",
            matches.len(),
            lines.join("\n")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_index() -> Vec<ToolIndexEntry> {
        vec![
            ToolIndexEntry {
                name: "read_file".into(),
                description: "Read a file from disk".into(),
                deferred: false,
            },
            ToolIndexEntry {
                name: "web_scrape".into(),
                description: "Scrape a web page".into(),
                deferred: true,
            },
            ToolIndexEntry {
                name: "git_log".into(),
                description: "Show git commit history".into(),
                deferred: true,
            },
        ]
    }

    #[tokio::test]
    async fn test_search_by_keyword() {
        let activated = ActivatedTools::new();
        let tool = ToolSearchTool::new(make_index(), activated.clone());
        let result = tool
            .execute(
                serde_json::json!({"query": "web"}),
                &ExecutionContext {
                    metadata: HashMap::from([(
                        REQUEST_ID_META_KEY.to_string(),
                        Value::String("req-1".to_string()),
                    )]),
                    ..ExecutionContext::default()
                },
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("web_scrape"));
        assert!(!result.content.contains("read_file"));
        // Deferred tool should be activated
        assert!(activated.snapshot("req-1").await.contains("web_scrape"));
    }

    #[tokio::test]
    async fn test_search_no_results() {
        let activated = ActivatedTools::new();
        let tool = ToolSearchTool::new(make_index(), activated.clone());
        let result = tool
            .execute(
                serde_json::json!({"query": "database"}),
                &ExecutionContext::default(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No tools found"));
    }

    #[tokio::test]
    async fn test_empty_query_lists_all() {
        let activated = ActivatedTools::new();
        let tool = ToolSearchTool::new(make_index(), activated.clone());
        let result = tool
            .execute(
                serde_json::json!({"query": ""}),
                &ExecutionContext::default(),
            )
            .await
            .unwrap();
        assert!(result.content.contains("Available tools (3)"));
        assert!(result.content.contains("read_file"));
        assert!(result.content.contains("web_scrape"));
    }

    #[tokio::test]
    async fn test_non_deferred_not_activated() {
        let activated = ActivatedTools::new();
        let tool = ToolSearchTool::new(make_index(), activated.clone());
        let _ = tool
            .execute(
                serde_json::json!({"query": "read"}),
                &ExecutionContext {
                    metadata: HashMap::from([(
                        REQUEST_ID_META_KEY.to_string(),
                        Value::String("req-2".to_string()),
                    )]),
                    ..ExecutionContext::default()
                },
            )
            .await
            .unwrap();
        // read_file is not deferred, so it shouldn't be in activated
        assert!(!activated.snapshot("req-2").await.contains("read_file"));
    }
}
