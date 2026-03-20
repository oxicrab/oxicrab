# Operator Approval Workflow

## Goal

Add an interactive operator approval flow for mutating tool actions. When enabled, the bot pauses execution, sends an approval request with Approve/Deny buttons, and waits for a response before proceeding. When disabled (the default), the existing `requires_approval_for_action()` hard-block remains in place as a safety net.

## Architecture

Oneshot channel inside `execute_tool_call()`. When a tool action requires approval:

1. Register a `tokio::sync::oneshot` sender in a shared `ApprovalStore`
2. Send a "waiting for approval" message to the user's conversation
3. Send the approval request (with buttons) to the operator channel
4. Await the oneshot receiver with a configurable timeout
5. On approve: execute the tool. On deny or timeout: return error to LLM.

From the agent loop's perspective, the tool just takes longer to return. No new state machines, no new `ToolResult` variants.

## Prerequisite: Standardize tool action declarations

All tools must declare at least one action via the `actions!` macro so the approval system, startup warning, and policy systems work uniformly.

| Tool | Action declaration |
|------|-------------------|
| `ExecTool` | `actions![execute]` |
| `ReadFileTool` | `actions![read: ro]` |
| `WriteFileTool` | `actions![write]` |
| `EditFileTool` | `actions![edit]` |
| `ListDirTool` | `actions![list: ro]` |
| `WebSearchTool` | `actions![search: ro]` |
| `WebFetchTool` | `actions![fetch: ro]` |
| `HttpTool` | `actions![request]` |
| `ImageGenTool` | `actions![generate]` |
| `SpawnTool` | `actions![spawn]` |
| `StashRetrieveTool` | `actions![retrieve: ro]` |
| `ToolSearchTool` | `actions![search: ro]` |
| `AddButtonsTool` | `actions![add: ro]` |

These tools remain single-purpose — no `params["action"]` dispatch is added. The action name is metadata for the policy system only.

Note: `AddButtonsTool` is marked `ro` because it only modifies internal request-scoped metadata (no external side effects). This means the approval system will never gate it, which is intentional.

## Config schema

```toml
[agents.defaults.approval]
enabled = false                    # master switch, off by default
channel = ""                       # "slack:C0XXXXXX" or "" for same-conversation
timeout = 300                      # seconds to wait before auto-deny
actions = []                       # empty = use requires_approval_for_action() defaults; explicit list extends
```

`ApprovalConfig` struct in `crates/oxicrab-core/src/config/schema/agent.rs`.

### Channel format

The `channel` field uses the same `"channel_type:chat_id"` format as cron targets:
- `"slack:C0ABC123"` — Slack channel ID (not `#name`; resolve at config time or startup)
- `"discord:123456789"` — Discord channel ID
- `"telegram:12345"` — Telegram chat ID

The bot must be a member of the configured channel. At startup, validate the channel is reachable (send a test message or just log a warning if unreachable). If the channel is unreachable at approval time, fall back to same-conversation approval with a warning log.

### Behavior by configuration

- **`enabled = false` (default)**: No interactive approval. The existing `requires_approval_for_action()` hard-block remains active for Gmail/Calendar/GitHub/MCP. Startup warning logged for tools with mutating actions that have no approval gate at all.
- **`enabled = true`, `actions = []`**: Interactive approval for the default set (same tools/actions that `requires_approval_for_action()` currently gates). The hard-block is replaced with the interactive flow.
- **`enabled = true`, `actions = ["google_mail.send", "exec.execute"]`**: Interactive approval for the listed actions. This extends/replaces the defaults.
- **`channel = ""`**: Approval buttons appear in the user's own conversation (self-approval).
- **`channel = "slack:C0ABC123"`**: Approval requests routed to a dedicated operator channel.

### Action matching for single-purpose tools

For tools without a `params["action"]` field (single-purpose tools), the gate check uses the tool's declared action name from `ActionDescriptor`. `covers()` logic:

```rust
fn covers(&self, tool_name: &str, action_from_params: &str) -> bool {
    // For single-purpose tools, action_from_params is "" — use the tool's
    // declared action name from capabilities instead.
    let effective_action = if action_from_params.is_empty() {
        // Look up the tool's single declared action name
        tool_declared_action
    } else {
        action_from_params
    };
    // Match against "tool_name.action_name" or bare "tool_name"
}
```

The `covers()` method receives the tool's `ActionDescriptor` list as context so it can resolve single-purpose tools.

### Known limitation

Single global approval channel. Per-tool-category routing (e.g., email approvals to `#email-ops`, GitHub to `#dev-ops`) is not supported in v1.

## Separation from MCP trust gating

Two separate concerns with separate enforcement:

1. **MCP trust** (`requires_approval() = true` on `AttenuatedMcpTool`): Hard block. Untrusted MCP tools never execute regardless of approval config. Unchanged.
2. **Operator approval** (`ApprovalConfig`): Interactive flow for trusted built-in tools with consequential actions. Only active when `enabled = true`.

### Backward compatibility

When `enabled = false`, `requires_approval_for_action()` overrides on Gmail/Calendar/GitHub **remain as hard-blocks** — the existing behavior is preserved exactly. No security regression.

When `enabled = true`, the interactive approval flow **replaces** the hard-block for tools that are covered by the approval config. `requires_approval_for_action()` is still checked but its effect changes from "return error immediately" to "enter interactive approval flow."

The check order in `execute_tool_call()`:

```
1. MCP hard block: if tool.requires_approval() → error (unchanged, always active)
2. Interactive approval: if approval_enabled && config.covers(tool, action) → interactive flow
3. Legacy hard block: if !approval_enabled && tool.requires_approval_for_action(action) → error (existing behavior)
4. Normal execution
```

This means no configuration change is needed for existing deployments. Enabling approval is purely additive.

## ApprovalStore

```rust
pub struct ApprovalStore {
    pending: Mutex<HashMap<String, ApprovalEntry>>,
}

struct ApprovalEntry {
    sender: oneshot::Sender<ApprovalDecision>,
    tool_name: String,
    action: String,
    requested_by: String,
    source_channel: String,    // for authorization check
    operator_channel: String,  // where the request was sent
}

pub enum ApprovalDecision {
    Approved,
    Denied { reason: Option<String> },
}
```

### Approval ID format

UUID v4, formatted as `appr-{12 hex chars}` (e.g., `appr-a1b2c3d4e5f6`). 48 bits of entropy — sufficient to prevent guessing within the 5-minute timeout window.

### Authorization on resolve

When `resolve()` is called from the button callback, it verifies:
- The click originated from the configured operator channel (or the user's own channel for self-approval)
- If the source doesn't match, reject with "approval response from unauthorized channel"

No per-user operator allowlist in v1. Any user in the operator channel can approve/deny.

### Lifecycle

1. `register(approval_id, entry)` — stores the entry
2. `resolve(approval_id, source_channel, decision)` — validates source, takes the sender, fires it. Returns an error string if ID not found or unauthorized.
3. Timeout — the receiver returns `Err` when the sender is dropped
4. Stale entries — when `resolve()` is called for an unknown ID (e.g., after process restart), return "approval request expired or already resolved"

Lives in `AgentLoop` as `Arc<ApprovalStore>`. No persistence — approvals are ephemeral.

## Approval request message

Format sent to the operator channel (or same conversation):

```
📋 Approval Request

Tool: google_mail → send
Requested by: user1 (Slack #general)
Session: slack:U12345

To: alice@example.com
Subject: Q1 Budget Review
Body: Hi Alice, I've attached the Q1 budget summary. Key highlights:
- Revenue up 12% QoQ...
[1847 chars total]
```

With `[Approve]` and `[Deny]` buttons.

### Parameter formatting

- Show all params except `action` (already in header)
- Truncate values longer than 500 chars with `[N chars total]`
- Cap at 10 params displayed
- Recipient fields (`to`, `owner`, `repo`, `calendar_id`) always shown in full

### Button context

Both buttons use `ActionDispatchPayload`:

```json
{"tool": "__approval", "params": {"approval_id": "appr-a1b2c3d4e5f6", "decision": "approved"}}
```

`__approval` is a synthetic dispatch target handled in `handle_direct_dispatch()`.

### Post-resolution message update

After the operator clicks Approve or Deny, edit the approval request message to reflect the outcome and remove the buttons:

```
✅ Approved — google_mail.send for user1 (by operator1)
```
or
```
❌ Denied — google_mail.send for user1 (by operator1)
```

Use the channel's `edit_message()` if the message ID was captured from `send_and_get_id()`. If editing fails (channel doesn't support it), log and continue.

For stale button clicks (approval already resolved or process restarted), respond with: "This approval request has already been resolved or expired."

## Execution flow

### Step 1 — Gate check

In `execute_tool_call()`, after the MCP hard-block. The function needs additional parameters:

```rust
pub(super) async fn execute_tool_call(
    registry: &ToolRegistry,
    tc_name: &str,
    tc_args: &Value,
    available_tools: &[Value],
    ctx: &ExecutionContext,
    exfil_allow: Option<&[String]>,
    workspace: Option<&Path>,
    // New parameters for approval:
    approval_store: Option<&ApprovalStore>,
    approval_config: Option<&ApprovalConfig>,
    outbound_tx: Option<&Sender<OutboundMessage>>,
) -> ToolResult
```

All three are `Option` — when `None`, the approval flow is skipped (backward compatible with test call sites). The agent loop passes them from its own fields.

Gate check:

```rust
let action = tc_args.get("action").and_then(|v| v.as_str()).unwrap_or("");
let tool_caps = tool.capabilities();

if let (Some(store), Some(config), Some(tx)) = (approval_store, approval_config, outbound_tx) {
    if config.enabled && config.covers(tc_name, action, &tool_caps.actions) {
        return await_approval(tool, tc_args, ctx, store, config, tx).await;
    }
}

// Existing legacy hard-block (only reached when approval is disabled)
if tool.requires_approval_for_action(action) {
    return ToolResult::error("...");
}
```

### Step 2 — User feedback

Send to the user's conversation via `outbound_tx`:

```
⏳ This action requires approval. Waiting for an operator to approve `google_mail.send`...
```

### Step 3 — Operator request

Build the formatted approval message with buttons. Send via `outbound_tx` to the configured approval channel (or same conversation if `channel` is empty).

### Step 4 — Wait

```rust
match tokio::time::timeout(Duration::from_secs(config.timeout), approval_rx).await {
    Ok(Ok(ApprovalDecision::Approved)) => tool.execute(params, ctx).await,
    Ok(Ok(ApprovalDecision::Denied { reason })) => {
        ToolResult::error(format!("action denied by operator{}",
            reason.map(|r| format!(": {r}")).unwrap_or_default()))
    }
    _ => ToolResult::error("approval timed out — action not executed"),
}
```

### Step 5 — Button callback

Operator clicks Approve or Deny:

1. Channel receives button click → `InboundMessage` with `ActionDispatch`
2. `handle_direct_dispatch()` sees `tool: "__approval"`
3. Extracts `approval_id` and `decision`
4. Calls `approval_store.resolve(approval_id, source_channel, decision)`
5. Store validates source channel matches the configured operator channel
6. Oneshot fires → Step 4 unblocks
7. Edit the approval request message to show outcome and remove buttons
8. Returns confirmation to operator: "Approved google_mail.send for user1"

### Self-approval deadlock prevention

When `channel = ""` (self-approval mode), the approval request is sent to the same `channel:chat_id` as the user. The button click generates a new `InboundMessage` for the same session, which would try to acquire the per-session lock — deadlocking.

Fix: The `__approval` dispatch is handled **before** the per-session lock is acquired. In `process_message()`, check for `__approval` action dispatches and resolve them directly without entering `process_message_unlocked()`:

```rust
// In process_message(), before session lock acquisition:
if let Some(ref action) = msg.action {
    if action.tool == "__approval" {
        return self.resolve_approval(&msg, action).await;
    }
}
// Then acquire session lock and proceed normally
```

This bypasses the session lock entirely for approval callbacks, preventing deadlock. The `resolve_approval()` method only touches the `ApprovalStore` (its own mutex) — it doesn't need session state.

### Concurrent approvals (parallel tool calls)

When the LLM makes multiple tool calls in one iteration and more than one requires approval, each tool call is spawned as a separate `tokio::task`. Each independently:
- Registers its own oneshot in the `ApprovalStore`
- Sends its own approval request message
- Waits independently

The operator sees multiple approval request messages and can approve/deny each independently. The iteration's `join_all` waits for all tool results (including approval waits). If one is denied and others approved, the LLM receives a mix of success and error results — the same as any other mixed tool execution.

## Startup warning

When `approval.enabled = false`, log at startup for each tool with mutating actions that are NOT covered by `requires_approval_for_action()`:

```
warn!("tool 'exec' has mutating actions (execute) without approval gating")
```

Tools that DO have `requires_approval_for_action()` overrides (Gmail, Calendar, GitHub) are already gated by the legacy hard-block, so no warning for those.

Uses `ActionDescriptor.read_only` from the `actions!` macro. Runs once after all tools are registered. Skips MCP tools (separately gated by trust level). Skips tools with only read-only actions.

## Documentation updates

1. **`docs/_pages/config.html`**: New `[agents.defaults.approval]` section with field table. Include channel format examples and timeout behavior.
2. **`docs/_pages/tools.html`**: Subsection on mutating vs read-only actions and approval configuration. List which tools have mutating actions.
3. **`docs/_pages/channels.html`**: Note on configuring a channel as an approval target. Explain how to find channel IDs for Slack/Discord/Telegram.
4. **CLAUDE.md**: `ApprovalConfig`, `ApprovalStore` lifecycle, `__approval` dispatch bypasses session lock, startup warning, action standardization prerequisite.
5. **README.md**: Add "operator approval workflow" to Security features bullet.

## Testing

### Unit tests

- **ApprovalStore**: register/resolve, timeout, unknown ID (stale click), double resolve, unauthorized source channel
- **ApprovalConfig.covers()**: explicit `"tool.action"`, bare `"tool_name"`, empty list with tool ActionDescriptors, single-purpose tool with empty action param, miss
- **Startup warning**: fires for unprotected mutating tools when disabled, doesn't fire when enabled, doesn't fire for tools with `requires_approval_for_action()` overrides

### Integration tests

- **Full approval flow**: provider returns gated tool call → approval request sent → simulate approve button via ActionDispatch → tool executes → LLM gets result
- **Denial flow**: simulate deny button → `ToolResult::error` with denial message
- **Timeout flow**: no response within short timeout (1s in test) → auto-deny
- **Disabled**: tools execute without prompting when `enabled = false`; legacy hard-block still active for `requires_approval_for_action()` tools
- **Self-approval**: approval with `channel = ""` — button click on same session doesn't deadlock
- **Stale button click**: resolve with unknown ID returns error message
- **Concurrent approvals**: two tool calls requiring approval in same iteration — both get separate requests, can be resolved independently
