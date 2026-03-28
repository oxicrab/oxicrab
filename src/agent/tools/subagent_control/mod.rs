use crate::actions;
use crate::agent::subagent::SubagentManager;
use crate::agent::tools::base::{ExecutionContext, ToolCapabilities, ToolCategory};
use crate::agent::tools::{Tool, ToolResult};
use crate::require_param;
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
    fn name(&self) -> &'static str {
        "subagent_control"
    }

    fn description(&self) -> &'static str {
        "List or cancel running subagents. Use this to track background tasks or stop one by id."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "cancel"],
                    "description": "Action to perform. 'list' shows running subagents. \
                     'cancel' stops a subagent by task_id."
                },
                "task_id": {
                    "type": "string",
                    "description": "Subagent task id (required for cancel)"
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            category: ToolCategory::System,
            actions: actions![list: ro, cancel],
            ..Default::default()
        }
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = require_param!(params, "action").to_lowercase();

        match action.as_str() {
            "list" => {
                let tasks = self.manager.list_running().await;
                let (running, max, available) = self.manager.capacity().await;
                let capacity_line =
                    format!("Capacity: {running}/{max} running, {available} slots available");

                if tasks.is_empty() {
                    return Ok(ToolResult::new(format!(
                        "No running subagents.\n{capacity_line}"
                    )));
                }
                let mut buttons = Vec::new();
                let lines: Vec<String> = tasks
                    .iter()
                    .map(|t| {
                        let id = t.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                        let done = t
                            .get("done")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or_default();
                        let cancelled = t
                            .get("cancelled")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or_default();
                        let status = if cancelled {
                            "cancelled"
                        } else if done {
                            "done"
                        } else {
                            // Only offer cancel buttons for running subagents
                            if buttons.len() < 5 {
                                buttons.push(serde_json::json!({
                                    "id": format!("cancel-{id}"),
                                    "label": format!("Cancel: {}", if id.len() > 20 { &id[..20] } else { id }),
                                    "style": "danger",
                                    "context": serde_json::json!({
                                        "tool": "subagent_control",
                                        "params": {
                                            "action": "cancel",
                                            "task_id": id
                                        }
                                    }).to_string()
                                }));
                            }
                            "running"
                        };
                        format!("- [{id}] {status}")
                    })
                    .collect();
                Ok(ToolResult::new(format!(
                    "Running subagents:\n{}\n{}",
                    lines.join("\n"),
                    capacity_line
                ))
                .with_buttons(buttons))
            }
            "cancel" => {
                let task_id = require_param!(params, "task_id");

                let cancelled = self.manager.cancel(task_id).await;
                if cancelled {
                    Ok(ToolResult::new(format!(
                        "Subagent {task_id} cancelled successfully."
                    )))
                } else {
                    Ok(ToolResult::error(format!(
                        "subagent {task_id} not found or already finished"
                    )))
                }
            }
            _ => Ok(ToolResult::error(
                "unsupported action. Use 'list' or 'cancel'".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests;
