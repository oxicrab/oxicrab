# Action Dispatch Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A generic action dispatch layer that intercepts structured tool call payloads before the LLM, executing them directly via `ToolRegistry`. Covers buttons, webhooks, cron follow-ups, and inter-tool chaining.

**Architecture:** New top-level `src/dispatch/mod.rs` module defines shared types (`ActionDispatch`, `ActionSource`, `ActionDispatchPayload`, `DispatchContextStore`). `src/agent/loop/dispatch.rs` contains the execution logic. Channel handlers (Slack, Discord) create `ActionDispatch` from button clicks. `process_message_unlocked()` short-circuits when `InboundMessage.action` is `Some`.

**Tech Stack:** Rust, serde_json, tokio, uuid (already a dependency). Existing ToolRegistry/LeakDetector/SessionStore infrastructure.

**Spec:** `docs/superpowers/specs/2026-03-16-action-dispatch-design.md`

---

## Chunk 1: Core Types and Bus Integration

### Task 1: Create `src/dispatch/mod.rs` with core types

**Files:**
- Create: `src/dispatch/mod.rs`
- Modify: `src/lib.rs:16-29` (add module declaration)

- [ ] **Step 1: Write tests for `ActionDispatchPayload` deserialization**

```rust
// In src/dispatch/mod.rs at bottom

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payload_deserialize_valid() {
        let json = r#"{"tool": "rss", "params": {"action": "accept", "article_ids": ["abc"]}}"#;
        let payload: ActionDispatchPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.tool, "rss");
        assert_eq!(payload.params["action"], "accept");
    }

    #[test]
    fn test_payload_deserialize_missing_params() {
        let json = r#"{"tool": "rss"}"#;
        let result = serde_json::from_str::<ActionDispatchPayload>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_payload_deserialize_missing_tool() {
        let json = r#"{"params": {"action": "accept"}}"#;
        let result = serde_json::from_str::<ActionDispatchPayload>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_payload_roundtrip() {
        let payload = ActionDispatchPayload {
            tool: "google_calendar".into(),
            params: serde_json::json!({"action": "rsvp", "event_id": "e123"}),
        };
        let serialized = serde_json::to_string(&payload).unwrap();
        let deserialized: ActionDispatchPayload = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.tool, "google_calendar");
        assert_eq!(deserialized.params["event_id"], "e123");
    }

    #[test]
    fn test_payload_without_params_field_fails() {
        // Missing required "params" field — serde rejects this
        let json = r#"{"tool": "google_calendar", "event_id": "e1", "action": "rsvp_yes"}"#;
        let result = serde_json::from_str::<ActionDispatchPayload>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_action_source_display() {
        assert_eq!(
            ActionSource::Button { action_id: "rss-accept-abc".into() }.source_label(),
            "button"
        );
        assert_eq!(
            ActionSource::Webhook { webhook_name: "gh".into() }.source_label(),
            "webhook"
        );
        assert_eq!(
            ActionSource::Cron { job_id: "j1".into() }.source_label(),
            "cron"
        );
        assert_eq!(
            ActionSource::ToolChain { parent_tool: "rss".into() }.source_label(),
            "chain"
        );
        assert_eq!(
            ActionSource::Command { raw: "/rss next".into() }.source_label(),
            "command"
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib dispatch::tests -- --test-threads=1`
Expected: FAIL — module doesn't exist yet

- [ ] **Step 3: Implement the dispatch types module**

```rust
// src/dispatch/mod.rs

use serde::{Deserialize, Serialize};

/// Maximum depth for follow-up action chains to prevent infinite loops.
pub const MAX_DISPATCH_CHAIN_DEPTH: usize = 3;

/// Metadata key for follow-up actions in ToolResult.metadata.
pub const FOLLOW_UP_ACTION: &str = "follow_up_action";

/// A structured tool call that bypasses the LLM entirely.
/// Not serialized — consumed in process_message_unlocked() before bus serialization.
#[derive(Debug, Clone)]
pub struct ActionDispatch {
    pub tool: String,
    pub params: serde_json::Value,
    pub source: ActionSource,
}

/// Provenance of an action dispatch.
#[derive(Debug, Clone)]
pub enum ActionSource {
    /// Slack/Discord button click
    Button { action_id: String },
    /// Named webhook with structured payload
    Webhook { webhook_name: String },
    /// Cron follow-up action
    Cron { job_id: String },
    /// Channel slash command — deferred to future spec
    Command { raw: String },
    /// Follow-up from another tool's result
    ToolChain { parent_tool: String },
}

impl ActionSource {
    /// Short label for logging and session history.
    pub fn source_label(&self) -> &'static str {
        match self {
            Self::Button { .. } => "button",
            Self::Webhook { .. } => "webhook",
            Self::Cron { .. } => "cron",
            Self::Command { .. } => "command",
            Self::ToolChain { .. } => "chain",
        }
    }
}

/// The serialized payload format stored in ButtonSpec.context and webhook dispatch configs.
/// All tools adopt this single schema for button contexts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionDispatchPayload {
    pub tool: String,
    pub params: serde_json::Value,
}
```

- [ ] **Step 4: Add module declaration to `src/lib.rs`**

Add `pub mod dispatch;` to the module declarations block in `src/lib.rs` (after the `pub mod config;` line, around line 22).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib dispatch::tests -- --test-threads=1`
Expected: PASS — all 6 tests

- [ ] **Step 6: Run clippy and fmt**

Run: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings`
Expected: No errors

- [ ] **Step 7: Commit**

```bash
git add src/dispatch/mod.rs src/lib.rs
git commit -m "feat(dispatch): add core action dispatch types

ActionDispatch, ActionSource, ActionDispatchPayload, and constants
for the generic action dispatch layer."
```

---

### Task 2: Add `DispatchContextStore` for Discord button context

**Files:**
- Modify: `src/dispatch/mod.rs`

- [ ] **Step 1: Write tests for DispatchContextStore**

Add to the `tests` module in `src/dispatch/mod.rs`:

```rust
#[test]
fn test_dispatch_store_insert_and_get() {
    let store = DispatchContextStore::new(100);
    let payload = ActionDispatchPayload {
        tool: "rss".into(),
        params: serde_json::json!({"action": "accept"}),
    };
    store.insert("btn-1".to_string(), payload.clone());
    let retrieved = store.get("btn-1");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().tool, "rss");
}

#[test]
fn test_dispatch_store_missing_key() {
    let store = DispatchContextStore::new(100);
    assert!(store.get("nonexistent").is_none());
}

#[test]
fn test_dispatch_store_capacity_eviction() {
    let store = DispatchContextStore::new(2);
    let p = |name: &str| ActionDispatchPayload {
        tool: name.into(),
        params: serde_json::json!({}),
    };
    store.insert("a".into(), p("tool_a"));
    store.insert("b".into(), p("tool_b"));
    store.insert("c".into(), p("tool_c")); // should evict "a"
    assert!(store.get("a").is_none());
    assert!(store.get("b").is_some());
    assert!(store.get("c").is_some());
}

#[test]
fn test_dispatch_store_ttl_expiry() {
    let store = DispatchContextStore::with_ttl(100, std::time::Duration::from_millis(50));
    let payload = ActionDispatchPayload {
        tool: "rss".into(),
        params: serde_json::json!({}),
    };
    store.insert("btn-1".into(), payload);
    assert!(store.get("btn-1").is_some());
    std::thread::sleep(std::time::Duration::from_millis(60));
    assert!(store.get("btn-1").is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib dispatch::tests -- --test-threads=1`
Expected: FAIL — `DispatchContextStore` not defined

- [ ] **Step 3: Implement DispatchContextStore**

Add to `src/dispatch/mod.rs` before the `#[cfg(test)]` block:

```rust
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default TTL for dispatch context entries (15 minutes, matching Discord interaction token lifetime).
const DEFAULT_DISPATCH_TTL: Duration = Duration::from_secs(15 * 60);

struct DispatchStoreInner {
    entries: HashMap<String, (ActionDispatchPayload, Instant)>,
    insertion_order: VecDeque<String>,
}

/// In-memory LRU store for button dispatch contexts.
/// Used by Discord (which can't carry JSON in button payloads) to map
/// button IDs back to their ActionDispatchPayload.
/// Single mutex protects both the map and insertion order for consistency.
pub struct DispatchContextStore {
    inner: Mutex<DispatchStoreInner>,
    capacity: usize,
    ttl: Duration,
}

impl DispatchContextStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(DispatchStoreInner {
                entries: HashMap::new(),
                insertion_order: VecDeque::new(),
            }),
            capacity,
            ttl: DEFAULT_DISPATCH_TTL,
        }
    }

    #[cfg(test)]
    pub fn with_ttl(capacity: usize, ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(DispatchStoreInner {
                entries: HashMap::new(),
                insertion_order: VecDeque::new(),
            }),
            capacity,
            ttl,
        }
    }

    pub fn insert(&self, key: String, payload: ActionDispatchPayload) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        // If key already exists, remove old entry from insertion_order
        if inner.entries.contains_key(&key) {
            inner.insertion_order.retain(|k| k != &key);
        }

        // Evict oldest if at capacity
        while inner.entries.len() >= self.capacity {
            if let Some(oldest) = inner.insertion_order.pop_front() {
                inner.entries.remove(&oldest);
            } else {
                break;
            }
        }

        inner.entries.insert(key.clone(), (payload, Instant::now()));
        inner.insertion_order.push_back(key);
    }

    pub fn get(&self, key: &str) -> Option<ActionDispatchPayload> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((payload, inserted_at)) = inner.entries.get(key) {
            if inserted_at.elapsed() > self.ttl {
                inner.entries.remove(key);
                return None;
            }
            Some(payload.clone())
        } else {
            None
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib dispatch::tests -- --test-threads=1`
Expected: PASS — all 10 tests

- [ ] **Step 5: Commit**

```bash
git add src/dispatch/mod.rs
git commit -m "feat(dispatch): add DispatchContextStore for Discord button context

In-memory LRU map with TTL, used to bridge Discord's lack of a button
value field. Keyed by button ID, stores ActionDispatchPayload."
```

---

### Task 3: Add `action` field to `InboundMessage` and `AgentRunOverrides`

**Files:**
- Modify: `src/bus/events/mod.rs:36-45` (InboundMessage struct)
- Modify: `src/bus/events/mod.rs:74-111` (InboundMessageBuilder)
- Modify: `src/agent/loop/config.rs:11-26` (AgentRunOverrides)

- [ ] **Step 1: Add `action` field to `InboundMessage`**

In `src/bus/events/mod.rs`, add to the `InboundMessage` struct (after the `metadata` field, around line 43):

```rust
    #[serde(skip)]
    pub action: Option<crate::dispatch::ActionDispatch>,
```

- [ ] **Step 2: Add `.action()` builder method to `InboundMessageBuilder`**

In `src/bus/events/mod.rs`, add a new method to `InboundMessageBuilder` (after the `is_group` method, around line 105):

```rust
    pub fn action(mut self, dispatch: crate::dispatch::ActionDispatch) -> Self {
        self.inner.action = Some(dispatch);
        self
    }
```

- [ ] **Step 3: Add `action` field to `AgentRunOverrides`**

In `src/agent/loop/config.rs`, add to the `AgentRunOverrides` struct (after the `metadata` field, around line 25):

```rust
    /// Structured action dispatch — bypasses LLM when Some.
    pub action: Option<crate::dispatch::ActionDispatch>,
```

Update any place that constructs `AgentRunOverrides` with `..Default::default()` or explicit fields to include `action: None`. Search for all construction sites:

Run: `grep -rn "AgentRunOverrides" src/ --include="*.rs"` to find all sites.

Each construction site needs `action: None` added. Common sites:
- `src/agent/loop/processing.rs` (effective_overrides construction)
- `src/cli/commands/gateway_setup.rs` (cron execution overrides)
- `tests/common/mod.rs` (test helpers)

If `AgentRunOverrides` derives `Default`, then sites using `..Default::default()` or `Default::default()` won't need changes.

- [ ] **Step 4: Update `AgentRunOverrides` construction sites**

`AgentRunOverrides` does not derive `Default` — it is constructed with explicit fields. Search for all construction sites:

Run: `grep -rn "AgentRunOverrides" src/ tests/ --include="*.rs"`

Each construction site needs `action: None` added. Key sites include:
- `src/agent/loop/processing.rs` (effective_overrides construction)
- `src/cli/commands/gateway_setup.rs` (cron execution overrides)
- `tests/common/mod.rs` (test helpers)

Add `action: None` to each. If `AgentRunOverrides` does derive `Default`, then sites using `..Default::default()` won't need changes — but still check.

- [ ] **Step 5: Run full test suite**

Run: `cargo test --lib -- --test-threads=1`
Expected: PASS — the new fields are `Option` with `None` default, should be backward compatible

- [ ] **Step 6: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No errors

- [ ] **Step 7: Commit**

```bash
git add src/bus/events/mod.rs src/agent/loop/config.rs
git commit -m "feat(dispatch): add action field to InboundMessage and AgentRunOverrides

InboundMessage.action is #[serde(skip)] — consumed before bus
serialization. AgentRunOverrides.action carries dispatch through
process_direct_with_overrides() for cron/subagent paths."
```

---

## Chunk 2: Dispatch Execution and Processing Pipeline

### Task 4: Implement `execute_action_dispatch()` in `src/agent/loop/dispatch.rs`

**Files:**
- Create: `src/agent/loop/dispatch.rs`
- Modify: `src/agent/loop/mod.rs` (add `mod dispatch;` declaration)

- [ ] **Step 1: Write tests for dispatch execution**

```rust
// src/agent/loop/dispatch.rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::{ActionDispatch, ActionSource};

    fn mock_dispatch(tool: &str, params: serde_json::Value) -> ActionDispatch {
        ActionDispatch {
            tool: tool.to_string(),
            params,
            source: ActionSource::Button {
                action_id: "test-btn".to_string(),
            },
        }
    }

    #[test]
    fn test_format_session_user_message() {
        let dispatch = mock_dispatch("rss", serde_json::json!({"action": "accept"}));
        let msg = format_dispatch_user_message(&dispatch);
        assert_eq!(msg, "[action: rss.accept via button]");
    }

    #[test]
    fn test_format_session_user_message_no_action() {
        let dispatch = mock_dispatch("shell", serde_json::json!({"command": "ls"}));
        let msg = format_dispatch_user_message(&dispatch);
        assert_eq!(msg, "[action: shell via button]");
    }

    #[test]
    fn test_extract_buttons_from_metadata() {
        let mut meta = std::collections::HashMap::new();
        meta.insert(
            "suggested_buttons".to_string(),
            serde_json::json!([
                {"id": "btn-1", "label": "OK", "style": "primary"}
            ]),
        );
        let buttons = extract_buttons_from_tool_metadata(&[meta]);
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0]["id"], "btn-1");
    }

    #[test]
    fn test_extract_follow_up_action() {
        let mut meta = std::collections::HashMap::new();
        meta.insert(
            crate::dispatch::FOLLOW_UP_ACTION.to_string(),
            serde_json::json!({"tool": "rss", "params": {"action": "next"}}),
        );
        let follow_up = extract_follow_up(&meta);
        assert!(follow_up.is_some());
        let payload = follow_up.unwrap();
        assert_eq!(payload.tool, "rss");
        assert_eq!(payload.params["action"], "next");
    }

    #[test]
    fn test_extract_follow_up_action_missing() {
        let meta = std::collections::HashMap::new();
        let follow_up = extract_follow_up(&meta);
        assert!(follow_up.is_none());
    }

    #[test]
    fn test_extract_follow_up_action_malformed() {
        let mut meta = std::collections::HashMap::new();
        meta.insert(
            crate::dispatch::FOLLOW_UP_ACTION.to_string(),
            serde_json::json!("not an object"),
        );
        let follow_up = extract_follow_up(&meta);
        assert!(follow_up.is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib agent::loop::dispatch::tests -- --test-threads=1`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement dispatch helper functions**

```rust
// src/agent/loop/dispatch.rs

use std::collections::HashMap;

use tracing::{info, warn};

use crate::dispatch::{ActionDispatch, ActionDispatchPayload, FOLLOW_UP_ACTION};

/// Format the synthetic user message for session history.
/// e.g., "[action: rss.accept via button]"
pub(super) fn format_dispatch_user_message(dispatch: &ActionDispatch) -> String {
    let action = dispatch.params.get("action").and_then(|v| v.as_str());
    let source = dispatch.source.source_label();
    if let Some(action) = action {
        format!("[action: {}.{} via {}]", dispatch.tool, action, source)
    } else {
        format!("[action: {} via {}]", dispatch.tool, source)
    }
}

/// Extract suggested_buttons from collected tool metadata.
pub(super) fn extract_buttons_from_tool_metadata(
    collected: &[HashMap<String, serde_json::Value>],
) -> Vec<serde_json::Value> {
    collected
        .iter()
        .filter_map(|meta| meta.get("suggested_buttons")?.as_array())
        .flatten()
        .cloned()
        .collect()
}

/// Extract a follow-up action from tool result metadata.
pub(super) fn extract_follow_up(
    metadata: &HashMap<String, serde_json::Value>,
) -> Option<ActionDispatchPayload> {
    let value = metadata.get(FOLLOW_UP_ACTION)?;
    serde_json::from_value::<ActionDispatchPayload>(value.clone()).ok()
}
```

- [ ] **Step 4: Add `mod dispatch;` to `src/agent/loop/mod.rs`**

Add `mod dispatch;` to the module declarations in `src/agent/loop/mod.rs` (near the top, alongside other `mod` declarations like `mod processing;`, `mod iteration;`, etc.).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib agent::loop::dispatch::tests -- --test-threads=1`
Expected: PASS — all 6 tests

- [ ] **Step 6: Commit**

```bash
git add src/agent/loop/dispatch.rs src/agent/loop/mod.rs
git commit -m "feat(dispatch): add dispatch execution helpers

format_dispatch_user_message(), extract_buttons_from_tool_metadata(),
extract_follow_up() — building blocks for the dispatch pipeline."
```

---

### Task 5: Implement dispatch interception in `process_message_unlocked()`

**Files:**
- Modify: `src/agent/loop/processing.rs:17-66` (process_message_unlocked)
- Modify: `src/agent/loop/dispatch.rs` (add execute_action_dispatch)

- [ ] **Step 1: Add `execute_action_dispatch` method to AgentLoop**

In `src/agent/loop/dispatch.rs`, add the main dispatch execution method. This is an `impl AgentLoop` block:

```rust
use crate::agent::tools::base::{ExecutionContext, ToolResult};
use crate::bus::events::OutboundMessage;
use crate::bus::meta;
use crate::dispatch::MAX_DISPATCH_CHAIN_DEPTH;

impl super::AgentLoop {
    /// Execute an action dispatch, bypassing the LLM entirely.
    /// Returns the OutboundMessage to send to the channel.
    pub(super) async fn execute_action_dispatch(
        &self,
        dispatch: ActionDispatch,
        channel: &str,
        chat_id: &str,
        session_key: &str,
        inbound_metadata: &HashMap<String, serde_json::Value>,
    ) -> anyhow::Result<OutboundMessage> {
        info!(
            "action dispatch: tool={} source={} channel={} chat_id={}",
            dispatch.tool,
            dispatch.source.source_label(),
            channel,
            chat_id
        );

        // Build execution context
        let ctx = ExecutionContext {
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            context_summary: None,
            metadata: inbound_metadata.clone(),
        };

        // Execute with chaining support
        let (final_content, all_buttons) =
            self.execute_dispatch_chain(&dispatch, &ctx, 0).await?;

        // Record in session history
        // Session::add_message takes (role, content, extra_metadata)
        let user_msg = format_dispatch_user_message(&dispatch);
        if let Ok(mut session) = self.sessions.get_or_create(session_key).await {
            session.add_message("user", &user_msg, HashMap::new());
            session.add_message("assistant", &final_content, HashMap::new());
            let _ = self.sessions.save(&session).await;
        }

        // Build outbound message
        let mut metadata = HashMap::new();
        if !all_buttons.is_empty() {
            metadata.insert(
                meta::BUTTONS.to_string(),
                serde_json::Value::Array(all_buttons),
            );
        }

        Ok(OutboundMessage {
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            content: final_content,
            reply_to: inbound_metadata
                .get(meta::TS)
                .and_then(|v| v.as_str())
                .map(String::from),
            media: Vec::new(),
            metadata,
        })
    }

    /// Execute a single dispatch, chaining follow-up actions up to MAX_DISPATCH_CHAIN_DEPTH.
    async fn execute_dispatch_chain(
        &self,
        dispatch: &ActionDispatch,
        ctx: &ExecutionContext,
        depth: usize,
    ) -> anyhow::Result<(String, Vec<serde_json::Value>)> {
        // Validate tool exists
        let tool = self
            .tools
            .get(&dispatch.tool)
            .ok_or_else(|| anyhow::anyhow!("tool '{}' not found", dispatch.tool))?;

        // Check approval requirement
        if tool.requires_approval() {
            return Ok((
                format!(
                    "Action failed: tool '{}' requires approval and cannot be auto-dispatched.",
                    dispatch.tool
                ),
                Vec::new(),
            ));
        }

        // Secret-scan dispatch params
        let params_str = serde_json::to_string(&dispatch.params).unwrap_or_default();
        let redacted_str = self.leak_detector.redact(&params_str);
        let params = if redacted_str != params_str {
            warn!("action dispatch: secrets redacted from dispatch params");
            serde_json::from_str(&redacted_str).unwrap_or(dispatch.params.clone())
        } else {
            dispatch.params.clone()
        };

        // Execute tool
        let result = self.tools.execute(&dispatch.tool, params, ctx).await?;

        // Collect metadata
        let mut collected_meta = Vec::new();
        if let Some(meta) = &result.metadata {
            collected_meta.push(meta.clone());
        }

        let buttons = extract_buttons_from_tool_metadata(&collected_meta);
        let mut content = result.content.clone();

        // Check for follow-up action
        if let Some(meta) = &result.metadata {
            if let Some(follow_up) = extract_follow_up(meta) {
                if depth + 1 >= MAX_DISPATCH_CHAIN_DEPTH {
                    warn!(
                        "action dispatch: chain depth limit reached ({}), stopping",
                        MAX_DISPATCH_CHAIN_DEPTH
                    );
                } else {
                    let chained = ActionDispatch {
                        tool: follow_up.tool,
                        params: follow_up.params,
                        source: crate::dispatch::ActionSource::ToolChain {
                            parent_tool: dispatch.tool.clone(),
                        },
                    };
                    let (chain_content, chain_buttons) =
                        self.execute_dispatch_chain(&chained, ctx, depth + 1).await?;
                    content = format!("{}\n\n{}", content, chain_content);
                    // Chain buttons replace parent buttons (latest wins)
                    if !chain_buttons.is_empty() {
                        return Ok((content, chain_buttons));
                    }
                }
            }
        }

        Ok((content, buttons))
    }
}
```

- [ ] **Step 2: Add dispatch interception to `process_message_unlocked()`**

In `src/agent/loop/processing.rs`, after the system message check (around line 21, after `if msg.channel == "system" { return ... }`) and before the typing indicator, add:

```rust
        // Action dispatch: bypass LLM for structured tool calls
        if let Some(dispatch) = msg.action {
            let result = self
                .execute_action_dispatch(
                    dispatch,
                    &msg.channel,
                    &msg.chat_id,
                    &msg.session_key(),
                    &msg.metadata,
                )
                .await;
            return match result {
                Ok(outbound) => Ok(Some(outbound)),
                Err(e) => {
                    warn!("action dispatch failed: {e}");
                    Ok(Some(OutboundMessage {
                        channel: msg.channel.clone(),
                        chat_id: msg.chat_id.clone(),
                        content: format!("Action failed: {e}"),
                        reply_to: msg.metadata.get(crate::bus::meta::TS)
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        media: Vec::new(),
                        metadata: HashMap::new(),
                    }))
                }
            };
        }
```

- [ ] **Step 3: Add dispatch interception to `process_direct_with_overrides()`**

In `src/agent/loop/processing.rs`, in `process_direct_with_overrides()`, after the session lock is acquired and after secret scanning / prompt guard (around line 783), before session lookup, add:

```rust
        // Action dispatch: bypass LLM for structured tool calls
        if let Some(dispatch) = overrides.action.clone() {
            let result = self
                .execute_action_dispatch(
                    dispatch,
                    channel,
                    chat_id,
                    session_key,
                    &overrides.metadata,
                )
                .await;
            return match result {
                Ok(outbound) => Ok(super::config::DirectResult {
                    content: outbound.content,
                    metadata: outbound.metadata,
                }),
                Err(e) => {
                    warn!("action dispatch failed: {e}");
                    Ok(super::config::DirectResult {
                        content: format!("Action failed: {e}"),
                        metadata: HashMap::new(),
                    })
                }
            };
        }
```

- [ ] **Step 4: Run full test suite**

Run: `cargo test -- --test-threads=1`
Expected: PASS — existing tests unaffected (action is always None)

- [ ] **Step 5: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No errors

- [ ] **Step 6: Commit**

```bash
git add src/agent/loop/dispatch.rs src/agent/loop/processing.rs
git commit -m "feat(dispatch): implement dispatch execution and pipeline interception

execute_action_dispatch() handles tool execution, chaining, session
history, and secret scanning. Interception in process_message_unlocked()
and process_direct_with_overrides() short-circuits before the LLM."
```

---

## Chunk 3: Channel Integration (Slack + Discord)

### Task 6: Integrate dispatch into Slack button handler

**Files:**
- Modify: `src/channels/slack/mod.rs:1247-1265` (handle_interactive_payload content construction)

- [ ] **Step 1: Modify `handle_interactive_payload()` to create ActionDispatch**

In `src/channels/slack/mod.rs`, find the section in `handle_interactive_payload()` where the inbound message content is constructed (around lines 1247-1265). Replace:

```rust
    let content = if action_value.is_empty() {
        format!("[button:{action_id}]")
    } else {
        format!("[button:{action_id}]\nButton context: {action_value}")
    };
```

With:

```rust
    // Try to parse button context as ActionDispatchPayload for direct dispatch
    let (content, dispatch) = if !action_value.is_empty() {
        if let Ok(payload) =
            serde_json::from_str::<crate::dispatch::ActionDispatchPayload>(&action_value)
        {
            let dispatch = crate::dispatch::ActionDispatch {
                tool: payload.tool,
                params: payload.params,
                source: crate::dispatch::ActionSource::Button {
                    action_id: action_id.to_string(),
                },
            };
            // Keep content for logging/debugging
            (format!("[button:{action_id}]"), Some(dispatch))
        } else {
            // Legacy fallback: send as text to LLM
            (
                format!("[button:{action_id}]\nButton context: {action_value}"),
                None,
            )
        }
    } else {
        (format!("[button:{action_id}]"), None)
    };
```

Then find the `InboundMessage::builder(...)` call and add `.action()` when dispatch is present. After the existing builder chain that ends with `.build()`, modify to conditionally add the action:

```rust
    let mut builder = InboundMessage::builder(
        "slack",
        user_id.to_string(),
        channel_id.to_string(),
        content,
    )
    .meta("user_id", Value::String(user_id.to_string()))
    .meta("action_id", Value::String(action_id.to_string()))
    .is_group(!is_dm);

    if !action_value.is_empty() {
        builder = builder.meta("button_context", Value::String(action_value.to_string()));
    }
    if !message_ts.is_empty() {
        builder = builder.meta(crate::bus::meta::TS, Value::String(message_ts.to_string()));
    }
    if let Some(dispatch) = dispatch {
        builder = builder.action(dispatch);
    }

    let inbound_msg = builder.build();
```

- [ ] **Step 2: Run full test suite**

Run: `cargo test -- --test-threads=1`
Expected: PASS

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add src/channels/slack/mod.rs
git commit -m "feat(dispatch): integrate action dispatch into Slack button handler

Slack handle_interactive_payload() now parses button context as
ActionDispatchPayload. When valid, creates ActionDispatch on
InboundMessage for direct tool execution. Falls back to LLM path
for legacy button formats."
```

---

### Task 7: Integrate dispatch into Discord button handler with DispatchContextStore

**Files:**
- Modify: `src/channels/discord/mod.rs:28-35` (Handler struct — add dispatch_store field)
- Modify: `src/channels/discord/mod.rs:137-226` (handle_component — store lookup)
- Modify: `src/channels/discord/mod.rs:537-562` (parse_unified_buttons — store insertion)
- Modify: `src/channels/discord/mod.rs:821` (send — pass store to parse functions)

- [ ] **Step 1: Add `dispatch_store` field to Discord Handler struct**

In `src/channels/discord/mod.rs`, add to the `Handler` struct (around line 28):

```rust
    dispatch_store: std::sync::Arc<crate::dispatch::DispatchContextStore>,
```

- [ ] **Step 2: Initialize the store in the handler constructor**

Find where `Handler` is constructed (in the `start()` function or similar) and add:

```rust
    dispatch_store: std::sync::Arc::new(crate::dispatch::DispatchContextStore::new(1000)),
```

- [ ] **Step 3: Modify `parse_unified_buttons()` to accept optional store**

Change the signature of `parse_unified_buttons()` from:

```rust
fn parse_unified_buttons(metadata: &HashMap<String, serde_json::Value>) -> Vec<CreateActionRow>
```

To:

```rust
fn parse_unified_buttons(
    metadata: &HashMap<String, serde_json::Value>,
    dispatch_store: Option<&crate::dispatch::DispatchContextStore>,
) -> Vec<CreateActionRow>
```

Inside the function, after creating each button, if `dispatch_store` is provided, try to store the context:

```rust
    let btns: Vec<CreateButton> = buttons_arr
        .iter()
        .filter_map(|b| {
            let id = b["id"].as_str()?;
            let label = b["label"].as_str().unwrap_or(id);
            let style = parse_button_style(b["style"].as_str().unwrap_or("secondary"));

            // Store dispatch context if available
            if let Some(store) = dispatch_store {
                if let Some(ctx) = b["context"].as_str() {
                    if let Ok(payload) =
                        serde_json::from_str::<crate::dispatch::ActionDispatchPayload>(ctx)
                    {
                        store.insert(id.to_string(), payload);
                    }
                }
            }

            Some(CreateButton::new(id).label(label).style(style))
        })
        .collect();
```

- [ ] **Step 4: Update `parse_components_from_metadata()` to pass store**

Change its signature similarly to accept `dispatch_store: Option<&crate::dispatch::DispatchContextStore>`, and pass it through to the `parse_unified_buttons()` call at the end.

- [ ] **Step 5: Update `send()` and `send_interaction_followup()` callers**

In `send()` (line 821), change:
```rust
let components = parse_components_from_metadata(&msg.metadata);
```
To:
```rust
let components = parse_components_from_metadata(&msg.metadata, Some(&self.dispatch_store));
```

In `send_interaction_followup()` (line 1021), change similarly:
```rust
let components = parse_components_from_metadata(&msg.metadata, Some(&self.dispatch_store));
```

For `components_to_api_json()` if it also calls parse functions, update it too. If it doesn't use the unified path, pass `None`.

- [ ] **Step 6: Modify `handle_component()` to look up dispatch store**

In `handle_component()`, after extracting `custom_id` (around line 191), before building the content string, add the store lookup:

```rust
    let custom_id = comp.data.custom_id.clone();

    // Try dispatch store lookup (for buttons with structured context)
    let dispatch = self.dispatch_store.get(&custom_id).map(|payload| {
        crate::dispatch::ActionDispatch {
            tool: payload.tool,
            params: payload.params,
            source: crate::dispatch::ActionSource::Button {
                action_id: custom_id.clone(),
            },
        }
    });

    let content = format!("[button:{custom_id}]");
```

Then when building the `InboundMessage`, add the dispatch:

```rust
    let mut builder = InboundMessage::builder("discord", sender_id, comp.channel_id.to_string(), content)
        .metadata(metadata);

    if let Some(d) = dispatch {
        builder = builder.action(d);
    }

    let inbound_msg = builder.build();
```

- [ ] **Step 7: Run full test suite**

Run: `cargo test -- --test-threads=1`
Expected: PASS

- [ ] **Step 8: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No errors

- [ ] **Step 9: Commit**

```bash
git add src/channels/discord/mod.rs
git commit -m "feat(dispatch): integrate action dispatch into Discord with DispatchContextStore

Discord handler stores button dispatch payloads on render, looks them
up on click. Bridges Discord's lack of a button value field."
```

---

## Chunk 4: Webhook Dispatch and Follow-Up Actions

### Task 8: Add webhook dispatch config and handler

**Files:**
- Modify: `src/config/schema/mod.rs:188-221` (WebhookConfig struct)
- Modify: `src/gateway/mod.rs` (webhook handler)

- [ ] **Step 1: Add `dispatch` field to `WebhookConfig`**

In `src/config/schema/mod.rs`, add to the `WebhookConfig` struct (after the `agent_turn` field):

```rust
    /// Structured dispatch configuration for direct tool execution.
    /// When present, webhook payloads create an ActionDispatch instead of templated text.
    #[serde(default)]
    pub dispatch: Option<WebhookDispatchConfig>,
```

Add the new config struct nearby:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDispatchConfig {
    /// Tool name to dispatch to.
    pub tool: String,
    /// Template for tool params. Use `{{key}}` for JSON payload substitution.
    pub params_template: serde_json::Value,
}
```

Update the `Default` impl for `WebhookConfig` to include `dispatch: None`.

- [ ] **Step 2: Modify webhook handler to support dispatch**

In `src/gateway/mod.rs`, find the webhook handler function (the one that processes incoming webhook requests). After template substitution but before routing (the section that checks `agent_turn`), add a dispatch path:

```rust
    // Check for structured dispatch
    if let Some(ref dispatch_config) = webhook_config.dispatch {
        // Serialize params_template, substitute {{key}} placeholders, parse back
        // apply_template(template, body_str, json) — body_str is 2nd arg, json is 3rd
        let template_str = serde_json::to_string(&dispatch_config.params_template)
            .unwrap_or_default();
        let rendered = apply_template(&template_str, &body_str, json_value.as_ref());
        let params: serde_json::Value = match serde_json::from_str(&rendered) {
            Ok(v) => v,
            Err(e) => {
                warn!("webhook dispatch: params template parse failure: {e}");
                return (axum::http::StatusCode::BAD_REQUEST, "dispatch params parse failure").into_response();
            }
        };

        let dispatch = crate::dispatch::ActionDispatch {
            tool: dispatch_config.tool.clone(),
            params,
            source: crate::dispatch::ActionSource::Webhook {
                webhook_name: webhook_name.to_string(),
            },
        };

        // Route through agent loop via inbound_tx — same pattern as agent_turn webhooks.
        // The InboundMessage carries the dispatch; the agent loop short-circuits.
        for target in &webhook_config.targets {
            let inbound = crate::bus::events::InboundMessage::builder(
                &target.channel,
                "webhook".to_string(),
                &target.chat_id,
                format!("[webhook-dispatch:{}]", webhook_name),
            )
            .action(dispatch.clone())
            .build();

            if let Err(e) = inbound_tx.send(inbound).await {
                warn!("webhook dispatch: failed to send to agent loop: {e}");
            }
        }
        return (axum::http::StatusCode::OK, "dispatched").into_response();
    }
```

Note: `inbound_tx` is the `mpsc::Sender<InboundMessage>` available in the webhook handler scope. The existing `agent_turn: true` path already uses `inbound_tx` to route webhook messages through the agent loop — follow that same pattern. The `ActionDispatch` on the `InboundMessage` causes `process_message_unlocked()` to short-circuit before the LLM.

- [ ] **Step 3: Verify `apply_template` function exists and handles `{{key}}` substitution**

The webhook handler should already have a template substitution function. Find it and confirm it handles `{{key}}` and `{{body}}` substitution. Use the same function for `params_template`.

- [ ] **Step 4: Run full test suite**

Run: `cargo test -- --test-threads=1`
Expected: PASS

- [ ] **Step 5: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No errors

- [ ] **Step 6: Commit**

```bash
git add src/config/schema/mod.rs src/gateway/mod.rs
git commit -m "feat(dispatch): add webhook dispatch config and handler

WebhookConfig gains optional dispatch field with tool + params_template.
Webhook handler creates ActionDispatch and routes through agent loop
for direct tool execution, bypassing LLM."
```

---

### Task 9: Implement follow-up action handling in LLM-driven iteration loop

**Files:**
- Modify: `src/agent/loop/iteration.rs:477-480` (after metadata collection in handle_tool_results)

- [ ] **Step 1: Add follow-up action check after tool result metadata collection**

In `src/agent/loop/iteration.rs`, in `handle_tool_results()`, after the metadata collection block (around line 480, after `collected_tool_metadata.push(meta);`), add follow-up action handling.

Note: `handle_tool_results()` does not currently receive an `ExecutionContext`. The implementer must either:
- (a) Add a `ctx: &ExecutionContext` parameter to `handle_tool_results()` and thread it from the caller, or
- (b) Build a minimal `ExecutionContext` from `self` fields (channel/chat_id from the messages or from AgentLoop fields)

Option (a) is cleaner. The caller in the iteration loop already has access to the execution context.

```rust
                // Check for follow-up action in tool result metadata
                if let Some(ref meta) = result.metadata {
                    if let Some(follow_up) =
                        super::dispatch::extract_follow_up(meta)
                    {
                        use crate::dispatch::MAX_DISPATCH_CHAIN_DEPTH;

                        // Depth tracking: use a simple counter.
                        // For the iteration loop, a single follow-up is typical.
                        // Depth > MAX_DISPATCH_CHAIN_DEPTH is rejected.
                        let follow_up_id = format!("dispatch-{}", uuid::Uuid::new_v4());
                        info!(
                            "follow-up action: tool={} from parent tool call",
                            follow_up.tool
                        );

                        // Execute follow-up directly (no LLM)
                        let follow_up_result = match self
                            .tools
                            .execute(&follow_up.tool, follow_up.params.clone(), ctx)
                            .await
                        {
                            Ok(r) => r,
                            Err(e) => {
                                warn!("follow-up action failed: {e}");
                                ToolResult::error(format!("Follow-up action failed: {e}"))
                            }
                        };

                        // Collect follow-up metadata (including suggested_buttons)
                        if let Some(follow_meta) = &follow_up_result.metadata {
                            collected_tool_metadata.push(follow_meta.clone());
                        }

                        // Inject synthetic assistant tool_call + tool result
                        // Message::assistant(content, tool_calls) — content empty, tool_calls has our synthetic call
                        // Message::tool_result(tool_call_id, content, is_error) — 3 args
                        let synthetic_tc = crate::providers::base::ToolCallRequest {
                            id: follow_up_id.clone(),
                            name: follow_up.tool.clone(),
                            arguments: follow_up.params,
                        };
                        messages.push(crate::providers::base::Message::assistant(
                            "",
                            Some(vec![synthetic_tc]),
                        ));
                        messages.push(crate::providers::base::Message::tool_result(
                            &follow_up_id,
                            &follow_up_result.content,
                            follow_up_result.is_error,
                        ));

                        // Extract media from follow-up result
                        let follow_up_media =
                            crate::agent::loop::helpers::extract_media_paths(
                                &follow_up_result.content,
                            );
                        collected_media.extend(follow_up_media);
                    }
                }
```

- [ ] **Step 2: Add necessary imports**

At the top of `iteration.rs`, ensure these are imported (some may already be present — check first):

```rust
use tracing::{info, warn};
use uuid::Uuid;
```

`ExecutionContext`, `ToolResult`, and `Message` are likely already imported. Verify.

- [ ] **Step 3: Run full test suite**

Run: `cargo test -- --test-threads=1`
Expected: PASS

- [ ] **Step 4: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No errors

- [ ] **Step 5: Commit**

```bash
git add src/agent/loop/iteration.rs
git commit -m "feat(dispatch): add follow-up action handling in LLM-driven iteration

After tool execution, check result.metadata for follow_up_action.
Execute follow-up directly via ToolRegistry, inject synthetic
assistant tool_call + tool result messages for compaction safety."
```

---

## Chunk 5: Tool Migrations

### Task 10: Migrate RSS tool to structured button context

**Files:**
- Modify: `src/agent/tools/rss/articles.rs:214-221,278-293` (button context format)
- Modify: `src/agent/tools/rss/articles.rs` (accept/reject to return next article inline)

- [ ] **Step 1: Update `handle_next` button context to structured format**

In `src/agent/tools/rss/articles.rs`, find the accept/reject button context construction (around lines 278-293). Replace:

```rust
let accept_ctx = format!(
    "CALL rss tool with action=accept article_ids=[\"{short_id}\"] THEN call rss action=next"
);
let reject_ctx = format!(
    "CALL rss tool with action=reject article_ids=[\"{short_id}\"] THEN call rss action=next"
);
```

With:

```rust
let accept_ctx = serde_json::json!({
    "tool": "rss",
    "params": {"action": "accept", "article_ids": [&short_id]}
}).to_string();
let reject_ctx = serde_json::json!({
    "tool": "rss",
    "params": {"action": "reject", "article_ids": [&short_id]}
}).to_string();
```

Also update the feedback result buttons (around lines 214-221) — the "Next Article" and "Done Reviewing" buttons:

```rust
let next_ctx = serde_json::json!({
    "tool": "rss",
    "params": {"action": "next"}
}).to_string();
```

Replace any free-text context strings with the structured `{"tool": "rss", "params": {...}}` format.

- [ ] **Step 2: Make accept/reject return next article inline**

In the `handle_feedback()` function (which handles both accept and reject), after processing the feedback, add a call to fetch and return the next article. This replaces the "THEN call rss action=next" chain:

After the existing feedback processing (model update, DB writes), add:

```rust
    // Return next article inline (replaces the old "THEN next" chaining)
    let next_result = handle_next(db, model_data, profile, limit).await?;
    let combined = format!("{}\n\n{}", feedback_summary, next_result.content);

    // Carry over suggested_buttons from the next article
    let mut result = ToolResult::new(combined);
    if let Some(meta) = next_result.metadata {
        result = result.with_metadata(meta.into_iter().collect());
    }
    Ok(result)
```

The exact integration depends on the function signatures. The key change: `handle_feedback()` now calls `handle_next()` internally and concatenates the results.

- [ ] **Step 3: Run tests**

Run: `cargo test --lib rss -- --test-threads=1`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/agent/tools/rss/articles.rs
git commit -m "feat(dispatch): migrate RSS buttons to structured context format

Button contexts use ActionDispatchPayload JSON. Accept/reject now
return the next article inline, removing the need for chained actions."
```

---

### Task 11: Migrate Google Calendar buttons to structured context

**Files:**
- Modify: `src/agent/tools/google_calendar/mod.rs:538-594` (button context format)

- [ ] **Step 1: Update button context format**

In `src/agent/tools/google_calendar/mod.rs`, find `build_event_buttons()`. The current format is:

```rust
"context": serde_json::json!({
    "tool": "google_calendar",
    "event_id": event_id,
    "calendar_id": calendar_id,
    "action": "rsvp_yes"
}).to_string()
```

Change each button's context to the `ActionDispatchPayload` format:

```rust
"context": serde_json::json!({
    "tool": "google_calendar",
    "params": {
        "action": "rsvp",
        "event_id": event_id,
        "calendar_id": calendar_id,
        "response": "accepted"
    }
}).to_string()
```

Apply to all button variants:
- RSVP Yes: `"params": {"action": "rsvp", "event_id": ..., "calendar_id": ..., "response": "accepted"}`
- RSVP No: `"params": {"action": "rsvp", "event_id": ..., "calendar_id": ..., "response": "declined"}`
- Delete: `"params": {"action": "delete_event", "event_id": ..., "calendar_id": ...}`

Verify the action names match the tool's actual action dispatch (check the `execute()` match arms).

- [ ] **Step 2: Run tests**

Run: `cargo test --lib google_calendar -- --test-threads=1`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/agent/tools/google_calendar/mod.rs
git commit -m "feat(dispatch): migrate Google Calendar buttons to structured context"
```

---

### Task 12: Migrate Google Mail buttons to structured context

**Files:**
- Modify: `src/agent/tools/google_mail/mod.rs:370-415` (button context format)

- [ ] **Step 1: Update button context format**

Current:
```rust
"context": serde_json::json!({
    "tool": "google_mail",
    "message_id": msg_id,
    "action": "read"
}).to_string()
```

Change to:
```rust
"context": serde_json::json!({
    "tool": "google_mail",
    "params": {
        "action": "read",
        "message_id": msg_id
    }
}).to_string()
```

Apply to all button variants (read, reply, archive). Verify action names match the tool's action dispatch.

- [ ] **Step 2: Run tests and commit**

Run: `cargo test --lib google_mail -- --test-threads=1`

```bash
git add src/agent/tools/google_mail/mod.rs
git commit -m "feat(dispatch): migrate Google Mail buttons to structured context"
```

---

### Task 13: Migrate Google Tasks buttons to structured context

**Files:**
- Modify: `src/agent/tools/google_tasks/mod.rs:311-321` (button context format)

- [ ] **Step 1: Update button context format**

Current:
```rust
"context": serde_json::json!({
    "tool": "google_tasks",
    "task_id": task_id,
    "tasklist_id": tasklist_id,
    "action": "complete"
}).to_string()
```

Change to:
```rust
"context": serde_json::json!({
    "tool": "google_tasks",
    "params": {
        "action": "complete",
        "task_id": task_id,
        "tasklist_id": tasklist_id
    }
}).to_string()
```

- [ ] **Step 2: Run tests and commit**

Run: `cargo test --lib google_tasks -- --test-threads=1`

```bash
git add src/agent/tools/google_tasks/mod.rs
git commit -m "feat(dispatch): migrate Google Tasks buttons to structured context"
```

---

### Task 14: Migrate Todoist buttons to structured context

**Files:**
- Modify: `src/agent/tools/todoist/mod.rs:495-504` (button context format)

- [ ] **Step 1: Update button context format**

Current:
```rust
"context": serde_json::json!({
    "tool": "todoist",
    "task_id": task_id,
    "action": "complete"
}).to_string()
```

Change to:
```rust
"context": serde_json::json!({
    "tool": "todoist",
    "params": {
        "action": "complete_task",
        "task_id": task_id
    }
}).to_string()
```

Verify action name matches the tool's `execute()` action dispatch.

- [ ] **Step 2: Run tests and commit**

Run: `cargo test --lib todoist -- --test-threads=1`

```bash
git add src/agent/tools/todoist/mod.rs
git commit -m "feat(dispatch): migrate Todoist buttons to structured context"
```

---

### Task 15: Migrate GitHub buttons to structured context

**Files:**
- Modify: `src/agent/tools/github/mod.rs:657-737` (button context format)

- [ ] **Step 1: Update button context format**

Current (close issue, lines 662-667):
```rust
"context": serde_json::json!({
    "tool": "github",
    "repo": repo,
    "issue_number": number,
    "action": "close_issue"
}).to_string()
```

Change to:
```rust
"context": serde_json::json!({
    "tool": "github",
    "params": {
        "action": "close_issue",
        "repo": repo,
        "issue_number": number
    }
}).to_string()
```

Apply same transformation to all button variants:
- Close issue (line 662): wrap repo, issue_number, action into `params`
- Approve PR (line 694): wrap repo, pr_number, action into `params`
- Request changes (line 732): wrap repo, pr_number, action into `params`

Verify action names match the tool's `execute()` action dispatch.

- [ ] **Step 2: Run tests and commit**

Run: `cargo test --lib github -- --test-threads=1`

```bash
git add src/agent/tools/github/mod.rs
git commit -m "feat(dispatch): migrate GitHub buttons to structured context"
```

---

### Task 16: Migrate Cron buttons to structured context

**Files:**
- Modify: `src/agent/tools/cron/mod.rs:261-286` (button context format)

- [ ] **Step 1: Update button context format**

Current:
```rust
"context": serde_json::json!({
    "tool": "cron",
    "job_id": job.id,
    "action": action  // "pause", "resume", "remove"
}).to_string()
```

Change to:
```rust
"context": serde_json::json!({
    "tool": "cron",
    "params": {
        "action": action,
        "job_id": job.id
    }
}).to_string()
```

Apply to both the pause/resume and remove buttons.

- [ ] **Step 2: Run tests and commit**

Run: `cargo test --lib cron -- --test-threads=1`

```bash
git add src/agent/tools/cron/mod.rs
git commit -m "feat(dispatch): migrate Cron buttons to structured context"
```

---

## Chunk 6: Integration Testing and Documentation

### Task 17: Write integration tests for action dispatch

**Files:**
- Modify: `tests/message_flow.rs` or create: `tests/action_dispatch.rs`

- [ ] **Step 1: Write end-to-end dispatch integration test**

Check if a `tests/message_flow.rs` exists. If so, add tests there. Otherwise create `tests/action_dispatch.rs`.

The test should:
1. Create a test agent (using `create_test_agent_with()` from `tests/common/mod.rs`)
2. Build an `InboundMessage` with `action: Some(ActionDispatch { ... })`
3. Call `process_message()` on the agent
4. Verify the outbound message contains the tool result
5. Verify no LLM was called (the mock provider should have zero invocations)

```rust
#[tokio::test]
async fn test_action_dispatch_bypasses_llm() {
    // Setup test agent with a mock tool registry
    // Create InboundMessage with action dispatch to a known test tool
    // Call process_message()
    // Assert: outbound contains tool result
    // Assert: mock LLM provider was never called
}

#[tokio::test]
async fn test_action_dispatch_unknown_tool_returns_error() {
    // Create dispatch for tool "nonexistent"
    // Assert: outbound contains "Action failed: tool 'nonexistent' is not available."
}

#[tokio::test]
async fn test_action_dispatch_records_session_history() {
    // Execute dispatch
    // Load session from store
    // Assert: contains synthetic user message "[action: ...]"
    // Assert: contains assistant message with tool result
}
```

Note: The exact test setup depends heavily on the existing test infrastructure. Use `MockLLMProvider` and `TempDir` patterns from `tests/common/mod.rs`. The implementer should study `tests/message_flow.rs` or `tests/session_management.rs` for patterns.

- [ ] **Step 2: Run integration tests**

Run: `cargo test --test action_dispatch -- --test-threads=1`
(or `cargo test --test message_flow -- --test-threads=1` if added there)
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add tests/
git commit -m "test(dispatch): add integration tests for action dispatch"
```

---

### Task 18: Update documentation

**Files:**
- Modify: `CLAUDE.md` (add action dispatch section)
- No `docs/_pages/*.html` changes needed (action dispatch is internal plumbing, not user-facing tool documentation)

- [ ] **Step 1: Add action dispatch documentation to CLAUDE.md**

Add a new bullet point in the Common Pitfalls section of `CLAUDE.md`:

```markdown
- **Action dispatch (button auto-dispatch)**: `src/dispatch/mod.rs` defines `ActionDispatch`, `ActionSource`, `ActionDispatchPayload`, `DispatchContextStore`. When `InboundMessage.action` is `Some`, `process_message_unlocked()` short-circuits before the LLM — executes the tool directly via `ToolRegistry::execute()`, records synthetic session history, and returns the result. Channels create dispatch from button clicks: Slack deserializes `ButtonSpec.context` as `ActionDispatchPayload`, Discord uses `DispatchContextStore` (in-memory LRU, 15-min TTL). All tool button contexts use the `{"tool": "...", "params": {...}}` format. Follow-up actions via `ToolResult.metadata["follow_up_action"]` chain automatically (depth limit 3). Webhooks with `dispatch` config also bypass LLM. `AgentRunOverrides.action` carries dispatch through `process_direct_with_overrides()`.
```

- [ ] **Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: document action dispatch system in CLAUDE.md"
```

---

### Task 19: Run full test suite and final verification

- [ ] **Step 1: Run all unit tests**

Run: `cargo test --lib -- --test-threads=1`
Expected: PASS

- [ ] **Step 2: Run all integration tests**

Run: `cargo test --test session_management --test cron_jobs --test tool_registry --test message_flow -- --test-threads=1`
Expected: PASS

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No warnings

- [ ] **Step 4: Run fmt check**

Run: `cargo fmt -- --check`
Expected: No formatting issues

- [ ] **Step 5: Build release**

Run: `cargo build --release`
Expected: Success
