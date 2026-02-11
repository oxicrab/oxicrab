use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub channel: String,
    pub sender_id: String,
    pub chat_id: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub media: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl InboundMessage {
    pub fn session_key(&self) -> String {
        format!("{}:{}", self.channel, self.chat_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub channel: String,
    pub chat_id: String,
    pub content: String,
    pub reply_to: Option<String>,
    pub media: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// A streaming edit request sent from the agent loop to the channel manager.
/// When `message_id` is empty, the consumer should send a new message via
/// `send_and_get_id` and track the returned ID for subsequent edits.
#[derive(Debug, Clone)]
pub struct StreamingEdit {
    pub channel: String,
    pub chat_id: String,
    /// Platform-specific message ID for editing. Empty on initial send
    /// (the consumer tracks the ID after `send_and_get_id`).
    #[allow(dead_code)]
    pub message_id: String,
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_streaming_edit_struct() {
        let edit = StreamingEdit {
            channel: "telegram".into(),
            chat_id: "123".into(),
            message_id: "msg1".into(),
            content: "Hello world".into(),
        };
        assert_eq!(edit.channel, "telegram");
        assert_eq!(edit.message_id, "msg1");
    }

    #[test]
    fn test_streaming_edit_empty_message_id() {
        let edit = StreamingEdit {
            channel: "slack".into(),
            chat_id: "C123".into(),
            message_id: String::new(),
            content: "...".into(),
        };
        assert!(edit.message_id.is_empty());
    }

    #[test]
    fn test_streaming_edit_reset_sentinel() {
        // Empty content signals the consumer to reset its tracked message ID
        let edit = StreamingEdit {
            channel: "telegram".into(),
            chat_id: "123".into(),
            message_id: String::new(),
            content: String::new(),
        };
        assert!(edit.content.is_empty(), "Reset sentinel has empty content");
        assert!(
            edit.message_id.is_empty(),
            "Reset sentinel has empty message_id"
        );
    }
}
