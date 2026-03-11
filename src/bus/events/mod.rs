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
    /// Whether this execution originates from a cron job (`bool`).
    pub const IS_CRON_JOB: &str = "is_cron_job";
    /// Interactive buttons to attach to the outbound message (`array`).
    /// Unified format: `[{"id": "...", "label": "...", "style": "primary|danger|success|secondary"}]`
    pub const BUTTONS: &str = "buttons";
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

    /// Start building an `InboundMessage` with the required fields.
    /// Timestamp defaults to `Utc::now()`.
    pub fn builder(
        channel: impl Into<String>,
        sender_id: impl Into<String>,
        chat_id: impl Into<String>,
        content: impl Into<String>,
    ) -> InboundMessageBuilder {
        InboundMessageBuilder {
            inner: InboundMessage {
                channel: channel.into(),
                sender_id: sender_id.into(),
                chat_id: chat_id.into(),
                content: content.into(),
                timestamp: Utc::now(),
                media: Vec::new(),
                metadata: HashMap::new(),
            },
        }
    }
}

/// Builder for [`InboundMessage`]. Created via [`InboundMessage::builder()`].
#[must_use]
pub struct InboundMessageBuilder {
    inner: InboundMessage,
}

impl InboundMessageBuilder {
    pub fn timestamp(mut self, ts: DateTime<Utc>) -> Self {
        self.inner.timestamp = ts;
        self
    }

    pub fn media(mut self, paths: Vec<String>) -> Self {
        self.inner.media = paths;
        self
    }

    /// Insert a single metadata key-value pair.
    pub fn meta(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.inner.metadata.insert(key.into(), value);
        self
    }

    /// Replace the entire metadata map.
    pub fn metadata(mut self, map: HashMap<String, serde_json::Value>) -> Self {
        self.inner.metadata = map;
        self
    }

    /// Shorthand for `.meta(meta::IS_GROUP, Value::Bool(flag))`.
    pub fn is_group(self, flag: bool) -> Self {
        self.meta(meta::IS_GROUP, serde_json::Value::Bool(flag))
    }

    pub fn build(self) -> InboundMessage {
        self.inner
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

impl OutboundMessage {
    /// Start building an `OutboundMessage` with the required fields.
    pub fn builder(
        channel: impl Into<String>,
        chat_id: impl Into<String>,
        content: impl Into<String>,
    ) -> OutboundMessageBuilder {
        OutboundMessageBuilder {
            inner: OutboundMessage {
                channel: channel.into(),
                chat_id: chat_id.into(),
                content: content.into(),
                reply_to: None,
                media: Vec::new(),
                metadata: HashMap::new(),
            },
        }
    }

    /// Build from an inbound message, moving `channel`, `chat_id`, and `metadata`.
    pub fn from_inbound(msg: InboundMessage, content: impl Into<String>) -> OutboundMessageBuilder {
        OutboundMessageBuilder {
            inner: OutboundMessage {
                channel: msg.channel,
                chat_id: msg.chat_id,
                content: content.into(),
                reply_to: None,
                media: Vec::new(),
                metadata: msg.metadata,
            },
        }
    }
}

/// Builder for [`OutboundMessage`]. Created via [`OutboundMessage::builder()`] or
/// [`OutboundMessage::from_inbound()`].
#[must_use]
pub struct OutboundMessageBuilder {
    inner: OutboundMessage,
}

impl OutboundMessageBuilder {
    pub fn media(mut self, paths: Vec<String>) -> Self {
        self.inner.media = paths;
        self
    }

    pub fn reply_to(mut self, id: impl Into<String>) -> Self {
        self.inner.reply_to = Some(id.into());
        self
    }

    /// Insert a single metadata key-value pair.
    pub fn meta(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.inner.metadata.insert(key.into(), value);
        self
    }

    /// Replace the entire metadata map.
    pub fn metadata(mut self, map: HashMap<String, serde_json::Value>) -> Self {
        self.inner.metadata = map;
        self
    }

    /// Merge keys from a `HashMap` without replacing existing keys.
    /// Inbound metadata (e.g. `ts`, `is_group`) takes precedence.
    pub fn merge_metadata(mut self, extra: HashMap<String, serde_json::Value>) -> Self {
        for (k, v) in extra {
            self.inner.metadata.entry(k).or_insert(v);
        }
        self
    }

    pub fn build(self) -> OutboundMessage {
        self.inner
    }
}

#[cfg(test)]
mod tests;
