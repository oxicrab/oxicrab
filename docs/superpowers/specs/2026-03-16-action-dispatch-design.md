# Action Dispatch Design

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan.

**Goal:** A generic action dispatch layer that intercepts structured tool call payloads before the LLM, executing them directly via `ToolRegistry`. Buttons are the first and most urgent consumer, but the same mechanism handles webhooks, cron follow-ups, channel commands, and inter-tool chaining.

**Motivation:** Both DeepSeek V3 and Kimi K2.5 fabricate tool results as text instead of calling tools when button clicks arrive as `[button:{action_id}]` messages. This is architectural — the LLM must interpret opaque action IDs, figure out which tool to call, construct parameters, and make the call. Every model tested fails at this consistently. The fix is to bypass the LLM entirely for structured actions.

## Core Data Structures

### `ActionDispatch`

```rust
/// A structured tool call that bypasses the LLM entirely.
pub struct ActionDispatch {
    pub tool: String,                // tool name (e.g., "rss", "google_calendar")
    pub params: serde_json::Value,   // full tool params (e.g., {"action": "accept", "article_ids": ["abc"]})
    pub source: ActionSource,        // provenance
}
```

`ActionDispatch` does **not** derive `Serialize`/`Deserialize`. It is consumed in `process_message_unlocked()` before any serialization occurs. All dispatch types live in `src/dispatch/mod.rs` (top-level module) because they are referenced across layers: `bus` (InboundMessage), `channels` (Slack/Discord creators), and `agent::loop` (consumer).

### `ActionSource`

```rust
pub enum ActionSource {
    Button { action_id: String },        // Slack/Discord button click
    Webhook { webhook_name: String },    // Named webhook with structured payload
    Cron { job_id: String },             // Cron follow-up action
    Command { raw: String },             // Channel slash command — deferred to future spec
    ToolChain { parent_tool: String },   // Follow-up from another tool's result
}
```

**Note:** `Command` is defined in the enum for forward compatibility but is not implemented in this spec. Channel command parsing requires its own grammar design and will be addressed separately.

### `ActionDispatchPayload`

The serialized format stored in `ButtonSpec.context` and used across all sources:

```rust
#[derive(Deserialize, Serialize)]
pub struct ActionDispatchPayload {
    pub tool: String,
    pub params: serde_json::Value,
}
```

```json
{
  "tool": "rss",
  "params": {"action": "accept", "article_ids": ["abc123"]}
}
```

This replaces the current inconsistent formats (free-text instructions for RSS, nested JSON for Google Calendar, etc.). All tools adopt this single schema.

### `InboundMessage` change

```rust
pub struct InboundMessage {
    // ... existing fields ...
    #[serde(skip)]
    pub action: Option<ActionDispatch>,  // NEW — not serialized
}
```

The builder gains `.action(dispatch)`. The field is `#[serde(skip)]` because `ActionDispatch` is consumed before any bus serialization and does not need to survive `MessageBus::publish_inbound()` truncation.

### Constants

```rust
pub const MAX_DISPATCH_CHAIN_DEPTH: usize = 3;
pub const FOLLOW_UP_ACTION: &str = "follow_up_action";
```

## Dispatch Interception Point

In `process_message_unlocked()`, after the session lock is acquired but **before** prompt guard, image encoding, or LLM invocation:

```
process_message_unlocked(msg):
    acquire session lock

    if msg.action is Some(dispatch):
        result = execute_action_dispatch(dispatch, &msg)  // secret scanning happens inside
        record_dispatch_in_session(dispatch, result, &session)
        return build_outbound_from_dispatch(result, &msg)

    // existing pipeline unchanged...
    secret scanning → prompt guard → remember fast path → image encoding → LLM
```

The same check applies in `process_direct_with_overrides()`. Rather than changing its function signature, `ActionDispatch` is added as an optional field on `AgentRunOverrides`:

```rust
pub struct AgentRunOverrides {
    // ... existing fields ...
    pub action: Option<ActionDispatch>,  // NEW
}
```

This is consistent with how `metadata` is already carried in `AgentRunOverrides`. Existing callers pass `action: None` (the default). When `Some`, `process_direct_with_overrides()` short-circuits the same way as `process_message_unlocked()`.

### `execute_action_dispatch()`

1. Build an `ExecutionContext` from the message (channel, chat_id, metadata).
2. Validate `dispatch.tool` exists in `ToolRegistry`. If not, return an error outbound message.
3. Check `tool.requires_approval()`. If true, return an error: "Action failed: tool '{name}' requires approval and cannot be auto-dispatched." This prevents untrusted MCP tools from being invoked without user confirmation.
4. Run `leak_detector.redact()` on the serialized dispatch params. This is especially important for webhook dispatches where params originate from external payloads.
5. Call `ToolRegistry::execute(dispatch.tool, dispatch.params, &ctx)`. This is the public `execute()` method which internally calls `execute_with_guards()` (panic isolation via `tokio::task::spawn`, timeout enforcement) and runs the full middleware chain (leak detection on output, truncation, prompt injection scanning on results).
6. Extract `suggested_buttons` from `result.metadata` for follow-up buttons.
7. Check for `follow_up_action` in `result.metadata`. If present, re-dispatch with depth counter. When depth exceeds `MAX_DISPATCH_CHAIN_DEPTH` (3), return the last result as-is and log a warning.
8. Build `OutboundMessage` with tool result content + buttons.

### What does NOT happen

- No LLM call.
- No hallucination detection.
- No prompt guard on the dispatch payload (it's structured data, not free text — but params are secret-scanned).

### What DOES happen

- Typing indicator fires.
- Slack reaction emoji lifecycle applies (eyes on receive, checkmark on send).
- Tool execution goes through `ToolRegistry::execute()` with full middleware.
- Dispatch params are scanned by `LeakDetector` before execution (critical for webhook sources).
- A synthetic message pair is appended to session history for conversational continuity (see Session History below).
- The outbound message routes through the normal channel dispatch path.

### Session History Recording

After a successful dispatch, two synthetic messages are appended to the session:

**User message (role=user):**
```
[action: {tool}.{action} via {source}]
```
Example: `[action: rss.accept via button]`

**Assistant message (role=assistant):**
The raw tool result content (same text sent to the channel).

This gives the LLM context if the user follows up conversationally (e.g., "why did you recommend that article?"). The synthetic messages use the same `session.add_message()` path as normal messages.

### Observability

Each dispatch execution logs at `info!` level:

```
info!("action dispatch: tool={} source={} channel={} chat_id={}", tool, source, channel, chat_id)
```

No new database table — dispatch events are observable through tracing logs and the synthetic session history messages. Token cost tracking is not applicable (no LLM tokens consumed).

### No LLM formatting

Tool results go directly to the channel. Tools already produce human-readable output. This is the fastest, cheapest path. If a specific tool ever needs LLM polish, a `needs_formatting: bool` field can be added to `ActionDispatchPayload` later (YAGNI for now).

## Source Integration

### Buttons (Slack)

In `handle_interactive_payload()`, after extracting `action_value` (the Slack button's `value` field, which carries `ButtonSpec.context`):

```rust
if let Ok(payload) = serde_json::from_str::<ActionDispatchPayload>(&action_value) {
    let dispatch = ActionDispatch {
        tool: payload.tool,
        params: payload.params,
        source: ActionSource::Button { action_id: action_id.to_string() },
    };
    // Build InboundMessage with .action(dispatch)
} else {
    // Legacy fallback: send as [button:{action_id}] text to LLM
}
```

Graceful degradation: if `action_value` doesn't parse as `ActionDispatchPayload`, the message falls through to the existing `[button:{action_id}]` text path.

### Buttons (Discord)

Discord buttons only carry `custom_id` (100 chars max) — there is no `value` field equivalent to Slack. A server-side dispatch context store bridges this gap.

**`DispatchContextStore`:** An in-memory LRU map (capacity 1000, TTL 15 minutes matching Discord's interaction token lifetime) keyed by button ID, valued by `ActionDispatchPayload`. Shared as `Arc<DispatchContextStore>` between the button rendering path and the click handler.

**On button render** (`parse_unified_buttons()` / outbound path): When `metadata["buttons"]` entries have a `context` field that parses as `ActionDispatchPayload`, store `button_id → payload` in the `DispatchContextStore`.

**On button click** (`handle_component()`): Look up `comp.data.custom_id` in the store. If found, create `ActionDispatch` from the stored payload. If not found (expired or unknown), fall through to the `[button:{custom_id}]` text path.

```rust
if let Some(payload) = dispatch_store.get(&custom_id) {
    let dispatch = ActionDispatch {
        tool: payload.tool.clone(),
        params: payload.params.clone(),
        source: ActionSource::Button { action_id: custom_id.clone() },
    };
    // Build InboundMessage with .action(dispatch)
} else {
    // Legacy fallback: [button:{custom_id}] to LLM
}
```

The store lives in the Discord channel handler struct as `dispatch_store: Arc<DispatchContextStore>`. No persistence needed — buttons older than 15 minutes can't be clicked anyway (Discord interaction tokens expire).

**Wiring the store to the outbound path:** `parse_unified_buttons()` and `parse_components_from_metadata()` are currently free functions. They gain an optional `dispatch_store: Option<&DispatchContextStore>` parameter. When provided, each button with a parseable `ActionDispatchPayload` context is stored. The Discord `send()` method (which has `&self` access to the handler struct) passes the store when calling these functions. Other callers (e.g., interaction followup JSON builders) pass `None`.

### Webhooks

When a webhook config includes a `dispatch` field, the webhook handler creates an `ActionDispatch` instead of templating a text message:

```json
{
  "webhooks": {
    "github-deploy": {
      "secret": "...",
      "dispatch": {
        "tool": "github",
        "params_template": {"action": "trigger_workflow", "workflow_id": "{{workflow}}"}
      },
      "targets": [{"channel": "slack", "chatId": "C123"}]
    }
  }
}
```

Template substitution fills in params, then direct dispatch. When `dispatch` is absent, existing behavior (template + text + targets) is unchanged.

**`params_template` substitution rules:** The `params_template` value is serialized to a JSON string, `{{key}}` placeholders are substituted using the same logic as the existing webhook `template` system (flat string replacement from the webhook JSON payload), then the result is parsed back to `serde_json::Value`. This reuses the existing substitution code path. Behavior for edge cases:
- Missing key in payload: `{{key}}` is left as a literal string (matches existing `template` behavior)
- Nested JSON values from payload: stringified when injected (since substitution operates on the serialized string)
- `{{body}}` is supported (raw payload body, same as `template`)
- Parse failure after substitution: webhook returns 400, action not dispatched

Webhook dispatch payloads are from external sources but go through:
- HMAC-SHA256 signature validation (existing)
- Replay protection via `X-Webhook-Timestamp` (existing)
- `LeakDetector::redact()` on rendered params (in `execute_action_dispatch()`)
- Tool's own input validation in `execute()`

### Cron follow-ups

When a tool result's metadata contains `follow_up_action`:

```json
{"follow_up_action": {"tool": "rss", "params": {"action": "next"}}}
```

The dispatch handler picks it up after tool execution and re-dispatches. This works in the action dispatch fast path (where follow-ups chain automatically) and during normal LLM-driven tool execution in the iteration loop.

**Behavior during LLM-driven iteration:** After each tool call's result is processed in `handle_tool_results()` (iteration.rs), check `result.metadata` for `follow_up_action`. If present, execute it immediately via `ToolRegistry::execute()` — do not route it through the LLM.

To maintain message structure integrity (required by `strip_orphaned_tool_messages()` during compaction), both a synthetic assistant message with a `tool_calls` entry AND the corresponding tool result message are injected into the conversation:

1. Generate a synthetic `tool_call_id` (e.g., `"dispatch-{uuid}"`)
2. Inject a synthetic assistant message containing a `tool_calls` array with one entry: `{ id: tool_call_id, name: dispatch.tool, arguments: dispatch.params }`
3. Execute the tool via `ToolRegistry::execute()`
4. Inject the tool result message with matching `tool_call_id`

The LLM sees both the original result and the follow-up result in its next iteration. Depth-limited to `MAX_DISPATCH_CHAIN_DEPTH` (3).

**Metadata collection:** The follow-up tool result's `metadata` (including `suggested_buttons`) must be pushed to `collected_tool_metadata` just like normal tool results. Otherwise buttons emitted by follow-up tools would be lost.

### Channel commands

**Deferred to a future spec.** Channel command parsing requires a grammar design (syntax, argument handling, tool name resolution, error messages) that is out of scope for this spec. The `ActionSource::Command` variant is defined for forward compatibility. When implemented, channel handlers will create `ActionDispatch` with `source: Command { raw }` and the message will bypass the LLM.

### Inter-tool chaining

Same mechanism as cron follow-ups — `follow_up_action` in `ToolResult.metadata`. The dispatch handler checks for it after every tool execution, both in the dispatch fast path and the LLM-driven iteration loop. Depth-limited to `MAX_DISPATCH_CHAIN_DEPTH` (3).

## Tool Migration

Every tool that emits `suggested_buttons` updates its context to the structured format. The change is mechanical — same data, different shape.

### RSS

Before:
```
"CALL rss tool with action=accept article_ids=[\"abc\"] THEN call rss action=next"
```

After:
```json
{"tool": "rss", "params": {"action": "accept", "article_ids": ["abc123"]}}
```

The "THEN next" part moves into the `accept` action's implementation — it returns the next article + new buttons as part of its response.

### Google Calendar

Before:
```json
{"tool": "google_calendar", "event_id": "...", "calendar_id": "...", "action": "rsvp_yes"}
```

After:
```json
{"tool": "google_calendar", "params": {"action": "rsvp", "event_id": "...", "calendar_id": "...", "response": "accepted"}}
```

### Other tools

Same pattern for google_mail, google_tasks, todoist, github, cron. Each tool's button builder updates to emit `{"tool": "<name>", "params": {...}}`.

### `ButtonSpec.context` remains `Option<String>`

The field type doesn't change. It's serialized as a JSON string for transport through Slack's `value` field and Discord's component data. The structured format is a contract on what that string contains.

## Error Handling

| Scenario | Behavior |
|----------|----------|
| Tool not found | Return outbound: "Action failed: tool '{name}' is not available." |
| Tool requires approval | Return outbound: "Action failed: tool '{name}' requires approval and cannot be auto-dispatched." |
| Tool execution error (`is_error`) | Return error content as outbound message |
| Malformed dispatch payload | Fall through to LLM path (graceful degradation) |
| Follow-up chain depth exceeded (>3) | Return last tool result as-is, log warning |
| Stale button (resource gone) | Tool handles it — returns `ToolResult::error(...)` |
| Concurrent button clicks | Session lock serializes within-session; cross-session concurrent |
| Cron dispatch | Same dispatch check in `process_direct_with_overrides()`. `IS_CRON_JOB` metadata carries through |
| Discord button expired (>15 min) | `DispatchContextStore` TTL miss → fall through to LLM path |
| Secrets in dispatch params | `LeakDetector::redact()` runs on serialized params before tool execution |

## Files Changed

### New
- `src/dispatch/mod.rs` — `ActionDispatch`, `ActionSource`, `ActionDispatchPayload`, `DispatchContextStore`, constants (`MAX_DISPATCH_CHAIN_DEPTH`, `FOLLOW_UP_ACTION`). Top-level module because these types are referenced across layers: `bus` (InboundMessage), `channels` (Slack/Discord), and `agent::loop` (processing).
- `src/agent/loop/dispatch.rs` — `execute_action_dispatch()`, follow-up chain logic, session history recording. Imports types from `crate::dispatch`.

### Modified
- `src/lib.rs` — add `pub mod dispatch;`
- `src/bus/events/mod.rs` — `InboundMessage` gets `#[serde(skip)] action: Option<ActionDispatch>`, builder gets `.action()`
- `src/agent/loop/processing.rs` — dispatch check in `process_message_unlocked()` and `process_direct_with_overrides()`
- `src/agent/loop/config.rs` — `AgentRunOverrides` gains `action: Option<ActionDispatch>` field
- `src/channels/slack/mod.rs` — `handle_interactive_payload()` deserializes context, populates `action` field
- `src/channels/discord/mod.rs` — `handle_component()` uses `DispatchContextStore` lookup; outbound button rendering stores payloads in store
- `src/gateway/mod.rs` — webhook handler gains optional dispatch path
- `src/config/schema/mod.rs` — `WebhookConfig` gets optional `dispatch` field
- `src/agent/tools/rss/articles.rs` — structured button context, `accept`/`reject` return next article inline
- `src/agent/tools/google_calendar/mod.rs` — structured button context
- `src/agent/tools/google_mail/mod.rs` — structured button context
- `src/agent/tools/google_tasks/mod.rs` — structured button context
- `src/agent/tools/todoist/mod.rs` — structured button context
- `src/agent/tools/github/mod.rs` — structured button context
- `src/agent/tools/cron/mod.rs` — structured button context
- `src/agent/loop/iteration.rs` — check `follow_up_action` in tool result metadata during normal LLM-driven execution (synthetic tool call injection)

### Unchanged
- `ButtonSpec` struct (context stays `Option<String>`)
- `add_buttons` tool (LLM-driven button creation still works)
- `merge_suggested_buttons()` (button merging unchanged for LLM paths)
- Tool `execute()` signatures (no trait changes)
- Channel button rendering for Slack (Block Kit builder unchanged)
- Telegram channel (no button support, unaffected)

## Testing Strategy

### Unit tests
- `ActionDispatchPayload` deserialization: valid JSON, missing fields, legacy format fallback
- Dispatch interception: mock tool registry, verify LLM never called when action present
- Follow-up chain: verify depth limit at `MAX_DISPATCH_CHAIN_DEPTH`, verify 1-level and 2-level chains work
- Follow-up chain depth exceeded: verify last result returned, warning logged
- Button context serialization: each tool produces valid `{"tool": "...", "params": {...}}`
- Source-specific parsing: Slack `action_value` → `ActionDispatch`, Discord store lookup → `ActionDispatch`, webhook → `ActionDispatch`
- Graceful degradation: malformed context falls through to LLM path
- Session history: synthetic user + assistant messages appended after dispatch
- `DispatchContextStore`: insert/get, TTL expiry, LRU eviction at capacity
- Approval-required tool blocked from dispatch
- Secret scanning on dispatch params (verify redaction applied)

### Integration tests
- End-to-end button dispatch: `InboundMessage` with `action` → `process_message()` → verify tool called with correct params → verify outbound has result + buttons
- Webhook dispatch: webhook payload with `dispatch` config → verify tool executes
- Error paths: unknown tool, tool execution failure, chain depth exceeded, approval-required tool
- Cron follow-up: tool returns `follow_up_action` → verify re-dispatch
- LLM-driven follow-up: during normal iteration, tool returns `follow_up_action` → verify synthetic tool call injected

### Unchanged coverage
- All tool unit tests (test `execute()` directly, unaffected)
- Session management integration tests
- Message flow integration tests
