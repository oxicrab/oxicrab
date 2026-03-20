# Operator Approval Workflow

## Goal

Replace the current hard-block behavior for mutating tool actions with an interactive operator approval flow. When enabled, the bot pauses execution, sends an approval request with Approve/Deny buttons, and waits for a response before proceeding.

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

These tools remain single-purpose. The action name is metadata for the policy system only. No `params["action"]` dispatch is added.

## Config schema

```toml
[agents.defaults.approval]
enabled = false                    # master switch, off by default
channel = ""                       # "slack:#ops-approvals" or "" for same-conversation
timeout = 300                      # seconds to wait before auto-deny
actions = []                       # empty = all mutating actions; explicit list replaces defaults
```

`ApprovalConfig` struct in `crates/oxicrab-core/src/config/schema/agent.rs`.

### Behavior by configuration

- **`enabled = false` (default)**: No approval flow. All tools execute freely. Startup warning logged for each tool with unprotected mutating actions.
- **`enabled = true`, `actions = []`**: Interactive approval for all non-read-only actions across all tools.
- **`enabled = true`, `actions = ["google_mail.send", "exec.execute"]`**: Interactive approval only for the listed actions.
- **`channel = ""`**: Approval buttons appear in the user's own conversation (self-approval).
- **`channel = "slack:#ops-approvals"`**: Approval requests routed to a dedicated operator channel.

### Action matching

Format: `"tool_name.action_name"` (e.g., `"google_mail.send"`). A bare `"tool_name"` matches all actions for that tool. When `actions` is empty, all non-read-only actions (from `ActionDescriptor` metadata) are covered.

## Separation from MCP trust gating

Two separate concerns with separate enforcement:

1. **MCP trust** (`requires_approval() = true` on `AttenuatedMcpTool`): Hard block. Untrusted MCP tools never execute regardless of approval config.
2. **Operator approval** (`ApprovalConfig`): Interactive flow for trusted built-in tools with consequential actions.

The check order in `execute_tool_call()`:

```
1. MCP hard block: if tool.requires_approval() → error (unchanged)
2. Approval gate:  if approval_enabled && config.covers(tool, action) → interactive flow
3. Normal execution
```

`requires_approval_for_action()` overrides on Gmail/Calendar/GitHub are removed. Those tools no longer self-declare approval needs. The approval config drives everything.

## ApprovalStore

```rust
pub struct ApprovalStore {
    pending: Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>,
}

pub enum ApprovalDecision {
    Approved,
    Denied { reason: Option<String> },
}
```

Lifecycle:
1. `register(approval_id, sender)` — stores the sender
2. `resolve(approval_id, decision)` — takes the sender and fires it. Returns false if ID not found.
3. Timeout — the receiver returns `Err` when the sender is dropped (cleanup on timeout)

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
{"tool": "__approval", "params": {"approval_id": "appr-a1b2c3d4", "decision": "approved"}}
```

`__approval` is a synthetic dispatch target handled in `handle_direct_dispatch()`. It looks up the `ApprovalStore` and resolves the oneshot. Returns a confirmation message to the operator channel.

## Execution flow

### Step 1 — Gate check

In `execute_tool_call()`, after the MCP hard-block:

```rust
let action = params["action"].as_str().unwrap_or("");
if approval_config.enabled && approval_config.covers(tool_name, action) {
    return await_approval(tool, params, ctx, ...).await;
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
match tokio::time::timeout(Duration::from_secs(timeout), approval_rx).await {
    Ok(Ok(ApprovalDecision::Approved)) => tool.execute(params, ctx).await,
    Ok(Ok(ApprovalDecision::Denied { reason })) => ToolResult::error("action denied by operator"),
    _ => ToolResult::error("approval timed out — action not executed"),
}
```

### Step 5 — Button callback

Operator clicks Approve or Deny:

1. Channel receives button click → `InboundMessage` with `ActionDispatch`
2. `handle_direct_dispatch()` sees `tool: "__approval"`
3. Extracts `approval_id` and `decision`
4. Calls `approval_store.resolve(approval_id, decision)`
5. Oneshot fires → Step 4 unblocks
6. Returns confirmation to operator: "Approved google_mail.send for user1"

### What `execute_tool_call` needs

Currently a standalone function. Additional parameters needed:
- `Arc<ApprovalStore>`
- `Arc<Sender<OutboundMessage>>`
- `ApprovalConfig`

Passed via `ExecutionContext` or as additional function parameters.

## Startup warning

When `approval.enabled = false`, log at startup for each tool with unprotected mutating actions:

```
warn!("tool 'google_mail' has mutating actions (send, reply) without approval gating")
```

Uses `ActionDescriptor.read_only` from the `actions!` macro. Runs once after all tools are registered. Skips MCP tools (separately gated). Skips tools with only read-only actions.

## Documentation updates

1. **`docs/_pages/config.html`**: New `[agents.defaults.approval]` section with field table
2. **`docs/_pages/tools.html`**: Subsection on mutating vs read-only actions and approval configuration
3. **`docs/_pages/channels.html`**: Note on configuring a channel as an approval target
4. **CLAUDE.md**: `ApprovalConfig`, `ApprovalStore` lifecycle, `__approval` dispatch, startup warning
5. **README.md**: Add "operator approval workflow" to Security features bullet

## Testing

### Unit tests

- **ApprovalStore**: register/resolve, timeout, unknown ID, double resolve
- **ApprovalConfig.covers()**: explicit action, bare tool name, empty list (defaults to mutating), miss
- **Startup warning**: fires when disabled, doesn't fire when enabled

### Integration tests

- **Full approval flow**: provider returns gated tool call → approval request sent → simulate approve button → tool executes → LLM gets result
- **Denial flow**: simulate deny button → `ToolResult::error`
- **Timeout flow**: no response within short timeout → auto-deny
- **Disabled**: tools execute without prompting when `enabled = false`
