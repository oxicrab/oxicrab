//! Slack-side `StreamConsumer` impl. Sends an initial
//! `chat.postMessage`, then progressively edits via `chat.update` as
//! deltas arrive.
//!
//! Per-turn isolation via `turn_id` keyed `DashMap`. Late deltas for
//! a removed turn are silently dropped.

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use oxicrab_core::streaming::{StreamConsumer, StreamOutcome};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Slack `chat.update` allows ~50 edits/min per channel. We use a
/// 1-second throttle window matching the other channels for parity
/// and to keep edit volume well under that ceiling.
const EDIT_THROTTLE: Duration = Duration::from_millis(1_000);

/// Slack hard-caps message text at 40k chars. Practically, anything
/// over a few thousand looks awful in the client; truncate at 8k as
/// a sane upper bound for streamed turns.
const MAX_SLACK_LEN: usize = 8_000;

const PLACEHOLDER_BODY: &str = "…";

struct TurnState {
    channel: String,
    /// Slack message timestamp (its primary key for `chat.update`).
    ts: String,
    last_edit: Instant,
    skipped_edit: bool,
}

#[derive(Clone)]
pub struct SlackStreamConsumer {
    bot_token: String,
    client: reqwest::Client,
    state: Arc<DashMap<String, TurnState>>,
}

impl SlackStreamConsumer {
    pub fn new(bot_token: String, client: reqwest::Client) -> Self {
        Self {
            bot_token,
            client,
            state: Arc::new(DashMap::new()),
        }
    }

    fn truncate(content: &str) -> &str {
        if content.len() <= MAX_SLACK_LEN {
            return content;
        }
        let mut idx = MAX_SLACK_LEN;
        while idx > 0 && !content.is_char_boundary(idx) {
            idx -= 1;
        }
        &content[..idx]
    }

    async fn slack_post(&self, method: &str, params: &HashMap<&str, Value>) -> Result<Value> {
        let url = format!("https://slack.com/api/{method}");
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.bot_token)
            .form(params)
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() || body.get("ok") != Some(&Value::Bool(true)) {
            anyhow::bail!("slack {method} failed: status={status} body={body}",);
        }
        Ok(body)
    }
}

#[async_trait]
impl StreamConsumer for SlackStreamConsumer {
    async fn begin(&self, turn_id: &str, chat_id: &str) -> Result<()> {
        let mut params: HashMap<&str, Value> = HashMap::new();
        params.insert("channel", Value::String(chat_id.to_string()));
        params.insert("text", Value::String(PLACEHOLDER_BODY.to_string()));
        let body = self.slack_post("chat.postMessage", &params).await?;
        let ts = body
            .get("ts")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("slack postMessage missing ts"))?
            .to_string();
        self.state.insert(
            turn_id.to_string(),
            TurnState {
                channel: chat_id.to_string(),
                ts,
                last_edit: Instant::now()
                    .checked_sub(EDIT_THROTTLE)
                    .unwrap_or_else(Instant::now),
                skipped_edit: false,
            },
        );
        debug!("slack stream begin: turn={turn_id} channel={chat_id}");
        Ok(())
    }

    async fn update(&self, turn_id: &str, content: &str) -> Result<()> {
        if content.is_empty() {
            return Ok(());
        }
        let (channel, ts) = {
            let Some(mut entry) = self.state.get_mut(turn_id) else {
                debug!("slack stream update: unknown turn_id={turn_id}, dropping");
                return Ok(());
            };
            if entry.last_edit.elapsed() < EDIT_THROTTLE && !entry.skipped_edit {
                entry.skipped_edit = true;
                return Ok(());
            }
            entry.last_edit = Instant::now();
            entry.skipped_edit = false;
            (entry.channel.clone(), entry.ts.clone())
        };

        let body = Self::truncate(content);
        let mut params: HashMap<&str, Value> = HashMap::new();
        params.insert("channel", Value::String(channel));
        params.insert("ts", Value::String(ts));
        params.insert("text", Value::String(body.to_string()));
        if let Err(e) = self.slack_post("chat.update", &params).await {
            warn!("slack stream edit failed for turn={turn_id}: {e}");
        }
        Ok(())
    }

    async fn end(
        &self,
        turn_id: &str,
        outcome: StreamOutcome,
        final_content: &str,
        buttons: Option<&Value>,
    ) -> Result<()> {
        let Some((_k, state)) = self.state.remove(turn_id) else {
            debug!("slack stream end: unknown turn_id={turn_id}");
            return Ok(());
        };
        let body = Self::truncate(final_content);
        if body.is_empty() && buttons.is_none() {
            return Ok(());
        }

        // Slack chat.update accepts both `text` and `blocks` in one
        // call, so the buttons land on the same message that was
        // streamed — no sidecar.
        let button_blocks = buttons
            .map(crate::slack::convert_buttons_value_to_blocks)
            .unwrap_or_default();

        if button_blocks.is_empty() {
            let mut params: HashMap<&str, Value> = HashMap::new();
            params.insert("channel", Value::String(state.channel));
            params.insert("ts", Value::String(state.ts));
            params.insert("text", Value::String(body.to_string()));
            if let Err(e) = self.slack_post("chat.update", &params).await {
                warn!(
                    "slack stream final edit failed (outcome={:?}) for turn={turn_id}: {e}",
                    outcome
                );
            }
            return Ok(());
        }

        // With buttons: use chat.update with JSON body so blocks
        // serialize correctly. Block Kit section.text caps at 3000
        // chars, so we have to clip the body further than the
        // streaming throttle's 8k cap.
        let block_text: String = if body.chars().count() > 3000 {
            let byte_idx = body.char_indices().nth(2997).map_or(body.len(), |(i, _)| i);
            format!("{}...", &body[..byte_idx])
        } else {
            body.to_string()
        };
        let mut blocks = vec![serde_json::json!({
            "type": "section",
            "text": {"type": "mrkdwn", "text": block_text},
        })];
        blocks.extend(button_blocks);

        let body_json = serde_json::json!({
            "channel": state.channel,
            "ts": state.ts,
            "text": body,
            "blocks": blocks,
        });
        let url = "https://slack.com/api/chat.update";
        let resp = self
            .client
            .post(url)
            .bearer_auth(&self.bot_token)
            .json(&body_json)
            .send()
            .await;
        match resp {
            Ok(r) => {
                let status = r.status();
                let body: Value = r.json().await.unwrap_or(Value::Null);
                if !status.is_success() || body.get("ok") != Some(&Value::Bool(true)) {
                    warn!(
                        "slack stream final edit failed (outcome={:?}) turn={turn_id} \
                         status={status} body={body}",
                        outcome
                    );
                }
            }
            Err(e) => {
                warn!(
                    "slack stream final edit transport error (outcome={:?}) turn={turn_id}: {e}",
                    outcome
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_caps_at_max() {
        let s = "x".repeat(MAX_SLACK_LEN + 1000);
        assert_eq!(SlackStreamConsumer::truncate(&s).len(), MAX_SLACK_LEN);
    }

    #[test]
    fn truncate_short_unchanged() {
        let s = "hi";
        assert_eq!(SlackStreamConsumer::truncate(s), s);
    }
}
