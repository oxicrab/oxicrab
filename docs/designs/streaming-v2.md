# Streaming v2 — design review

Status: design only, not implemented. Captures (a) why oxicrab's first
streaming attempt was removed, (b) what the new design must guarantee
to avoid the same failure, (c) the proposed implementation. **Don't
ship streaming until this design has been reviewed.**

## Why streaming was removed (Feb 12, 2026)

Three commits tell the story:

- `fb4aba7` — Added streaming
- `729c253` — Fix streaming edits overwriting previous bot messages
- `ef350aa` — Removed streaming (same day as `729c253`)

The bug from `729c253`:

> The streaming edit consumer persisted `current_message_id` across
> separate user messages in the same chat, causing new responses to
> edit the old bot message instead of creating a fresh one.

The attempted fix sent a "reset sentinel" (empty content) at the start
of each agent loop run to clear the consumer's tracked message ID.
That apparently wasn't enough — streaming was ripped out the same day.

The architectural lesson: the **stream consumer's state was
session-global**, with reset markers as the lifecycle boundary. Any
race between "new turn starts" and "old chunk arrives late" would
corrupt the consumer state.

## What the new design must guarantee

1. **Per-turn consumer isolation.** Stream state lives entirely inside
   one agent turn. There is no session-global "current_message_id".
   When a new turn starts, a fresh consumer is constructed; the old
   one is dropped. Late chunks from a dropped consumer are silently
   ignored.

2. **Explicit lifecycle.** Every stream emits exactly one `Begin`
   event, zero or more `Delta` events, and exactly one `End` event.
   `End` always fires, even on error or cancellation, via RAII drop.

3. **Stream-to-channel via per-turn message ID.** When `Begin` fires,
   the channel sends an initial empty (or "thinking…") message and
   captures its message ID into the per-turn consumer. Subsequent
   `Delta` events edit that specific ID. The channel manager does
   NOT track message IDs at the session level.

4. **Fail-safe to non-streaming.** If anything goes wrong inside the
   stream — provider error mid-stream, edit-message API failure,
   chunk parse error — the consumer aborts streaming AND emits a
   single non-streaming OutboundMessage with the accumulated content
   so far. The user sees the partial response as a regular message
   rather than a half-edited message that stays "live" forever.

5. **Cancellation respected.** The cancellation token introduced in
   T2.2 fires through every chunk-receive `await`. On cancellation,
   the consumer emits its final state as a non-streaming message and
   stops.

6. **Off by default.** First rollout under
   `agents.defaults.streaming.enabled = false`. Operators flip it
   on per channel via `channels.<name>.stream = true` after
   validating the consumer doesn't break their channel's specific
   edit semantics (Slack vs Discord vs Telegram all differ).

## Proposed implementation

### Provider layer

`LLMProvider` trait gets a new method:

```rust
async fn chat_stream(
    &self,
    req: &ChatRequest,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<Pin<Box<dyn Stream<Item = StreamChunk> + Send>>>;
```

Where `StreamChunk` is:

```rust
pub enum StreamChunk {
    Delta { text: String },
    ToolCallStart { id: String, name: String },
    ToolCallArgs { id: String, args_delta: String },
    Finish { reason: String, full_response: LLMResponse },
    Error { message: String },
}
```

Default impl on `LLMProvider`: collect a non-streaming `chat()` and
emit a single `Delta` + `Finish` so callers don't branch.

### Iteration loop

`run_agent_loop_with_overrides` gets a `streaming_emit:
Option<Box<dyn Fn(StreamEvent) + Send + Sync>>` field on
`AgentRunOverrides`. When set, the loop calls `chat_stream` instead
of `chat` for the LLM step and wires the chunks into `streaming_emit`
+ a local `accumulated_text` (already added in T1.1!). The final
`Finish.full_response` becomes the `LLMResponse` for the rest of the
loop's existing logic — so tool calls, finish_reason guards,
duplicate detection, etc. all keep working unchanged.

### Channel emit

`StreamEvent` is what the channel layer consumes:

```rust
pub enum StreamEvent {
    Begin {
        turn_id: String,        // unique per agent turn
        channel: String,
        chat_id: String,
    },
    Delta {
        turn_id: String,
        accumulated: String,    // full content so far, not just delta
    },
    End {
        turn_id: String,
        outcome: StreamOutcome, // Complete | Cancelled | Failed
        final_content: String,
    },
}
```

Each channel implements `StreamConsumer`:

```rust
pub trait StreamConsumer: Send + Sync {
    async fn begin(&self, turn_id: &str, chat_id: &str) -> Result<MessageId>;
    async fn update(&self, turn_id: &str, message_id: &MessageId, content: &str) -> Result<()>;
    async fn end(&self, turn_id: &str, message_id: &MessageId, final_content: &str, outcome: StreamOutcome) -> Result<()>;
}
```

Per-turn state lives in a `ChannelManager` `DashMap<turn_id,
MessageId>`, **not** keyed on session. When the turn's `End` event
fires, the entry is removed. Late `Delta` events for a turn_id
that's no longer in the map are silently dropped (logs at debug
level).

### Why this avoids the original bug

The Feb-2026 failure was: session 1's stream → message ID stored on
session-keyed cursor → session 1 ends → session 2 starts → next
delta edits session-1 message (because session-keyed cursor still
holds the old ID).

The new design uses `turn_id`, generated fresh per agent turn. Each
turn has its own message ID slot. When the turn ends, the slot is
removed. There is no concept of a "current" message that persists
across turns.

The "reset sentinel" approach was a hack on top of the wrong
abstraction. Per-turn isolation is the right abstraction.

### Edit throttling

Channels have rate limits on edit-message APIs:

- Slack: `chat.update` ~50/min per channel
- Discord: ~5/sec per webhook
- Telegram: 1/sec per message (unofficial; will get capped before that in practice)

Throttle deltas to **at most 1 edit per second per turn**, accumulating
chunks in between. Always send the final content unconditionally on
`End` so the throttle gate doesn't drop the closing edit.

### Telemetry

Required metrics from day one:
- `oxicrab_streaming_turns_total{channel,outcome}`
- `oxicrab_streaming_edit_failures_total{channel,reason}`
- `oxicrab_streaming_fallback_to_nonstream_total{reason}`

If `oxicrab_streaming_fallback_to_nonstream_total` is non-trivial in
production, that's the signal to investigate before expanding rollout.

## Test plan

Before merge:
1. Mock-provider streaming end-to-end: Begin → Delta×N → End,
   message ID isolation across two consecutive turns in the same
   session.
2. Mid-stream provider error → fallback to non-streaming → user sees
   partial accumulated content as a single regular message.
3. Cancellation token fires mid-stream → End{Cancelled} emits final
   content, no further edits.
4. Two concurrent sessions streaming simultaneously → no edit
   collision (the F#13 race is impossible because turn_ids are
   distinct).
5. Late Delta after End — verify it's dropped, not applied.
6. Channel-specific edit-rate-limit response — throttle gate works.

## Out of scope for v1

- Streaming with **tool calls in the same turn**. Most channels can't
  represent "tool call mid-edit" sensibly. v1 streams the **final
  text only** — once the loop has a final assistant text response,
  that text is streamed. Tool-call iterations remain non-streaming.
  Microclaw/Zeroclaw both do it this way.
- Streaming reasoning_content (extended thinking). v1 streams only
  `text` deltas. Thinking comes through `Finish.full_response` for
  persistence, never displayed live.
- Cross-channel streaming (e.g. agent calls `send_message` to fan
  out to multiple channels mid-stream). v1 streams to the originating
  channel only.

## Adoption gate

Don't ship streaming until:
1. T2.2 cancellation token is in production and proven to abort
   in-flight calls cleanly (already shipped, needs prod soak time).
2. The default-fallback path is exercised against at least one real
   provider failure (we artificially induce one to validate).
3. The Feb-2026 regression test (Test #1 above) is in CI.

When those three are green, streaming v2 is safe to roll out behind
the off-by-default flag.
