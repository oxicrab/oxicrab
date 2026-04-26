//! Telegram-side `StreamConsumer` impl. Edits a single Telegram
//! message progressively as deltas arrive from the agent loop.
//!
//! Per-turn isolation: the `state` map is keyed by `turn_id` (the
//! UUID generated per agent run). Each turn gets its own
//! `(ChatId, MessageId)` slot; when the turn ends, the slot is
//! removed. Late deltas for a removed turn are silently dropped —
//! they have no message to edit. This is the structural fix for the
//! Feb-2026 regression where session-keyed state let late chunks
//! corrupt the next turn's message.

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use oxicrab_core::streaming::{StreamConsumer, StreamOutcome};
use std::sync::Arc;
use std::time::{Duration, Instant};
use teloxide::prelude::*;
use teloxide::types::{ChatId, MessageId};
use tracing::{debug, warn};

use crate::dispatch::DispatchContextStore;
use crate::telegram::build_inline_keyboard_from_value;

/// Minimum wall time between successive `editMessageText` calls for
/// the same message. Telegram allows roughly 1 edit/sec per message
/// before it starts rejecting with `429 Too Many Requests`.
const EDIT_THROTTLE: Duration = Duration::from_millis(1_000);

/// Initial placeholder body sent on `Begin`. The model's first
/// non-empty `Delta` overwrites it; the placeholder exists only so
/// the Telegram message has some body (the API rejects empty
/// `sendMessage` payloads).
const PLACEHOLDER_BODY: &str = "…";

/// Telegram messages cap at 4096 chars. Truncate beyond that to keep
/// edits valid; the operator-visible message is replaced by the
/// caller's normal post-stream send when this limit is hit.
const MAX_TELEGRAM_LEN: usize = 4096;

struct TurnState {
    chat_id: ChatId,
    message_id: MessageId,
    last_edit: Instant,
}

#[derive(Clone)]
pub struct TelegramStreamConsumer {
    bot: Bot,
    /// Per-turn message bookkeeping. Keyed by `turn_id` so two
    /// concurrent turns in the same chat never share state.
    state: Arc<DashMap<String, TurnState>>,
    /// Shared dispatch store so a button click on a streamed message
    /// can recover full action-dispatch context that doesn't fit in
    /// Telegram's 64-byte `callback_data`.
    dispatch_store: Arc<DispatchContextStore>,
}

impl TelegramStreamConsumer {
    pub fn new(bot: Bot, dispatch_store: Arc<DispatchContextStore>) -> Self {
        Self {
            bot,
            state: Arc::new(DashMap::new()),
            dispatch_store,
        }
    }

    fn parse_chat_id(chat_id: &str) -> Result<ChatId> {
        let parsed: i64 = chat_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid telegram chat_id '{chat_id}': {e}"))?;
        Ok(ChatId(parsed))
    }

    fn truncate_for_telegram(content: &str) -> &str {
        if content.len() <= MAX_TELEGRAM_LEN {
            return content;
        }
        // Walk back to a UTF-8 boundary so we don't slice mid-codepoint.
        let mut idx = MAX_TELEGRAM_LEN;
        while idx > 0 && !content.is_char_boundary(idx) {
            idx -= 1;
        }
        &content[..idx]
    }
}

#[async_trait]
impl StreamConsumer for TelegramStreamConsumer {
    async fn begin(&self, turn_id: &str, chat_id: &str) -> Result<()> {
        let cid = Self::parse_chat_id(chat_id)?;
        let sent = self.bot.send_message(cid, PLACEHOLDER_BODY).await?;
        self.state.insert(
            turn_id.to_string(),
            TurnState {
                chat_id: cid,
                message_id: sent.id,
                // Set last_edit to the past so the first delta isn't throttled.
                last_edit: Instant::now()
                    .checked_sub(EDIT_THROTTLE)
                    .unwrap_or_else(Instant::now),
            },
        );
        debug!(
            "telegram stream begin: turn={turn_id} chat={cid} msg_id={:?}",
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
                debug!("telegram stream update: unknown turn_id={turn_id}, dropping");
                return Ok(());
            };
            // Throttle: drop this delta if we edited within the
            // window. Each delta carries the FULL accumulated text,
            // so dropping intermediate deltas only costs a temporary
            // visual lag — `end()` always commits the final state.
            if entry.last_edit.elapsed() < EDIT_THROTTLE {
                return Ok(());
            }
            entry.last_edit = Instant::now();
            (entry.chat_id, entry.message_id)
        };

        let body = Self::truncate_for_telegram(content);
        if let Err(e) = self.bot.edit_message_text(cid, mid, body).await {
            // Don't propagate edit failures — the agent loop's
            // fallback path will deliver the final message via
            // normal send if needed.
            warn!("telegram stream edit failed for turn={turn_id}: {e}");
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
            debug!("telegram stream end: unknown turn_id={turn_id}");
            return Ok(());
        };
        // Always perform the closing edit, even if the throttle
        // window has not elapsed — the user sees the final answer.
        let body = Self::truncate_for_telegram(final_content);
        if body.is_empty() && buttons.is_none() {
            return Ok(());
        }
        let body = if body.is_empty() { "\u{200B}" } else { body };

        let keyboard = buttons.and_then(|buttons_val| {
            build_inline_keyboard_from_value(buttons_val, Some(&self.dispatch_store))
        });

        // Single-call commit: editMessageText accepts reply_markup so
        // text + keyboard land atomically (no window where text is
        // updated but buttons aren't yet attached). Telegram replaces
        // any existing markup; if `keyboard` is None the markup is
        // cleared, which is what we want when no buttons are present.
        let mut req = self
            .bot
            .edit_message_text(state.chat_id, state.message_id, body);
        if let Some(kb) = keyboard {
            req = req.reply_markup(kb);
        }
        if let Err(e) = req.await {
            warn!(
                "telegram stream final edit failed (outcome={:?}) for turn={turn_id}: {e}",
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
    fn truncate_keeps_short_content() {
        let s = "hello";
        assert_eq!(TelegramStreamConsumer::truncate_for_telegram(s), s);
    }

    #[test]
    fn truncate_caps_long_content() {
        let s = "x".repeat(5000);
        let out = TelegramStreamConsumer::truncate_for_telegram(&s);
        assert_eq!(out.len(), MAX_TELEGRAM_LEN);
    }

    #[test]
    fn truncate_respects_utf8_boundaries() {
        // 4-byte emoji at the cap boundary.
        let mut s = "a".repeat(MAX_TELEGRAM_LEN - 1);
        s.push_str("🦀🦀");
        let out = TelegramStreamConsumer::truncate_for_telegram(&s);
        // Must be a valid &str slice — implicitly checked by
        // truncate_for_telegram returning &str without panicking.
        assert!(out.len() <= MAX_TELEGRAM_LEN);
    }

    #[test]
    fn parse_chat_id_rejects_non_numeric() {
        assert!(TelegramStreamConsumer::parse_chat_id("abc").is_err());
        assert!(TelegramStreamConsumer::parse_chat_id("123").is_ok());
        assert!(TelegramStreamConsumer::parse_chat_id("-1001234").is_ok());
    }
}
