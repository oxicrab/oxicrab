//! Per-turn streaming primitives.
//!
//! The Feb-2026 streaming attempt failed because consumer state
//! (the live message ID being edited) lived at session scope. When
//! a turn finished and a new turn started, late deltas could still
//! edit the previous turn's message. The fix here is structural:
//! every piece of state is keyed by `turn_id`, generated fresh per
//! agent run. There is no concept of a "current message" outside
//! of one turn.
//!
//! ## Lifecycle
//!
//! 1. The agent loop generates a `turn_id` (UUID) at the start of
//!    a streamed run.
//! 2. It builds a `StreamDispatcher` (carrying a sender side of an
//!    mpsc channel) and emits `StreamEvent::Begin`.
//! 3. As LLM `StreamChunk::Delta`s arrive, the dispatcher emits
//!    `StreamEvent::Delta { accumulated, .. }` with the **full**
//!    accumulated text — channels are responsible for replacing,
//!    not appending, so partial deliveries can't desync.
//! 4. On normal completion, the dispatcher emits
//!    `StreamEvent::End { outcome: Complete, .. }`.
//! 5. On error or cancellation, the dispatcher emits
//!    `StreamEvent::End { outcome: Failed | Cancelled, .. }` and
//!    the agent loop falls back to delivering the accumulated text
//!    as a regular non-streaming message.
//!
//! ## Channel side
//!
//! Channel implementations of [`StreamConsumer`] map turn_ids to
//! provider message IDs in their own per-turn state (typically a
//! `DashMap<String, MessageId>`). They MUST NOT reuse message IDs
//! across turns. When a turn `End`s, the entry is removed.

use serde::{Deserialize, Serialize};

/// Result of a [`StreamDispatcher::begin`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeginOutcome {
    /// Begin event was emitted on this call.
    Sent,
    /// Begin had already been emitted by an earlier call on this
    /// dispatcher (or a clone of it).
    AlreadySent,
    /// The channel-side receiver was dropped, so no further events
    /// will be delivered. Caller should fall back to non-streaming
    /// delivery.
    ReceiverGone,
}

/// What happened at end-of-stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamOutcome {
    /// Stream completed normally with a `Finish` chunk.
    Complete,
    /// The cancellation token fired mid-stream.
    Cancelled,
    /// The provider yielded `StreamChunk::Error`, or the consumer
    /// hit an unrecoverable channel error. The agent loop will
    /// emit the accumulated content as a non-streaming message.
    Failed,
}

/// Events the agent loop emits to channel-side consumers.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// New stream starting. The consumer should send an initial
    /// (empty / "thinking…") message to the channel and store its
    /// provider message ID indexed by `turn_id`.
    Begin {
        turn_id: String,
        channel: String,
        chat_id: String,
    },
    /// Incremental update. `accumulated` is the **full content**
    /// produced by the LLM so far, not the delta — the consumer
    /// replaces the message body unconditionally.
    Delta {
        turn_id: String,
        accumulated: String,
    },
    /// Stream finished. The consumer should perform any final edit
    /// to ensure `final_content` is the canonical message body and
    /// attach `buttons` (when present) to that final message before
    /// dropping its per-turn state for `turn_id`.
    End {
        turn_id: String,
        outcome: StreamOutcome,
        final_content: String,
        /// Unified-format button metadata (the `meta::BUTTONS` JSON
        /// array) to render on the final message edit. `None` skips
        /// keyboard rendering — the consumer just commits the text.
        buttons: Option<serde_json::Value>,
    },
}

/// A handle the iteration loop uses to push `StreamEvent`s without
/// caring about which channel is on the other side. Built around an
/// unbounded mpsc sender so the LLM stream is never blocked by a
/// slow channel — channels apply their own throttling on receive.
///
/// `begin_emitted` is shared across clones so multiple iterations of
/// the same agent run cannot send a duplicate `Begin` (which would
/// create a second placeholder message in the channel). The first
/// non-empty Delta in the run wins.
#[derive(Debug, Clone)]
pub struct StreamDispatcher {
    sender: tokio::sync::mpsc::UnboundedSender<StreamEvent>,
    turn_id: String,
    channel: String,
    chat_id: String,
    begin_emitted: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl StreamDispatcher {
    pub fn new(
        sender: tokio::sync::mpsc::UnboundedSender<StreamEvent>,
        turn_id: String,
        channel: String,
        chat_id: String,
    ) -> Self {
        Self {
            sender,
            turn_id,
            channel,
            chat_id,
            begin_emitted: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    pub fn channel(&self) -> &str {
        &self.channel
    }

    pub fn chat_id(&self) -> &str {
        &self.chat_id
    }

    /// True iff a successful `Begin` has already been emitted on
    /// this dispatcher (from any clone). The first non-empty Delta
    /// in the agent run flips this to true; subsequent iterations
    /// that resume streaming on the same dispatcher must skip Begin.
    pub fn has_begun(&self) -> bool {
        self.begin_emitted
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Emit a `Begin` exactly once per dispatcher. Returns:
    /// - [`BeginOutcome::Sent`] — Begin sent successfully now
    /// - [`BeginOutcome::AlreadySent`] — Begin was already sent by
    ///   an earlier call (later iterations of the same agent run)
    /// - [`BeginOutcome::ReceiverGone`] — channel-side receiver was
    ///   dropped; caller should fall back to non-streaming delivery
    pub fn begin(&self) -> BeginOutcome {
        if self
            .begin_emitted
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return BeginOutcome::AlreadySent;
        }
        if self
            .sender
            .send(StreamEvent::Begin {
                turn_id: self.turn_id.clone(),
                channel: self.channel.clone(),
                chat_id: self.chat_id.clone(),
            })
            .is_err()
        {
            // Restore the flag so a future retry could fire, even
            // though in practice a closed mpsc never recovers.
            self.begin_emitted
                .store(false, std::sync::atomic::Ordering::Release);
            return BeginOutcome::ReceiverGone;
        }
        BeginOutcome::Sent
    }

    pub fn delta(&self, accumulated: &str) -> bool {
        self.sender
            .send(StreamEvent::Delta {
                turn_id: self.turn_id.clone(),
                accumulated: accumulated.to_string(),
            })
            .is_ok()
    }

    pub fn end(
        &self,
        outcome: StreamOutcome,
        final_content: &str,
        buttons: Option<serde_json::Value>,
    ) -> bool {
        self.sender
            .send(StreamEvent::End {
                turn_id: self.turn_id.clone(),
                outcome,
                final_content: final_content.to_string(),
                buttons,
            })
            .is_ok()
    }
}

/// Channel-side stream consumer. Each channel that supports live
/// edits (Slack, Discord, Telegram) implements this to bind a
/// per-turn message ID to a `turn_id` and apply edits against it.
///
/// The trait is intentionally `async-trait`-shaped rather than
/// using GATs to keep it object-safe — the channel manager holds
/// `Arc<dyn StreamConsumer>`.
#[async_trait::async_trait]
pub trait StreamConsumer: Send + Sync {
    /// Send an initial empty message and remember its provider
    /// message ID for `turn_id`. Returning `Err` aborts streaming
    /// for this turn — the agent loop falls back to non-streaming.
    async fn begin(&self, turn_id: &str, chat_id: &str) -> anyhow::Result<()>;

    /// Replace the body of the message bound to `turn_id` with
    /// `content`. Implementations SHOULD throttle (~1 edit/sec).
    async fn update(&self, turn_id: &str, content: &str) -> anyhow::Result<()>;

    /// Final write for `turn_id`: commit `final_content` and attach
    /// `buttons` (when present) to the same message in one shot.
    /// Implementations MUST drop their per-turn state after this
    /// call returns. When `buttons` is `None`, do not modify any
    /// existing keyboard state — just commit the text.
    async fn end(
        &self,
        turn_id: &str,
        outcome: StreamOutcome,
        final_content: &str,
        buttons: Option<&serde_json::Value>,
    ) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dispatcher_emits_full_lifecycle() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let d = StreamDispatcher::new(tx, "turn-1".into(), "telegram".into(), "chat-42".into());

        assert_eq!(d.begin(), BeginOutcome::Sent);
        assert!(d.delta("hello"));
        assert!(d.delta("hello world"));
        assert!(d.end(StreamOutcome::Complete, "hello world.", None));

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        assert_eq!(events.len(), 4);
        assert!(matches!(events[0], StreamEvent::Begin { .. }));
        assert!(matches!(
            events[3],
            StreamEvent::End {
                outcome: StreamOutcome::Complete,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn dispatcher_returns_false_when_receiver_dropped() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
        drop(rx);
        let d = StreamDispatcher::new(tx, "t".into(), "c".into(), "x".into());
        assert_eq!(d.begin(), BeginOutcome::ReceiverGone);
    }

    #[tokio::test]
    async fn begin_is_idempotent_across_clones() {
        // Mixed text+tool turns may try to emit Begin on a SECOND
        // streamed iteration of the same run. The dispatcher's
        // begin_emitted atomic must guarantee at-most-one Begin.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let d = StreamDispatcher::new(tx, "turn-1".into(), "c".into(), "x".into());
        let d_clone = d.clone();

        assert_eq!(d.begin(), BeginOutcome::Sent, "first begin succeeds");
        assert_eq!(
            d_clone.begin(),
            BeginOutcome::AlreadySent,
            "second begin is a no-op"
        );
        assert!(d.has_begun());
        assert!(d_clone.has_begun());

        let mut count = 0;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, StreamEvent::Begin { .. }) {
                count += 1;
            }
        }
        assert_eq!(count, 1, "exactly one Begin event must reach the channel");
    }

    #[tokio::test]
    async fn turn_ids_are_isolated() {
        // Two dispatchers with distinct turn_ids share the same
        // sender and the receiver can demultiplex by turn_id.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let d1 = StreamDispatcher::new(tx.clone(), "turn-A".into(), "c".into(), "x".into());
        let d2 = StreamDispatcher::new(tx, "turn-B".into(), "c".into(), "x".into());

        let _ = d1.begin();
        d1.delta("a1");
        let _ = d2.begin();
        d2.delta("b1");
        d1.end(StreamOutcome::Complete, "a-final", None);
        d2.end(StreamOutcome::Complete, "b-final", None);

        let mut a_count = 0;
        let mut b_count = 0;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                StreamEvent::Begin { turn_id, .. }
                | StreamEvent::Delta { turn_id, .. }
                | StreamEvent::End { turn_id, .. } => {
                    if turn_id == "turn-A" {
                        a_count += 1;
                    } else if turn_id == "turn-B" {
                        b_count += 1;
                    }
                }
            }
        }
        assert_eq!(a_count, 3);
        assert_eq!(b_count, 3);
    }
}
