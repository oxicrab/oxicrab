use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory};
use crate::agent::tools::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

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
    /// Shared set of tool names activated during the current agent run.
    /// The agent loop reads this to dynamically expand tool definitions.
    activated: Arc<Mutex<HashSet<String>>>,
}

impl ToolSearchTool {
    pub fn new(index: Vec<ToolIndexEntry>, activated: Arc<Mutex<HashSet<String>>>) -> Self {
        Self { index, activated }
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
            category: ToolCategory::System,
            subagent_access: SubagentAccess::Denied,
            ..Default::default()
        }
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
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
        {
            let mut activated = self.activated.lock().await;
            for m in &matches {
                if m.deferred {
                    activated.insert(m.name.clone());
                }
            }
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
        let activated = Arc::new(Mutex::new(HashSet::new()));
        let tool = ToolSearchTool::new(make_index(), activated.clone());
        let result = tool
            .execute(
                serde_json::json!({"query": "web"}),
                &ExecutionContext::default(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("web_scrape"));
        assert!(!result.content.contains("read_file"));
        // Deferred tool should be activated
        assert!(activated.lock().await.contains("web_scrape"));
    }

    #[tokio::test]
    async fn test_search_no_results() {
        let activated = Arc::new(Mutex::new(HashSet::new()));
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
        let activated = Arc::new(Mutex::new(HashSet::new()));
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
        let activated = Arc::new(Mutex::new(HashSet::new()));
        let tool = ToolSearchTool::new(make_index(), activated.clone());
        let _ = tool
            .execute(
                serde_json::json!({"query": "read"}),
                &ExecutionContext::default(),
            )
            .await
            .unwrap();
        // read_file is not deferred, so it shouldn't be in activated
        assert!(!activated.lock().await.contains("read_file"));
    }
}
