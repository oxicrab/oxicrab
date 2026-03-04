use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Well-known metadata keys for [`InboundMessage`] and [`OutboundMessage`].
///
/// Using these constants prevents typos and makes metadata key usage discoverable.
pub mod meta {
    /// Whether the message originates from a group chat (`bool`).
    pub const IS_GROUP: &str = "is_group";
    /// Slack/Telegram message timestamp for threading (`string`).
    pub const TS: &str = "ts";
    /// Slack thread timestamp for reply threading (`string`).
    pub const THREAD_TS: &str = "thread_ts";
    /// Whether this outbound message is a streaming status update (`bool`).
    pub const STATUS: &str = "status";
    /// Gateway HTTP session ID for conversation continuity (`string`).
    pub const SESSION_ID: &str = "session_id";
    /// Requested response format from the HTTP API (`json`).
    pub const RESPONSE_FORMAT: &str = "response_format";
    /// Name of the webhook that produced this message (`string`).
    pub const WEBHOOK_SOURCE: &str = "webhook_source";
    /// Name of the webhook that triggered this inbound message (`string`).
    pub const WEBHOOK_NAME: &str = "webhook_name";
    /// Provider-reported input tokens from the last LLM call (`u64`).
    pub const LAST_INPUT_TOKENS: &str = "last_input_tokens";
    /// Names of tools used during agent processing (`string[]`).
    pub const TOOLS_USED: &str = "tools_used";
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub channel: String,
    pub chat_id: String,
    pub content: String,
    pub reply_to: Option<String>,
    pub media: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests;
