use crate::agent::subagent::SubagentManager;
use crate::agent::tools::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct SpawnTool {
    manager: Arc<SubagentManager>,
    origin_channel: Arc<tokio::sync::Mutex<String>>,
    origin_chat_id: Arc<tokio::sync::Mutex<String>>,
    context_summary: Arc<tokio::sync::Mutex<Option<String>>>,
}

impl SpawnTool {
    pub fn new(manager: Arc<SubagentManager>) -> Self {
        Self {
            manager,
            origin_channel: Arc::new(tokio::sync::Mutex::new("cli".to_string())),
            origin_chat_id: Arc::new(tokio::sync::Mutex::new("direct".to_string())),
            context_summary: Arc::new(tokio::sync::Mutex::new(None)),
        }
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

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let task = params["task"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'task' parameter"))?
            .to_string();

        let label = params["label"]
            .as_str()
            .map(std::string::ToString::to_string);

        let channel = self.origin_channel.lock().await.clone();
        let chat_id = self.origin_chat_id.lock().await.clone();
        let context = self.context_summary.lock().await.clone();

        let result = self
            .manager
            .spawn(task, label, channel, chat_id, false, context)
            .await?;
        Ok(ToolResult::new(result))
    }

    async fn set_context(&self, channel: &str, chat_id: &str) {
        *self.origin_channel.lock().await = channel.to_string();
        *self.origin_chat_id.lock().await = chat_id.to_string();
    }

    async fn set_context_summary(&self, summary: &str) {
        *self.context_summary.lock().await = if summary.is_empty() {
            None
        } else {
            Some(summary.to_string())
        };
    }
}
