use crate::agent::subagent::SubagentManager;
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
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

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: false,
            subagent_access: SubagentAccess::Denied,
            actions: vec![],
        }
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?
            .to_lowercase();

        match action.as_str() {
            "list" => {
                let tasks = self.manager.list_running().await;
                let (running, max, available) = self.manager.capacity().await;
                let capacity_line = format!(
                    "Capacity: {}/{} running, {} slots available",
                    running, max, available
                );

                if tasks.is_empty() {
                    return Ok(ToolResult::new(format!(
                        "No running subagents.\n{}",
                        capacity_line
                    )));
                }
                let lines: Vec<String> = tasks
                    .iter()
                    .map(|t| {
                        let id = t.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                        let done = t
                            .get("done")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        let cancelled = t
                            .get("cancelled")
                            .and_then(serde_json::Value::as_bool)
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
                    "Running subagents:\n{}\n{}",
                    lines.join("\n"),
                    capacity_line
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
                        "subagent {} not found or already finished",
                        task_id
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
mod tests {
    use super::*;
    use crate::agent::subagent::SubagentManager;
    use crate::agent::tools::Tool;
    use crate::agent::tools::base::SubagentAccess;
    use crate::bus::MessageBus;
    use crate::config::{PromptGuardConfig, SandboxConfig};
    use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse};
    use std::path::PathBuf;

    struct DummyProvider;

    #[async_trait::async_trait]
    impl LLMProvider for DummyProvider {
        async fn chat(&self, _req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
            unreachable!()
        }
        fn default_model(&self) -> &'static str {
            "dummy"
        }
    }

    fn make_tool() -> SubagentControlTool {
        let bus = Arc::new(tokio::sync::Mutex::new(MessageBus::new(10, 1.0, 16, 16)));
        let config = crate::agent::subagent::SubagentConfig {
            provider: Arc::new(DummyProvider),
            workspace: PathBuf::from("/tmp"),
            model: None,
            brave_api_key: None,
            exec_timeout: 30,
            restrict_to_workspace: false,
            allowed_commands: vec![],
            max_tokens: 1024,
            tool_temperature: 0.0,
            max_concurrent: 2,
            exfil_blocked_tools: vec![],
            cost_guard: None,
            prompt_guard_config: PromptGuardConfig::default(),
            sandbox_config: SandboxConfig::default(),
        };
        let manager = Arc::new(SubagentManager::new(config, bus));
        SubagentControlTool::new(manager)
    }

    #[test]
    fn test_subagent_control_capabilities() {
        let tool = make_tool();
        let caps = tool.capabilities();
        assert!(caps.built_in);
        assert!(!caps.network_outbound);
        assert_eq!(caps.subagent_access, SubagentAccess::Denied);
        assert!(caps.actions.is_empty());
    }
}
