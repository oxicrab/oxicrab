use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use crate::bus::OutboundMessage;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct MessageTool {
    send_tx: Option<Arc<mpsc::Sender<OutboundMessage>>>,
    default_channel: Arc<tokio::sync::Mutex<String>>,
    default_chat_id: Arc<tokio::sync::Mutex<String>>,
}

impl MessageTool {
    pub fn new(send_tx: Option<Arc<mpsc::Sender<OutboundMessage>>>) -> Self {
        Self {
            send_tx,
            default_channel: Arc::new(tokio::sync::Mutex::new(String::new())),
            default_chat_id: Arc::new(tokio::sync::Mutex::new(String::new())),
        }
    }
}

#[async_trait]
impl Tool for MessageTool {
    fn name(&self) -> &str {
        "message"
    }

    fn description(&self) -> &str {
        "Send a message to a user on any channel. Defaults to the current conversation's channel and chat, or specify 'channel' and 'chat_id' to target a different destination."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The message content to send"
                },
                "channel": {
                    "type": "string",
                    "description": "Optional: target channel (telegram, discord, etc.)"
                },
                "chat_id": {
                    "type": "string",
                    "description": "Optional: target chat/user ID"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let content = params["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?
            .to_string();

        let channel = if let Some(ch) = params["channel"].as_str() {
            ch.to_string()
        } else {
            self.default_channel.lock().await.clone()
        };

        let chat_id = if let Some(cid) = params["chat_id"].as_str() {
            cid.to_string()
        } else {
            self.default_chat_id.lock().await.clone()
        };

        if channel.is_empty() || chat_id.is_empty() {
            return Ok(ToolResult::error(
                "Error: No target channel/chat specified".to_string(),
            ));
        }

        if let Some(tx) = &self.send_tx {
            let channel_clone = channel.clone();
            let chat_id_clone = chat_id.clone();
            let msg = OutboundMessage {
                channel,
                chat_id,
                content,
                reply_to: None,
                media: vec![],
                metadata: std::collections::HashMap::new(),
            };
            tx.send(msg)
                .await
                .map_err(|e| anyhow::anyhow!("Send error: {}", e))?;
            Ok(ToolResult::new(format!(
                "Message sent to {}:{}",
                channel_clone, chat_id_clone
            )))
        } else {
            Ok(ToolResult::error(
                "Error: Message sending not configured".to_string(),
            ))
        }
    }

    async fn set_context(&self, channel: &str, chat_id: &str) {
        *self.default_channel.lock().await = channel.to_string();
        *self.default_chat_id.lock().await = chat_id.to_string();
    }
}

#[cfg(test)]
mod tests;
