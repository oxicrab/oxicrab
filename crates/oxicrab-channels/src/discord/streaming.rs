//! Discord-side `StreamConsumer` impl. Edits a single Discord
//! message progressively as deltas arrive.
//!
//! Per-turn isolation via `turn_id` keyed `DashMap`. Late deltas for
//! a removed turn are silently dropped.

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use oxicrab_core::streaming::{StreamConsumer, StreamOutcome};
use serenity::model::id::{ChannelId, MessageId};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

use crate::discord::payloads::parse_unified_buttons_value;
use crate::dispatch::DispatchContextStore;

/// Discord allows roughly 5 edits per second per webhook. We use a
/// 1-second throttle window matching the other channels for parity.
const EDIT_THROTTLE: Duration = Duration::from_millis(1_000);

/// Discord message limit.
const MAX_DISCORD_LEN: usize = 2_000;

const PLACEHOLDER_BODY: &str = "…";

struct TurnState {
    channel_id: ChannelId,
    message_id: MessageId,
    last_edit: Instant,
}

#[derive(Clone)]
pub struct DiscordStreamConsumer {
    http: Arc<serenity::http::Http>,
    state: Arc<DashMap<String, TurnState>>,
    dispatch_store: Arc<DispatchContextStore>,
}

impl DiscordStreamConsumer {
    pub fn new(http: Arc<serenity::http::Http>, dispatch_store: Arc<DispatchContextStore>) -> Self {
        Self {
            http,
            state: Arc::new(DashMap::new()),
            dispatch_store,
        }
    }

    fn parse_channel_id(chat_id: &str) -> Result<ChannelId> {
        let id: u64 = chat_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid discord channel_id '{chat_id}': {e}"))?;
        Ok(ChannelId::new(id))
    }

    fn truncate(content: &str) -> &str {
        if content.len() <= MAX_DISCORD_LEN {
            return content;
        }
        let mut idx = MAX_DISCORD_LEN;
        while idx > 0 && !content.is_char_boundary(idx) {
            idx -= 1;
        }
        &content[..idx]
    }
}

#[async_trait]
impl StreamConsumer for DiscordStreamConsumer {
    async fn begin(&self, turn_id: &str, chat_id: &str) -> Result<()> {
        let cid = Self::parse_channel_id(chat_id)?;
        let sent = cid
            .say(self.http.as_ref(), PLACEHOLDER_BODY)
            .await
            .map_err(|e| anyhow::anyhow!("discord stream begin send failed: {e}"))?;
        self.state.insert(
            turn_id.to_string(),
            TurnState {
                channel_id: cid,
                message_id: sent.id,
                last_edit: Instant::now()
                    .checked_sub(EDIT_THROTTLE)
                    .unwrap_or_else(Instant::now),
            },
        );
        debug!(
            "discord stream begin: turn={turn_id} chan={cid} msg_id={:?}",
            sent.id
        );
        Ok(())
    }

    async fn update(&self, turn_id: &str, content: &str) -> Result<()> {
        if content.is_empty() {
            return Ok(());
        }
        let (cid, mid) = {
            let Some(mut entry) = self.state.get_mut(turn_id) else {
                debug!("discord stream update: unknown turn_id={turn_id}, dropping");
                return Ok(());
            };
            // Throttle: drop deltas inside the window. Each delta
            // carries the FULL accumulated text; end() commits the
            // final state unconditionally.
            if entry.last_edit.elapsed() < EDIT_THROTTLE {
                return Ok(());
            }
            entry.last_edit = Instant::now();
            (entry.channel_id, entry.message_id)
        };

        let body = Self::truncate(content);
        let builder = serenity::builder::EditMessage::new().content(body);
        if let Err(e) = cid.edit_message(self.http.as_ref(), mid, builder).await {
            warn!("discord stream edit failed for turn={turn_id}: {e}");
        }
        Ok(())
    }

    async fn end(
        &self,
        turn_id: &str,
        outcome: StreamOutcome,
        final_content: &str,
        buttons: Option<&serde_json::Value>,
    ) -> Result<()> {
        let Some((_k, state)) = self.state.remove(turn_id) else {
            debug!("discord stream end: unknown turn_id={turn_id}");
            return Ok(());
        };
        let body = Self::truncate(final_content);
        if body.is_empty() && buttons.is_none() {
            return Ok(());
        }
        // Discord rejects edits with empty content. Substitute the
        // same zero-width-space placeholder used by Telegram so a
        // buttons-only follow-up still commits.
        let body = if body.is_empty() { "\u{200B}" } else { body };
        // Discord lets editMessage carry both content and components
        // in one call, so the streamed message ends up with buttons
        // attached without a sidecar.
        let mut builder = serenity::builder::EditMessage::new().content(body);
        if let Some(buttons_val) = buttons {
            let rows = parse_unified_buttons_value(buttons_val, Some(&self.dispatch_store));
            if !rows.is_empty() {
                builder = builder.components(rows);
            }
        }
        if let Err(e) = state
            .channel_id
            .edit_message(self.http.as_ref(), state.message_id, builder)
            .await
        {
            warn!(
                "discord stream final edit failed (outcome={:?}) for turn={turn_id}: {e}",
                outcome
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_caps_at_2000() {
        let s = "x".repeat(3000);
        assert_eq!(DiscordStreamConsumer::truncate(&s).len(), MAX_DISCORD_LEN);
    }

    #[test]
    fn truncate_short_content_unchanged() {
        let s = "hello world";
        assert_eq!(DiscordStreamConsumer::truncate(s), s);
    }

    #[test]
    fn parse_channel_id_validates() {
        assert!(DiscordStreamConsumer::parse_channel_id("12345").is_ok());
        assert!(DiscordStreamConsumer::parse_channel_id("abc").is_err());
    }
}
