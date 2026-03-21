# Operator Approval Workflow Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an interactive operator approval flow for mutating tool actions, replacing the current hard-block with Approve/Deny buttons and configurable timeouts.

**Architecture:** Oneshot channel inside `execute_tool_call()` bridges the tool execution (blocking) with the button callback (async). `ApprovalStore` maps approval IDs to oneshot senders. `__approval` synthetic dispatch target resolves approvals before the session lock to prevent deadlock.

**Tech Stack:** Rust, tokio (oneshot, timeout, Notify), serde, existing button/dispatch infrastructure

**Spec:** `docs/superpowers/specs/2026-03-20-operator-approval-design.md`

---

## Chunk 1: Prerequisites and Config

### Task 1: Standardize action declarations on single-purpose tools

**Files:**
- Modify: `crates/oxicrab-tools-system/src/shell/mod.rs`
- Modify: `crates/oxicrab-tools-system/src/filesystem/mod.rs`
- Modify: `crates/oxicrab-tools-web/src/web/mod.rs`
- Modify: `crates/oxicrab-tools-web/src/http/mod.rs`
- Modify: `crates/oxicrab-tools-api/src/image_gen/mod.rs`
- Modify: `src/agent/tools/spawn/mod.rs`
- Modify: `src/agent/tools/stash/mod.rs`
- Modify: `src/agent/tools/tool_search/mod.rs`
- Modify: `src/agent/tools/interactive/mod.rs`

- [ ] **Step 1: Add `capabilities()` override to each tool**

For each tool listed above, add or modify the `capabilities()` method to include an `actions` declaration. Each tool needs a `ToolCapabilities` return with the appropriate action.

**ExecTool** (`crates/oxicrab-tools-system/src/shell/mod.rs`):
```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        actions: actions![execute],
        category: ToolCategory::System,
        ..Default::default()
    }
}
```

**ReadFileTool** (`crates/oxicrab-tools-system/src/filesystem/mod.rs`):
```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        actions: actions![read: ro],
        category: ToolCategory::System,
        ..Default::default()
    }
}
```

**WriteFileTool**:
```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        actions: actions![write],
        category: ToolCategory::System,
        ..Default::default()
    }
}
```

**EditFileTool**:
```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        actions: actions![edit],
        category: ToolCategory::System,
        ..Default::default()
    }
}
```

**ListDirTool**:
```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        actions: actions![list: ro],
        category: ToolCategory::System,
        ..Default::default()
    }
}
```

**WebSearchTool** (`crates/oxicrab-tools-web/src/web/mod.rs`):
```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        network_outbound: true,
        actions: actions![search: ro],
        category: ToolCategory::Web,
        ..Default::default()
    }
}
```

**WebFetchTool**:
```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        network_outbound: true,
        actions: actions![fetch: ro],
        category: ToolCategory::Web,
        ..Default::default()
    }
}
```

**HttpTool** (`crates/oxicrab-tools-web/src/http/mod.rs`):
```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        network_outbound: true,
        actions: actions![request],
        category: ToolCategory::Web,
        ..Default::default()
    }
}
```

**ImageGenTool** (`crates/oxicrab-tools-api/src/image_gen/mod.rs`):
```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        network_outbound: true,
        actions: actions![generate],
        category: ToolCategory::Media,
        ..Default::default()
    }
}
```

**SpawnTool** (`src/agent/tools/spawn/mod.rs`):
```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        subagent_access: SubagentAccess::Full,
        actions: actions![spawn],
        ..Default::default()
    }
}
```

**StashRetrieveTool** (`src/agent/tools/stash/mod.rs`):
```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        actions: actions![retrieve: ro],
        ..Default::default()
    }
}
```

**ToolSearchTool** (`src/agent/tools/tool_search/mod.rs`):
```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        actions: actions![search: ro],
        ..Default::default()
    }
}
```

**AddButtonsTool** (`src/agent/tools/interactive/mod.rs`):
```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        actions: actions![add: ro],
        ..Default::default()
    }
}
```

Read each file first. Some tools may already have a `capabilities()` override with some fields set (e.g., `network_outbound: true`, `category: ToolCategory::Web`). If so, add the `actions` field to the existing override — don't replace other fields.

- [ ] **Step 2: Run tests**

```bash
cargo test --lib -- --test-threads=1
```

Expected: All pass. The new action declarations are metadata only.

- [ ] **Step 3: Commit**

```bash
git add crates/oxicrab-tools-system/src/shell/mod.rs crates/oxicrab-tools-system/src/filesystem/mod.rs crates/oxicrab-tools-web/src/web/mod.rs crates/oxicrab-tools-web/src/http/mod.rs crates/oxicrab-tools-api/src/image_gen/mod.rs src/agent/tools/spawn/mod.rs src/agent/tools/stash/mod.rs src/agent/tools/tool_search/mod.rs src/agent/tools/interactive/mod.rs
git commit -m "refactor(tools): standardize action declarations on all single-purpose tools"
```

---

### Task 2: Add `ApprovalConfig` to config schema

**Files:**
- Modify: `crates/oxicrab-core/src/config/schema/agent.rs`
- Modify: `crates/oxicrab-core/src/config/schema/mod.rs` (if `AgentDefaults` is here)
- Modify: `src/agent/loop/config.rs`
- Modify: `config.example.toml`
- Modify: `src/config/schema/tests.rs` (credential overlays for config freshness test)

- [ ] **Step 1: Define `ApprovalConfig` struct**

In `crates/oxicrab-core/src/config/schema/agent.rs`, add:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApprovalConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub channel: String,
    #[serde(default = "default_approval_timeout")]
    pub timeout: u64,
    #[serde(default)]
    pub actions: Vec<String>,
}

fn default_approval_timeout() -> u64 {
    300
}

impl Default for ApprovalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            channel: String::new(),
            timeout: 300,
            actions: vec![],
        }
    }
}
```

- [ ] **Step 2: Add to `AgentDefaults`**

Find `AgentDefaults` struct. Add:
```rust
#[serde(default)]
pub approval: ApprovalConfig,
```

- [ ] **Step 3: Wire into `AgentLoopConfig`**

In `src/agent/loop/config.rs`, add to `AgentLoopConfig`:
```rust
pub approval_config: crate::config::ApprovalConfig,
```

In `from_config()`, add:
```rust
approval_config: config.agents.defaults.approval.clone(),
```

In `test_defaults()`, add:
```rust
approval_config: crate::config::ApprovalConfig::default(),
```

- [ ] **Step 4: Update `config.example.toml`**

Add after the existing `[agents.defaults]` section:
```toml
[agents.defaults.approval]
enabled = false
# channel = "slack:C0ABC123"   # operator channel (omit for same-conversation)
timeout = 300
# actions = ["google_mail.send", "google_mail.reply"]
```

- [ ] **Step 5: Update config freshness test if needed**

Check `src/config/schema/tests.rs`. If `credential_overlays()` needs an entry for approval fields, add it.

- [ ] **Step 6: Run tests**

```bash
cargo test --lib -- --test-threads=1
```

Expected: All pass, including `test_config_example_is_up_to_date`.

- [ ] **Step 7: Commit**

```bash
git commit -m "feat(config): add ApprovalConfig for operator approval workflow"
```

---

### Task 3: Implement `ApprovalConfig.covers()` with tests

**Files:**
- Modify: `crates/oxicrab-core/src/config/schema/agent.rs`

- [ ] **Step 1: Write failing tests**

Add tests at the bottom of `agent.rs` (or in a `tests` submodule):

```rust
#[cfg(test)]
mod approval_tests {
    use super::*;
    use crate::tools::base::ActionDescriptor;

    fn make_actions(names: &[(&str, bool)]) -> Vec<ActionDescriptor> {
        names.iter().map(|(n, ro)| ActionDescriptor {
            name: n.to_string(),
            read_only: *ro,
        }).collect()
    }

    #[test]
    fn test_covers_explicit_tool_dot_action() {
        let config = ApprovalConfig {
            enabled: true,
            actions: vec!["google_mail.send".to_string()],
            ..Default::default()
        };
        let actions = make_actions(&[("send", false), ("search", true)]);
        assert!(config.covers("google_mail", "send", &actions));
        assert!(!config.covers("google_mail", "search", &actions));
    }

    #[test]
    fn test_covers_bare_tool_name_matches_all() {
        let config = ApprovalConfig {
            enabled: true,
            actions: vec!["google_mail".to_string()],
            ..Default::default()
        };
        let actions = make_actions(&[("send", false), ("search", true)]);
        assert!(config.covers("google_mail", "send", &actions));
        assert!(config.covers("google_mail", "search", &actions));
    }

    #[test]
    fn test_covers_empty_list_uses_mutating_actions() {
        let config = ApprovalConfig {
            enabled: true,
            actions: vec![],
            ..Default::default()
        };
        let actions = make_actions(&[("send", false), ("search", true)]);
        // Empty list = all non-read-only actions
        assert!(config.covers("google_mail", "send", &actions));
        assert!(!config.covers("google_mail", "search", &actions));
    }

    #[test]
    fn test_covers_single_purpose_tool_empty_action_param() {
        let config = ApprovalConfig {
            enabled: true,
            actions: vec!["exec.execute".to_string()],
            ..Default::default()
        };
        let actions = make_actions(&[("execute", false)]);
        // Single-purpose tool: action_from_params is "" → falls back to declared action
        assert!(config.covers("exec", "", &actions));
    }

    #[test]
    fn test_covers_miss() {
        let config = ApprovalConfig {
            enabled: true,
            actions: vec!["google_mail.send".to_string()],
            ..Default::default()
        };
        let actions = make_actions(&[("list_issues", false)]);
        assert!(!config.covers("github", "list_issues", &actions));
    }

    #[test]
    fn test_covers_disabled_always_false() {
        let config = ApprovalConfig {
            enabled: false,
            actions: vec!["google_mail.send".to_string()],
            ..Default::default()
        };
        let actions = make_actions(&[("send", false)]);
        assert!(!config.covers("google_mail", "send", &actions));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p oxicrab-core --lib approval_tests -- --test-threads=1
```

Expected: FAIL — `covers()` method doesn't exist yet.

- [ ] **Step 3: Implement `covers()`**

```rust
impl ApprovalConfig {
    /// Check if a tool action is covered by this approval config.
    /// `action_from_params` is the action from tool call params (empty for single-purpose tools).
    /// `tool_actions` is the tool's declared ActionDescriptor list from capabilities().
    pub fn covers(&self, tool_name: &str, action_from_params: &str, tool_actions: &[ActionDescriptor]) -> bool {
        if !self.enabled {
            return false;
        }

        // Resolve effective action for single-purpose tools
        let effective_action = if action_from_params.is_empty() && tool_actions.len() == 1 {
            tool_actions[0].name.as_str()
        } else {
            action_from_params
        };

        if self.actions.is_empty() {
            // Default: cover all non-read-only actions
            tool_actions.iter().any(|a| a.name == effective_action && !a.read_only)
        } else {
            // Explicit list
            let full_key = format!("{tool_name}.{effective_action}");
            self.actions.iter().any(|a| *a == full_key || *a == tool_name)
        }
    }
}
```

Add `use crate::tools::base::ActionDescriptor;` at the top of the file if not already imported.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p oxicrab-core --lib approval_tests -- --test-threads=1
```

Expected: All 6 pass.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(approval): implement ApprovalConfig.covers() with action matching"
```

---

## Chunk 2: ApprovalStore and Core Flow

### Task 4: Implement `ApprovalStore`

**Files:**
- Create: `src/agent/approval/mod.rs`
- Modify: `src/agent/mod.rs` (add `pub mod approval;`)

- [ ] **Step 1: Write failing tests**

Create `src/agent/approval/mod.rs` with tests first:

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::oneshot;

#[derive(Debug)]
pub enum ApprovalDecision {
    Approved,
    Denied { reason: Option<String> },
}

pub(crate) struct ApprovalEntry {
    pub sender: oneshot::Sender<ApprovalDecision>,
    pub tool_name: String,
    pub action: String,
    pub requested_by: String,
    pub operator_channel: String,
}

pub struct ApprovalStore {
    pending: Mutex<HashMap<String, ApprovalEntry>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_resolve() {
        let store = ApprovalStore::new();
        let (tx, rx) = oneshot::channel();
        let entry = ApprovalEntry {
            sender: tx,
            tool_name: "gmail".into(),
            action: "send".into(),
            requested_by: "user1".into(),
            operator_channel: "slack:C123".into(),
        };
        store.register("appr-abc123", entry);
        let result = store.resolve("appr-abc123", "slack:C123", ApprovalDecision::Approved);
        assert!(result.is_ok());
        // rx should have received the decision
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn test_resolve_unknown_id() {
        let store = ApprovalStore::new();
        let result = store.resolve("appr-unknown", "slack:C123", ApprovalDecision::Approved);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_wrong_channel() {
        let store = ApprovalStore::new();
        let (tx, _rx) = oneshot::channel();
        let entry = ApprovalEntry {
            sender: tx,
            tool_name: "gmail".into(),
            action: "send".into(),
            requested_by: "user1".into(),
            operator_channel: "slack:C123".into(),
        };
        store.register("appr-abc123", entry);
        let result = store.resolve("appr-abc123", "slack:CWRONG", ApprovalDecision::Approved);
        assert!(result.is_err());
    }

    #[test]
    fn test_double_resolve() {
        let store = ApprovalStore::new();
        let (tx, _rx) = oneshot::channel();
        let entry = ApprovalEntry {
            sender: tx,
            tool_name: "gmail".into(),
            action: "send".into(),
            requested_by: "user1".into(),
            operator_channel: "slack:C123".into(),
        };
        store.register("appr-abc123", entry);
        assert!(store.resolve("appr-abc123", "slack:C123", ApprovalDecision::Approved).is_ok());
        // Second resolve should fail — entry consumed
        assert!(store.resolve("appr-abc123", "slack:C123", ApprovalDecision::Approved).is_err());
    }

    #[test]
    fn test_self_approval_empty_channel() {
        let store = ApprovalStore::new();
        let (tx, rx) = oneshot::channel();
        let entry = ApprovalEntry {
            sender: tx,
            tool_name: "gmail".into(),
            action: "send".into(),
            requested_by: "user1".into(),
            operator_channel: String::new(), // self-approval
        };
        store.register("appr-abc123", entry);
        // Any source channel is accepted when operator_channel is empty
        let result = store.resolve("appr-abc123", "slack:U12345", ApprovalDecision::Approved);
        assert!(result.is_ok());
        assert!(rx.try_recv().is_ok());
    }
}
```

- [ ] **Step 2: Implement `ApprovalStore`**

```rust
impl ApprovalStore {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub fn register(&self, approval_id: &str, entry: ApprovalEntry) {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(approval_id.to_string(), entry);
    }

    /// Resolve a pending approval. Returns Ok(tool_info) on success,
    /// Err(message) if not found or unauthorized.
    pub fn resolve(
        &self,
        approval_id: &str,
        source_channel: &str,
        decision: ApprovalDecision,
    ) -> Result<(String, String, String), String> {
        let mut pending = self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let Some(entry) = pending.remove(approval_id) else {
            return Err("this approval request has already been resolved or expired".into());
        };

        // Validate source channel (empty operator_channel = self-approval, accept any source)
        if !entry.operator_channel.is_empty() && source_channel != entry.operator_channel {
            // Put entry back — wrong channel, don't consume it
            let tool_name = entry.tool_name.clone();
            pending.insert(approval_id.to_string(), entry);
            return Err(format!("approval response from unauthorized channel for {tool_name}"));
        }

        let tool_name = entry.tool_name.clone();
        let action = entry.action.clone();
        let requested_by = entry.requested_by.clone();
        let _ = entry.sender.send(decision);
        Ok((tool_name, action, requested_by))
    }

    pub fn generate_id() -> String {
        format!("appr-{}", &uuid::Uuid::new_v4().to_string()[..12])
    }
}
```

- [ ] **Step 3: Register the module**

In `src/agent/mod.rs`, add:
```rust
pub mod approval;
```

- [ ] **Step 4: Run tests**

```bash
cargo test --lib approval -- --test-threads=1
```

Expected: All 5 pass.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(approval): implement ApprovalStore with register/resolve/authorization"
```

---

### Task 5: Wire approval into `execute_tool_call` and `AgentLoop`

**Files:**
- Modify: `src/agent/loop/helpers.rs`
- Modify: `src/agent/loop/iteration.rs`
- Modify: `src/agent/loop/processing.rs`
- Modify: `src/agent/loop/mod.rs`

- [ ] **Step 1: Add `ApprovalStore` field to `AgentLoop`**

In `src/agent/loop/mod.rs`, add to the `AgentLoop` struct:
```rust
approval_store: Arc<crate::agent::approval::ApprovalStore>,
approval_config: crate::config::ApprovalConfig,
outbound_tx: Arc<tokio::sync::mpsc::Sender<OutboundMessage>>,
```

In `AgentLoop::new()`, initialize:
```rust
approval_store: Arc::new(crate::agent::approval::ApprovalStore::new()),
approval_config: config.approval_config,
outbound_tx: outbound_tx.clone(),  // outbound_tx is already available
```

Note: `outbound_tx` may already be stored somewhere. Check — it's used in `process_message_unlocked` for outbound message building. If it's not a field, it may be available through `self.bus` or another path. Read the code to find how outbound messages are sent, and use the same mechanism.

- [ ] **Step 2: Add approval parameters to `execute_tool_call`**

In `src/agent/loop/helpers.rs`, change the signature:

```rust
pub(super) async fn execute_tool_call(
    registry: &ToolRegistry,
    tc_name: &str,
    tc_args: &Value,
    available_tools: &[String],
    ctx: &ExecutionContext,
    exfil_allow: Option<&[String]>,
    workspace: Option<&std::path::Path>,
    // New:
    approval_ctx: Option<ApprovalContext<'_>>,
) -> ToolResult
```

Where `ApprovalContext` bundles the three new params:

```rust
pub(super) struct ApprovalContext<'a> {
    pub store: &'a crate::agent::approval::ApprovalStore,
    pub config: &'a crate::config::ApprovalConfig,
    pub outbound_tx: &'a tokio::sync::mpsc::Sender<OutboundMessage>,
    pub channel: &'a str,
    pub chat_id: &'a str,
    pub sender_id: &'a str,
}
```

- [ ] **Step 3: Add gate check and `await_approval` function**

In `execute_tool_call`, after the existing MCP hard-block and before the legacy hard-block:

```rust
// Interactive approval flow (when enabled)
if let Some(ref approval) = approval_ctx {
    let action = tc_args.get("action").and_then(|v| v.as_str()).unwrap_or("");
    let tool_caps = tool.capabilities();
    if approval.config.enabled && approval.config.covers(tc_name, action, &tool_caps.actions) {
        return await_approval(
            tool.as_ref(), tc_name, action, tc_args, ctx,
            approval.store, approval.config, approval.outbound_tx,
            approval.channel, approval.chat_id, approval.sender_id,
        ).await;
    }
}

// Existing legacy hard-block (only reached when approval is disabled or action not covered)
if tool.requires_approval_for_action(action) {
    // ... existing error return
}
```

Implement `await_approval`:

```rust
async fn await_approval(
    tool: &dyn Tool,
    tool_name: &str,
    action: &str,
    params: &Value,
    ctx: &ExecutionContext,
    store: &crate::agent::approval::ApprovalStore,
    config: &crate::config::ApprovalConfig,
    outbound_tx: &tokio::sync::mpsc::Sender<OutboundMessage>,
    channel: &str,
    chat_id: &str,
    sender_id: &str,
) -> ToolResult {
    use crate::agent::approval::{ApprovalDecision, ApprovalEntry, ApprovalStore};
    use crate::bus::OutboundMessage;

    let approval_id = ApprovalStore::generate_id();
    let (tx, rx) = tokio::sync::oneshot::channel();

    let display_action = if action.is_empty() {
        tool.capabilities().actions.first()
            .map(|a| a.name.as_str())
            .unwrap_or("execute")
            .to_string()
    } else {
        action.to_string()
    };

    // Determine operator channel target
    let operator_target = if config.channel.is_empty() {
        (channel.to_string(), chat_id.to_string())
    } else if let Some((ch, id)) = config.channel.split_once(':') {
        (ch.to_string(), id.to_string())
    } else {
        warn!("invalid approval channel format '{}', falling back to same conversation", config.channel);
        (channel.to_string(), chat_id.to_string())
    };

    let operator_channel_key = if config.channel.is_empty() {
        String::new()
    } else {
        config.channel.clone()
    };

    // Register the pending approval
    store.register(&approval_id, ApprovalEntry {
        sender: tx,
        tool_name: tool_name.to_string(),
        action: display_action.clone(),
        requested_by: sender_id.to_string(),
        operator_channel: operator_channel_key,
    });

    // Step 2: Send feedback to user
    let feedback = OutboundMessage::new(
        channel,
        chat_id,
        format!("This action requires approval. Waiting for an operator to approve `{tool_name}.{display_action}`..."),
    );
    let _ = outbound_tx.send(feedback).await;

    // Step 3: Build and send approval request to operator
    let request_text = format_approval_request(
        tool_name, &display_action, sender_id, channel, chat_id, params,
    );
    let approve_ctx = serde_json::json!({
        "tool": "__approval",
        "params": {"approval_id": approval_id, "decision": "approved"}
    }).to_string();
    let deny_ctx = serde_json::json!({
        "tool": "__approval",
        "params": {"approval_id": approval_id, "decision": "denied"}
    }).to_string();

    let buttons = vec![
        serde_json::json!({"id": format!("approve_{approval_id}"), "label": "Approve", "style": "primary", "context": approve_ctx}),
        serde_json::json!({"id": format!("deny_{approval_id}"), "label": "Deny", "style": "danger", "context": deny_ctx}),
    ];

    let mut request_msg = OutboundMessage::new(
        &operator_target.0,
        &operator_target.1,
        request_text,
    );
    request_msg.metadata.insert(
        crate::bus::meta::BUTTONS.to_string(),
        serde_json::Value::Array(buttons),
    );
    let _ = outbound_tx.send(request_msg).await;

    // Step 4: Wait for approval
    match tokio::time::timeout(
        std::time::Duration::from_secs(config.timeout),
        rx,
    ).await {
        Ok(Ok(ApprovalDecision::Approved)) => {
            info!("approval granted for {tool_name}.{display_action} (requested by {sender_id})");
            tool.execute(params.clone(), ctx).await
                .unwrap_or_else(|e| ToolResult::error(format!("tool execution failed after approval: {e}")))
        }
        Ok(Ok(ApprovalDecision::Denied { reason })) => {
            let reason_str = reason.map(|r| format!(": {r}")).unwrap_or_default();
            info!("approval denied for {tool_name}.{display_action} (requested by {sender_id}){reason_str}");
            ToolResult::error(format!("action denied by operator{reason_str}"))
        }
        _ => {
            warn!("approval timed out for {tool_name}.{display_action} (requested by {sender_id})");
            ToolResult::error("approval timed out — action not executed")
        }
    }
}

fn format_approval_request(
    tool_name: &str,
    action: &str,
    sender_id: &str,
    channel: &str,
    chat_id: &str,
    params: &Value,
) -> String {
    let mut lines = vec![
        "Approval Request".to_string(),
        String::new(),
        format!("Tool: {tool_name} -> {action}"),
        format!("Requested by: {sender_id} ({channel} {chat_id})"),
    ];

    if let Some(obj) = params.as_object() {
        lines.push(String::new());
        let mut count = 0;
        for (key, value) in obj {
            if key == "action" { continue; }
            if count >= 10 { break; }
            let val_str = if let Some(s) = value.as_str() {
                if s.len() > 500 {
                    let boundary = s.floor_char_boundary(500);
                    format!("{}...\n[{} chars total]", &s[..boundary], s.len())
                } else {
                    s.to_string()
                }
            } else {
                let s = value.to_string();
                if s.len() > 500 {
                    let boundary = s.floor_char_boundary(500);
                    format!("{}...\n[{} chars total]", &s[..boundary], s.len())
                } else {
                    s
                }
            };
            lines.push(format!("{key}: {val_str}"));
            count += 1;
        }
    }

    lines.join("\n")
}
```

- [ ] **Step 4: Update all `execute_tool_call` call sites**

Find all calls to `execute_tool_call` in `iteration.rs` and `processing.rs`. Add `approval_ctx` parameter. In the agent loop calls, pass `Some(ApprovalContext { ... })` built from `self.approval_store`, `self.approval_config`, `self.outbound_tx`, and context from the current message. In test call sites, pass `None`.

Read `iteration.rs` lines ~480 and ~511 to find the exact call sites. Read `processing.rs` lines ~1143 and ~1534 for the dispatch call sites.

- [ ] **Step 5: Run tests**

```bash
cargo test --lib -- --test-threads=1
```

Expected: All pass. Approval is disabled by default so no behavior change.

- [ ] **Step 6: Commit**

```bash
git commit -m "feat(approval): wire approval gate into execute_tool_call with oneshot flow"
```

---

### Task 6: Add `__approval` dispatch handler with deadlock prevention

**Files:**
- Modify: `src/agent/loop/mod.rs`
- Modify: `src/agent/loop/processing.rs`

- [ ] **Step 1: Add `__approval` bypass before session lock**

In `src/agent/loop/mod.rs`, in `process_message()`, before the session lock acquisition:

```rust
async fn process_message(&self, msg: InboundMessage) -> Result<Option<OutboundMessage>> {
    // Approval callbacks bypass the session lock to prevent deadlock
    // in self-approval mode (same channel as user).
    if let Some(ref action) = msg.action {
        if action.tool == "__approval" {
            return self.resolve_approval(&msg, action).await;
        }
    }

    // ... existing session lock and processing ...
}
```

- [ ] **Step 2: Implement `resolve_approval`**

```rust
async fn resolve_approval(
    &self,
    msg: &InboundMessage,
    action: &crate::dispatch::ActionDispatch,
) -> Result<Option<OutboundMessage>> {
    use crate::agent::approval::ApprovalDecision;

    let approval_id = action.params.get("approval_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let decision_str = action.params.get("decision")
        .and_then(|v| v.as_str())
        .unwrap_or("denied");

    let decision = if decision_str == "approved" {
        ApprovalDecision::Approved
    } else {
        ApprovalDecision::Denied { reason: None }
    };

    let source_channel = format!("{}:{}", msg.channel, msg.chat_id);

    match self.approval_store.resolve(approval_id, &source_channel, decision) {
        Ok((tool_name, action_name, requested_by)) => {
            let status = if decision_str == "approved" { "Approved" } else { "Denied" };
            let response = format!(
                "{status} {tool_name}.{action_name} for {requested_by} (by {})",
                msg.sender_id
            );
            Ok(Some(OutboundMessage::from_inbound(msg.clone(), response).build()))
        }
        Err(err_msg) => {
            Ok(Some(OutboundMessage::from_inbound(msg.clone(), err_msg).build()))
        }
    }
}
```

- [ ] **Step 3: Also handle `__approval` in `handle_direct_dispatch`**

In `processing.rs`, find `handle_direct_dispatch`. Add a check near the top:

```rust
if dispatch.tool == "__approval" {
    // Handled in process_message() before session lock — should not reach here.
    // But if it does (e.g., from a webhook dispatch), handle it.
    return self.resolve_approval(msg, &dispatch).await
        .map(|opt| opt.map(|msg| (msg, HashMap::new())));
}
```

Adapt the return type to match `handle_direct_dispatch`'s return type (read the actual code).

- [ ] **Step 4: Run tests**

```bash
cargo test --lib -- --test-threads=1
```

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(approval): add __approval dispatch handler with deadlock prevention"
```

---

## Chunk 3: Startup Warning, Integration Tests, Documentation

### Task 7: Add startup warning for unprotected mutating tools

**Files:**
- Modify: `src/agent/loop/mod.rs` (in `AgentLoop::new()`)

- [ ] **Step 1: Add warning logic after tool registration**

In `AgentLoop::new()`, after the `ToolRegistry` is built, add:

```rust
// Warn about tools with mutating actions that have no approval gate
if !config.approval_config.enabled {
    for tool_name in tools.tool_names() {
        if let Some(tool) = tools.get(&tool_name) {
            let caps = tool.capabilities();
            // Skip MCP tools (separately gated by trust level)
            if !caps.built_in { continue; }
            // Skip tools that have requires_approval_for_action overrides
            let has_legacy_gate = caps.actions.iter()
                .any(|a| !a.read_only && tool.requires_approval_for_action(&a.name));
            if has_legacy_gate { continue; }
            // Warn about unprotected mutating actions
            let mutating: Vec<&str> = caps.actions.iter()
                .filter(|a| !a.read_only)
                .map(|a| a.name.as_str())
                .collect();
            if !mutating.is_empty() {
                warn!(
                    "tool '{}' has mutating actions ({}) without approval gating",
                    tool_name,
                    mutating.join(", ")
                );
            }
        }
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test --lib -- --test-threads=1
```

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(approval): add startup warning for unprotected mutating tools"
```

---

### Task 8: Integration tests

**Files:**
- Create: `tests/approval_integration.rs`
- Modify: `tests/common/mod.rs` (if new helpers needed)

- [ ] **Step 1: Write integration tests**

Create `tests/approval_integration.rs` with the test scenarios from the spec. Each test:

1. Creates a `MockLLMProvider` with responses that trigger tool calls for gated actions
2. Creates an `AgentLoopConfig` with `approval_config.enabled = true`
3. Runs `process_direct` or `process_direct_with_overrides`
4. Checks outbound messages for approval request
5. Simulates button click via `ActionDispatch`
6. Verifies tool execution result

Key tests:
- `test_approval_flow_approve` — full approve flow
- `test_approval_flow_deny` — deny returns error
- `test_approval_timeout` — short timeout auto-denies
- `test_approval_disabled_legacy_block` — disabled config preserves hard-block
- `test_approval_disabled_no_block_for_uncovered` — disabled config, tool without legacy gate executes freely

Read `tests/message_flow.rs` and `tests/safety_integration.rs` for the test setup pattern. The approval flow is harder to test end-to-end because it requires simulating a button click while the tool is waiting. Use `tokio::spawn` to click the button after a short delay:

```rust
let store_clone = approval_store.clone();
tokio::spawn(async move {
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Find the pending approval and resolve it
    store_clone.resolve("...", "...", ApprovalDecision::Approved);
});
```

The challenge is getting the approval_id. One approach: make the store's `pending` map inspectable in tests via a `pending_ids()` method.

- [ ] **Step 2: Run integration tests**

```bash
cargo test --test approval_integration -- --test-threads=1
```

- [ ] **Step 3: Commit**

```bash
git commit -m "test(approval): add integration tests for approval flow"
```

---

### Task 9: Documentation updates

**Files:**
- Modify: `docs/_pages/config.html`
- Modify: `docs/_pages/tools.html`
- Modify: `docs/_pages/channels.html`
- Modify: `CLAUDE.md`
- Modify: `README.md`

- [ ] **Step 1: Update config.html**

Add `[agents.defaults.approval]` section with field table.

- [ ] **Step 2: Update tools.html**

Add subsection on mutating vs read-only actions and approval configuration.

- [ ] **Step 3: Update channels.html**

Add note about approval channel configuration.

- [ ] **Step 4: Update CLAUDE.md**

Add entry documenting `ApprovalConfig`, `ApprovalStore`, `__approval` dispatch, startup warning.

- [ ] **Step 5: Update README.md**

Add "operator approval workflow" to the Security features bullet.

- [ ] **Step 6: Rebuild docs**

```bash
python3 docs/build.py
```

- [ ] **Step 7: Run config freshness test**

```bash
cargo test --lib test_config_example -- --test-threads=1
```

- [ ] **Step 8: Commit**

```bash
git commit -m "docs: add operator approval workflow documentation"
```
