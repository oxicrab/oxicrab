use crate::agent::subagent::SubagentManager;
use crate::agent::tools::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct SubagentControlTool {
    manager: Arc<SubagentManager>,
}

impl SubagentControlTool {
    pub fn new(manager: Arc<SubagentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for SubagentControlTool {
    fn name(&self) -> &str {
        "subagent_control"
    }

    fn description(&self) -> &str {
        "List or cancel running subagents. Use this to track background tasks or stop one by id."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "cancel"],
                    "description": "Action to perform"
                },
                "task_id": {
                    "type": "string",
                    "description": "Subagent task id (required for cancel)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?
            .to_lowercase();

        match action.as_str() {
            "list" => {
                let tasks = self.manager.list_running().await;
                if tasks.is_empty() {
                    return Ok(ToolResult::new("No running subagents.".to_string()));
                }
                let lines: Vec<String> = tasks
                    .iter()
                    .map(|t| {
                        let id = t.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                        let done = t.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
                        let cancelled = t
                            .get("cancelled")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let status = if cancelled {
                            "cancelled"
                        } else if done {
                            "done"
                        } else {
                            "running"
                        };
                        format!("- [{}] {}", id, status)
                    })
                    .collect();
                Ok(ToolResult::new(format!(
                    "Running subagents:\n{}",
                    lines.join("\n")
                )))
            }
            "cancel" => {
                let task_id = params["task_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'task_id' parameter for cancel"))?;

                let cancelled = self.manager.cancel(task_id).await;
                if cancelled {
                    Ok(ToolResult::new(format!(
                        "Subagent {} cancelled successfully.",
                        task_id
                    )))
                } else {
                    Ok(ToolResult::error(format!(
                        "Error: subagent {} not found or already finished.",
                        task_id
                    )))
                }
            }
            _ => Ok(ToolResult::error(
                "Error: unsupported action. Use 'list' or 'cancel'.".to_string(),
            )),
        }
    }
}
