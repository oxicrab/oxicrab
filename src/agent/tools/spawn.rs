use crate::agent::subagent::SubagentManager;
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct SpawnTool {
    manager: Arc<SubagentManager>,
}

impl SpawnTool {
    pub fn new(manager: Arc<SubagentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for SpawnTool {
    fn name(&self) -> &'static str {
        "spawn"
    }

    fn description(&self) -> &'static str {
        "Spawn a subagent to handle a task in the background. Use this for complex or time-consuming tasks that can run independently. The subagent will complete the task and report back when done."
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: false,
            subagent_access: SubagentAccess::Denied,
            actions: vec![],
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The task for the subagent to complete"
                },
                "label": {
                    "type": "string",
                    "description": "Optional short label for the task (for display)"
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, params: Value, ctx: &ExecutionContext) -> Result<ToolResult> {
        let task = params["task"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'task' parameter"))?
            .to_string();

        let label = params["label"]
            .as_str()
            .map(std::string::ToString::to_string);

        let result = self
            .manager
            .spawn(
                task,
                label,
                ctx.channel.clone(),
                ctx.chat_id.clone(),
                false,
                ctx.context_summary.clone(),
            )
            .await?;
        Ok(ToolResult::new(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::subagent::SubagentManager;
    use crate::agent::tools::Tool;
    use crate::agent::tools::base::SubagentAccess;
    use crate::bus::MessageBus;
    use crate::config::PromptGuardConfig;
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

    fn make_tool() -> SpawnTool {
        let bus = Arc::new(tokio::sync::Mutex::new(MessageBus::new(10, 1.0, 16, 16)));
        let config = crate::agent::subagent::SubagentConfig {
            provider: Arc::new(DummyProvider),
            workspace: PathBuf::from("/tmp"),
            model: None,
            max_tokens: 1024,
            tool_temperature: 0.0,
            max_concurrent: 2,
            cost_guard: None,
            prompt_guard_config: PromptGuardConfig::default(),
            exfil_guard: crate::config::ExfiltrationGuardConfig::default(),
            main_tools: None,
        };
        let manager = Arc::new(SubagentManager::new(config, bus));
        SpawnTool::new(manager)
    }

    #[test]
    fn test_spawn_capabilities() {
        let tool = make_tool();
        let caps = tool.capabilities();
        assert!(caps.built_in);
        assert!(!caps.network_outbound);
        assert_eq!(caps.subagent_access, SubagentAccess::Denied);
        assert!(caps.actions.is_empty());
    }
}
