# Tool Capability Metadata Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace all hardcoded tool filtering lists with intrinsic capability metadata on the Tool trait, enabling subagents to access read-only actions of domain tools (e.g., GitHub list_prs).

**Architecture:** Add `ToolCapabilities` struct and `capabilities()` method to the `Tool` trait. Each tool declares its own metadata (built-in, network access, subagent access, per-action read-only flags). A `ReadOnlyToolWrapper` provides dual enforcement (schema filtering + execution-time rejection). All consumers (subagent builder, exfil guard, MCP shadow protection) query capabilities instead of hardcoded lists.

**Tech Stack:** Rust, async_trait, serde_json (for schema manipulation)

**Design doc:** `docs/plans/2026-02-25-tool-capabilities-design.md`

---

### Task 1: Core Types and Trait Method

Add `ToolCapabilities`, `ActionDescriptor`, `SubagentAccess` types and the `capabilities()` method to the `Tool` trait. Change `description()` return type from `&'static str` to `&str`.

**Files:**
- Modify: `src/agent/tools/base.rs:73-121` (Tool trait + new types)
- Modify: `src/agent/tools/mod.rs:26` (re-export new types)
- Modify: `src/agent/tools/mcp/proxy.rs:177` (AttenuatedMcpTool description() signature)

**Step 1: Write the failing test**

In `src/agent/tools/base.rs`, add at the bottom (before any existing test module, or create one):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_capabilities_are_deny_all() {
        let caps = ToolCapabilities::default();
        assert!(!caps.built_in);
        assert!(!caps.network_outbound);
        assert_eq!(caps.subagent_access, SubagentAccess::Denied);
        assert!(caps.actions.is_empty());
    }

    #[test]
    fn test_subagent_access_equality() {
        assert_eq!(SubagentAccess::Full, SubagentAccess::Full);
        assert_ne!(SubagentAccess::Full, SubagentAccess::ReadOnly);
        assert_ne!(SubagentAccess::ReadOnly, SubagentAccess::Denied);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib tools::base::tests -- --test-threads=1`
Expected: FAIL — `ToolCapabilities` and `SubagentAccess` not defined

**Step 3: Write minimal implementation**

In `src/agent/tools/base.rs`, add before the `Tool` trait definition (before line 73):

```rust
/// How a tool should be exposed in subagent contexts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentAccess {
    /// All actions available (e.g., read_file, exec, write_file)
    Full,
    /// Only read-only actions exposed; mutating actions hidden from schema
    /// and rejected at execution time
    ReadOnly,
    /// Tool not available to subagents at all (e.g., spawn, cron)
    Denied,
}

/// Per-action metadata for tools using the action dispatch pattern.
#[derive(Debug, Clone)]
pub struct ActionDescriptor {
    /// Action name matching the `"action"` enum value in `parameters()`
    pub name: &'static str,
    /// Whether this action only reads data (no side effects)
    pub read_only: bool,
}

/// Capability metadata intrinsic to a tool. Queried by the registry,
/// subagent builder, exfiltration guard, and MCP trust filter.
#[derive(Debug, Clone)]
pub struct ToolCapabilities {
    /// Tool is built-in to oxicrab. Protected from MCP shadowing.
    pub built_in: bool,
    /// Tool's primary purpose involves outbound network requests.
    /// Used by exfiltration guard to determine default blocking.
    pub network_outbound: bool,
    /// How this tool should be exposed in subagent contexts.
    pub subagent_access: SubagentAccess,
    /// Per-action metadata. Empty for single-purpose tools.
    /// For action-based tools, every action MUST be listed.
    pub actions: Vec<ActionDescriptor>,
}

impl Default for ToolCapabilities {
    fn default() -> Self {
        Self {
            built_in: false,
            network_outbound: false,
            subagent_access: SubagentAccess::Denied,
            actions: vec![],
        }
    }
}
```

Add to the `Tool` trait (after `execution_timeout()`):

```rust
    /// Capability metadata for this tool. Used by subagent builder,
    /// exfiltration guard, and MCP trust filter.
    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::default()
    }
```

Change `description()` signature from:
```rust
    fn description(&self) -> &'static str;
```
to:
```rust
    fn description(&self) -> &str;
```

Update `src/agent/tools/mod.rs` line 26 to re-export new types:
```rust
pub use base::{ActionDescriptor, ExecutionContext, SubagentAccess, Tool, ToolCapabilities, ToolMiddleware, ToolResult, ToolVersion};
```

Update `src/agent/tools/mcp/proxy.rs` line 177 — change `AttenuatedMcpTool::description()`:
```rust
    fn description(&self) -> &str {
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib tools::base::tests -- --test-threads=1`
Expected: PASS

**Step 5: Run full build + clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: PASS — the `&'static str` → `&str` change is backwards compatible

**Step 6: Commit**

```bash
git add src/agent/tools/base.rs src/agent/tools/mod.rs src/agent/tools/mcp/proxy.rs
git commit -m "feat(tools): add ToolCapabilities types and capabilities() trait method"
```

---

### Task 2: Annotate Single-Purpose Tools

Add `capabilities()` impl to all 15 single-purpose tools. Each returns a `ToolCapabilities` with the correct flags per the design doc tables. No actions list needed (single-purpose tools have `actions: vec![]`).

**Files:**
- Modify: `src/agent/tools/filesystem/mod.rs` (read_file, write_file, edit_file, list_dir — 4 Tool impls)
- Modify: `src/agent/tools/shell/mod.rs` (exec)
- Modify: `src/agent/tools/web/mod.rs` (web_search, web_fetch)
- Modify: `src/agent/tools/http/mod.rs` (http)
- Modify: `src/agent/tools/spawn.rs` (spawn)
- Modify: `src/agent/tools/subagent_control.rs` (subagent_control)
- Modify: `src/agent/tools/memory_search/mod.rs` (memory_search)
- Modify: `src/agent/tools/tmux/mod.rs` (tmux)
- Modify: `src/agent/tools/image_gen/mod.rs` (image_gen)
- Modify: `src/agent/tools/reddit/mod.rs` (reddit — all actions read-only)
- Modify: `src/agent/tools/weather/mod.rs` (weather — all actions read-only)

**Step 1: Write failing tests**

Add a test per tool in its existing test module. Example pattern for `read_file` in `src/agent/tools/filesystem/mod.rs` tests:

```rust
#[test]
fn test_read_file_capabilities() {
    use crate::agent::tools::base::SubagentAccess;
    let tool = ReadFileTool::new(None, None);
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(!caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::Full);
    assert!(caps.actions.is_empty());
}
```

Values per tool (from design doc):

| Tool | `built_in` | `network_outbound` | `subagent_access` |
|---|---|---|---|
| read_file | true | false | Full |
| write_file | true | false | Full |
| edit_file | true | false | Denied |
| list_dir | true | false | Full |
| exec | true | false | Full |
| web_search | true | true | Full |
| web_fetch | true | true | Full |
| http | true | true | Denied |
| spawn | true | false | Denied |
| subagent_control | true | false | Denied |
| memory_search | true | false | Denied |
| tmux | true | false | Denied |
| image_gen | true | true | Denied |
| reddit | true | true | ReadOnly |
| weather | true | true | ReadOnly |

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib capabilities -- --test-threads=1`
Expected: FAIL — all return defaults (built_in: false, SubagentAccess::Denied)

**Step 3: Implement capabilities() on each tool**

Add to each tool's `impl Tool for ...` block. Pattern:

```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        network_outbound: false,  // or true for web/network tools
        subagent_access: SubagentAccess::Full,  // or ReadOnly/Denied per table
        actions: vec![],
    }
}
```

Import needed in each file: `use crate::agent::tools::base::{SubagentAccess, ToolCapabilities};`

For reddit and weather (ReadOnly with all-read-only actions), declare actions to be consistent:

```rust
// reddit
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        network_outbound: true,
        subagent_access: SubagentAccess::ReadOnly,
        actions: vec![
            ActionDescriptor { name: "hot", read_only: true },
            ActionDescriptor { name: "new", read_only: true },
            ActionDescriptor { name: "top", read_only: true },
            ActionDescriptor { name: "search", read_only: true },
        ],
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib capabilities -- --test-threads=1`
Expected: PASS

**Step 5: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: PASS

**Step 6: Commit**

```bash
git add src/agent/tools/filesystem/ src/agent/tools/shell/ src/agent/tools/web/ \
  src/agent/tools/http/ src/agent/tools/spawn.rs src/agent/tools/subagent_control.rs \
  src/agent/tools/memory_search/ src/agent/tools/tmux/ src/agent/tools/image_gen/ \
  src/agent/tools/reddit/ src/agent/tools/weather/
git commit -m "feat(tools): annotate single-purpose tools with capabilities"
```

---

### Task 3: Annotate Action-Based Tools with Per-Action Metadata

Add `capabilities()` with `ActionDescriptor` lists to all 8 action-based tools. Add completeness tests verifying every action in the parameters schema is declared in capabilities.

**Files:**
- Modify: `src/agent/tools/github/mod.rs`
- Modify: `src/agent/tools/google_mail.rs`
- Modify: `src/agent/tools/google_calendar.rs`
- Modify: `src/agent/tools/todoist/mod.rs`
- Modify: `src/agent/tools/cron/mod.rs`
- Modify: `src/agent/tools/media/mod.rs`
- Modify: `src/agent/tools/obsidian/mod.rs`
- Modify: `src/agent/tools/browser/mod.rs`

**Step 1: Write failing tests — capabilities contract + completeness**

For each tool, add TWO tests. Example for github:

```rust
#[test]
fn test_github_capabilities() {
    use crate::agent::tools::base::SubagentAccess;
    let tool = GitHubTool::new("fake".to_string());
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::ReadOnly);

    let read_only: Vec<&str> = caps.actions.iter()
        .filter(|a| a.read_only).map(|a| a.name).collect();
    let mutating: Vec<&str> = caps.actions.iter()
        .filter(|a| !a.read_only).map(|a| a.name).collect();

    assert!(read_only.contains(&"list_prs"));
    assert!(read_only.contains(&"get_issue"));
    assert!(mutating.contains(&"create_issue"));
    assert!(mutating.contains(&"trigger_workflow"));
}

#[test]
fn test_github_actions_match_schema() {
    let tool = GitHubTool::new("fake".to_string());
    let caps = tool.capabilities();
    let params = tool.parameters();

    let schema_actions: Vec<String> = params["properties"]["action"]["enum"]
        .as_array().unwrap()
        .iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let cap_actions: Vec<String> = caps.actions.iter()
        .map(|a| a.name.to_string()).collect();

    for action in &schema_actions {
        assert!(cap_actions.contains(action),
            "action '{}' in schema but not in capabilities()", action);
    }
    for action in &cap_actions {
        assert!(schema_actions.contains(action),
            "action '{}' in capabilities() but not in schema", action);
    }
}
```

Action classifications (from design doc):

- **github**: read_only=[list_issues, get_issue, list_prs, get_pr, get_pr_files, get_file_content, get_workflow_runs, notifications], mutating=[create_issue, create_pr_review, trigger_workflow]
- **google_mail**: read_only=[search, read, list_labels], mutating=[send, reply, label]
- **google_calendar**: read_only=[list_events, get_event, list_calendars], mutating=[create_event, update_event, delete_event]
- **todoist**: read_only=[list_tasks, get_task, list_comments, list_projects], mutating=[create_task, update_task, complete_task, delete_task, add_comment]
- **cron**: read_only=[list, dlq_list], mutating=[add, remove, run, dlq_replay, dlq_clear]
- **media**: read_only=[search_movie, get_movie, list_movies, search_series, get_series, list_series, profiles, root_folders], mutating=[add_movie, add_series]
- **obsidian**: read_only=[read, search, list], mutating=[write, append]
- **browser**: read_only=[snapshot, get], mutating=[open, click, type, fill, screenshot, eval, scroll, wait, close, navigate]

All are: `built_in: true`, `network_outbound: true`, `subagent_access: SubagentAccess::ReadOnly`

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib actions_match_schema -- --test-threads=1`
Expected: FAIL

**Step 3: Implement capabilities() on each tool**

Pattern for github:

```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: true,
        network_outbound: true,
        subagent_access: SubagentAccess::ReadOnly,
        actions: vec![
            ActionDescriptor { name: "list_issues", read_only: true },
            ActionDescriptor { name: "create_issue", read_only: false },
            ActionDescriptor { name: "get_issue", read_only: true },
            ActionDescriptor { name: "list_prs", read_only: true },
            ActionDescriptor { name: "get_pr", read_only: true },
            ActionDescriptor { name: "get_pr_files", read_only: true },
            ActionDescriptor { name: "create_pr_review", read_only: false },
            ActionDescriptor { name: "get_file_content", read_only: true },
            ActionDescriptor { name: "trigger_workflow", read_only: false },
            ActionDescriptor { name: "get_workflow_runs", read_only: true },
            ActionDescriptor { name: "notifications", read_only: true },
        ],
    }
}
```

Repeat for all 8 tools with their respective action lists.

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib capabilities -- --test-threads=1` and `cargo test --lib actions_match_schema -- --test-threads=1`
Expected: PASS

**Step 5: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: PASS

**Step 6: Commit**

```bash
git add src/agent/tools/github/ src/agent/tools/google_mail.rs \
  src/agent/tools/google_calendar.rs src/agent/tools/todoist/ \
  src/agent/tools/cron/ src/agent/tools/media/ \
  src/agent/tools/obsidian/ src/agent/tools/browser/
git commit -m "feat(tools): annotate action-based tools with per-action capabilities"
```

---

### Task 4: ReadOnlyToolWrapper

Implement the dual-enforcement wrapper that exposes only read-only actions.

**Files:**
- Create: `src/agent/tools/read_only_wrapper.rs`
- Modify: `src/agent/tools/mod.rs` (add `pub mod read_only_wrapper;`)

**Step 1: Write failing tests**

Create `src/agent/tools/read_only_wrapper.rs` with a test module. Use a mock tool for testing:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::base::{ActionDescriptor, ExecutionContext, SubagentAccess, ToolCapabilities};

    struct MockActionTool;

    #[async_trait::async_trait]
    impl Tool for MockActionTool {
        fn name(&self) -> &str { "mock_action" }
        fn description(&self) -> &str {
            "Mock tool. Actions: read_data, write_data, delete_data."
        }
        fn parameters(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["read_data", "write_data", "delete_data"]
                    }
                },
                "required": ["action"]
            })
        }
        async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
            let action = params["action"].as_str().unwrap_or("unknown");
            Ok(ToolResult::new(format!("executed: {}", action)))
        }
        fn capabilities(&self) -> ToolCapabilities {
            ToolCapabilities {
                built_in: true,
                network_outbound: true,
                subagent_access: SubagentAccess::ReadOnly,
                actions: vec![
                    ActionDescriptor { name: "read_data", read_only: true },
                    ActionDescriptor { name: "write_data", read_only: false },
                    ActionDescriptor { name: "delete_data", read_only: false },
                ],
            }
        }
    }

    #[test]
    fn test_wrapper_filters_action_enum() {
        let tool = Arc::new(MockActionTool);
        let wrapper = ReadOnlyToolWrapper::new(tool).unwrap();
        let params = wrapper.parameters();
        let actions: Vec<String> = params["properties"]["action"]["enum"]
            .as_array().unwrap()
            .iter().map(|v| v.as_str().unwrap().to_string()).collect();
        assert_eq!(actions, vec!["read_data"]);
    }

    #[test]
    fn test_wrapper_updates_description() {
        let tool = Arc::new(MockActionTool);
        let wrapper = ReadOnlyToolWrapper::new(tool).unwrap();
        let desc = wrapper.description();
        assert!(desc.contains("read_data"), "description should list read-only actions");
        assert!(!desc.contains("write_data"), "description should not list mutating actions");
    }

    #[tokio::test]
    async fn test_wrapper_rejects_mutating_action() {
        let tool = Arc::new(MockActionTool);
        let wrapper = ReadOnlyToolWrapper::new(tool).unwrap();
        let ctx = ExecutionContext::default();
        let result = wrapper.execute(
            serde_json::json!({"action": "write_data"}), &ctx
        ).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not available"));
    }

    #[tokio::test]
    async fn test_wrapper_allows_read_only_action() {
        let tool = Arc::new(MockActionTool);
        let wrapper = ReadOnlyToolWrapper::new(tool).unwrap();
        let ctx = ExecutionContext::default();
        let result = wrapper.execute(
            serde_json::json!({"action": "read_data"}), &ctx
        ).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("executed: read_data"));
    }

    #[test]
    fn test_wrapper_returns_none_for_all_mutating() {
        struct AllMutatingTool;
        #[async_trait::async_trait]
        impl Tool for AllMutatingTool {
            fn name(&self) -> &str { "all_mutating" }
            fn description(&self) -> &str { "test" }
            fn parameters(&self) -> Value { serde_json::json!({}) }
            async fn execute(&self, _: Value, _: &ExecutionContext) -> anyhow::Result<ToolResult> {
                Ok(ToolResult::new(""))
            }
            fn capabilities(&self) -> ToolCapabilities {
                ToolCapabilities {
                    built_in: true,
                    network_outbound: false,
                    subagent_access: SubagentAccess::ReadOnly,
                    actions: vec![
                        ActionDescriptor { name: "delete", read_only: false },
                    ],
                }
            }
        }
        let tool = Arc::new(AllMutatingTool);
        assert!(ReadOnlyToolWrapper::new(tool).is_none());
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib read_only_wrapper -- --test-threads=1`
Expected: FAIL — `ReadOnlyToolWrapper` not defined

**Step 3: Implement ReadOnlyToolWrapper**

```rust
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Wraps an action-based tool to expose only its read-only actions.
/// Dual enforcement: schema filtering + execution-time rejection.
pub struct ReadOnlyToolWrapper {
    inner: Arc<dyn Tool>,
    read_only_actions: Vec<&'static str>,
    filtered_schema: Value,
    filtered_description: String,
}

impl ReadOnlyToolWrapper {
    /// Create a read-only wrapper. Returns `None` if the tool has no read-only actions.
    pub fn new(tool: Arc<dyn Tool>) -> Option<Self> {
        let caps = tool.capabilities();
        let read_only_actions: Vec<&'static str> = caps
            .actions
            .iter()
            .filter(|a| a.read_only)
            .map(|a| a.name)
            .collect();

        if read_only_actions.is_empty() {
            return None;
        }

        let filtered_schema = filter_action_enum(&tool.parameters(), &read_only_actions);
        let base_desc = tool.description().split(". Actions:").next()
            .unwrap_or(tool.description());
        let filtered_description = format!(
            "{} (read-only actions: {})",
            base_desc.trim_end_matches('.'),
            read_only_actions.join(", ")
        );

        Some(Self {
            inner: tool,
            read_only_actions,
            filtered_schema,
            filtered_description,
        })
    }
}

/// Filter the action enum in a parameters JSON schema to only include allowed actions.
fn filter_action_enum(schema: &Value, allowed: &[&str]) -> Value {
    let mut filtered = schema.clone();
    if let Some(action_enum) = filtered
        .get_mut("properties")
        .and_then(|p| p.get_mut("action"))
        .and_then(|a| a.get_mut("enum"))
    {
        if let Some(arr) = action_enum.as_array() {
            let kept: Vec<Value> = arr
                .iter()
                .filter(|v| v.as_str().map_or(false, |s| allowed.contains(&s)))
                .cloned()
                .collect();
            *action_enum = Value::Array(kept);
        }
    }
    filtered
}

#[async_trait]
impl Tool for ReadOnlyToolWrapper {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        &self.filtered_description
    }

    fn parameters(&self) -> Value {
        self.filtered_schema.clone()
    }

    async fn execute(&self, params: Value, ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        if let Some(action) = params.get("action").and_then(|a| a.as_str()) {
            if !self.read_only_actions.contains(&action) {
                return Ok(ToolResult::error(format!(
                    "action '{}' is not available in this context (read-only access)",
                    action
                )));
            }
        }
        self.inner.execute(params, ctx).await
    }

    fn capabilities(&self) -> ToolCapabilities {
        let mut caps = self.inner.capabilities();
        caps.subagent_access = SubagentAccess::Full; // already filtered
        caps
    }

    fn cacheable(&self) -> bool {
        self.inner.cacheable()
    }

    fn requires_approval(&self) -> bool {
        self.inner.requires_approval()
    }

    fn execution_timeout(&self) -> std::time::Duration {
        self.inner.execution_timeout()
    }
}
```

Add `pub mod read_only_wrapper;` to `src/agent/tools/mod.rs`.

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib read_only_wrapper -- --test-threads=1`
Expected: PASS

**Step 5: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

**Step 6: Commit**

```bash
git add src/agent/tools/read_only_wrapper.rs src/agent/tools/mod.rs
git commit -m "feat(tools): add ReadOnlyToolWrapper with dual enforcement"
```

---

### Task 5: ToolRegistry iter() and Rewire Subagent Builder

Add `iter()` method to `ToolRegistry`. Rewrite `build_subagent_tools()` to query capabilities from the main registry. Add `main_tools: Arc<ToolRegistry>` to `SubagentConfig`.

**Files:**
- Modify: `src/agent/tools/registry/mod.rs` (add `iter()`)
- Modify: `src/agent/subagent/mod.rs` (rewrite `build_subagent_tools()`, update `SubagentConfig`/`SubagentInner`)
- Modify: `src/agent/subagent/tests.rs` (update tests for new builder)
- Modify: `src/agent/loop/mod.rs:428-447` (pass main tools to `SubagentConfig`)
- Modify: `src/agent/tools/setup.rs` (pass registry to subagent config after building)
- Modify: `tests/common/mod.rs` (update `create_test_agent_with()`)
- Modify: `tests/compaction_integration.rs` (update `create_compaction_agent()`)

**Step 1: Add `iter()` to `ToolRegistry`**

In `src/agent/tools/registry/mod.rs`, add:

```rust
pub fn iter(&self) -> impl Iterator<Item = (&str, &Arc<dyn Tool>)> {
    self.tools.iter().map(|(k, v)| (k.as_str(), v))
}
```

**Step 2: Update `SubagentConfig` and `SubagentInner`**

Add `main_tools: Arc<ToolRegistry>` to `SubagentConfig` (line ~34) and `SubagentInner` (line ~64). This is the main agent's registry, used to build the subagent's filtered tools.

**Step 3: Rewrite `build_subagent_tools()`**

Replace the current function body with capability-based filtering:

```rust
fn build_subagent_tools(config: &SubagentInner) -> ToolRegistry {
    use crate::agent::tools::read_only_wrapper::ReadOnlyToolWrapper;

    let mut tools = ToolRegistry::new();
    for (name, tool) in config.main_tools.iter() {
        let caps = tool.capabilities();
        match caps.subagent_access {
            SubagentAccess::Full => {
                // Respect exfil guard for network-outbound tools
                if caps.network_outbound
                    && config.exfil_blocked_tools.contains(&name.to_string())
                {
                    continue;
                }
                tools.register(tool.clone());
            }
            SubagentAccess::ReadOnly => {
                if let Some(wrapped) = ReadOnlyToolWrapper::new(tool.clone()) {
                    tools.register(Arc::new(wrapped));
                }
            }
            SubagentAccess::Denied => {}
        }
    }
    tools
}
```

Remove the old tool construction imports (ReadFileTool, WriteFileTool, etc.) from the top of `subagent/mod.rs`.

**Step 4: Update `AgentLoop::new()` and `register_all_tools()`**

In `src/agent/loop/mod.rs`, after `register_all_tools()` returns, pass the main `ToolRegistry` to `SubagentConfig`. The exact wiring depends on current code structure — `SubagentConfig` is built in the `ToolBuildContext` before `register_all_tools()` runs, so you'll need to either:
- Set `main_tools` on the `SubagentManager` after `register_all_tools()` returns, OR
- Build `SubagentConfig` after `register_all_tools()`, OR
- Add an `Arc<ToolRegistry>` field that's set later

The cleanest approach: After `register_all_tools()` returns `(tools, subagents, mcp)`, call a new method `subagents.set_main_tools(tools.clone())` that stores the registry reference. Subagent tools are built lazily on first `spawn()`.

**Step 5: Update tests**

Update `src/agent/subagent/tests.rs` — the `make_manager()` and `make_inner()` helpers need to provide `main_tools`. Create a minimal registry for tests:

```rust
fn make_test_registry() -> Arc<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    // Register tools with known capabilities for testing
    registry.register(Arc::new(ReadFileTool::new(None, None)));
    // ... etc
    Arc::new(registry)
}
```

Update `test_subagent_tools_default_set` to verify that subagent tools now include read-only versions of action-based tools (e.g., github with filtered actions).

Update `tests/common/mod.rs` `create_test_agent_with()` and `tests/compaction_integration.rs` `create_compaction_agent()` to add `main_tools` to `SubagentConfig`.

**Step 6: Run tests**

Run: `cargo test --lib subagent -- --test-threads=1`
Then: `cargo test -- --test-threads=1` (full suite)
Expected: PASS

**Step 7: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

**Step 8: Commit**

```bash
git add src/agent/tools/registry/ src/agent/subagent/ src/agent/loop/ \
  src/agent/tools/setup.rs tests/
git commit -m "feat(subagent): build subagent tools from capabilities instead of hardcoded list"
```

---

### Task 6: Rewire Exfiltration Guard

Replace the hardcoded `blocked_tools` list with capability-based `network_outbound` filtering.

**Files:**
- Modify: `src/config/schema/tools.rs:5-25` (ExfiltrationGuardConfig)
- Modify: `src/agent/loop/mod.rs:1037-1049` (exfil guard filtering)
- Modify: `tests/common/mod.rs` (if ExfiltrationGuardConfig fields change)

**Step 1: Write failing test**

Add a test in `src/agent/loop/tests.rs` (or wherever loop tests live):

```rust
#[test]
fn test_exfil_guard_blocks_network_outbound_tools() {
    // Build a registry with network_outbound: true and false tools
    // Apply exfil guard filtering
    // Verify only non-network tools pass through
}
```

**Step 2: Update `ExfiltrationGuardConfig`**

In `src/config/schema/tools.rs`, add `allow_tools` field and keep `blocked_tools` for backwards compat:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExfiltrationGuardConfig {
    #[serde(default)]
    pub enabled: bool,
    /// DEPRECATED: Use network_outbound capability on tools instead.
    /// Kept for backwards compatibility — treated as extra blocked tools.
    #[serde(default, rename = "blockedTools")]
    pub blocked_tools: Vec<String>,
    /// Force-allow specific network_outbound tools when guard is enabled.
    #[serde(default, rename = "allowTools")]
    pub allow_tools: Vec<String>,
}
```

**Step 3: Update exfil guard filtering in agent loop**

In `src/agent/loop/mod.rs`, replace lines 1037-1049 with:

```rust
let tools_defs = if self.exfiltration_guard.enabled {
    let allowed = &self.exfiltration_guard.allow_tools;
    let extra_blocked = &self.exfiltration_guard.blocked_tools;
    tools_defs
        .into_iter()
        .filter(|td| {
            // Backwards compat: if tool is in legacy blocked_tools, block it
            if extra_blocked.contains(&td.name) {
                return false;
            }
            let is_network = self
                .tools
                .get(&td.name)
                .map(|t| t.capabilities().network_outbound)
                .unwrap_or(false);
            !is_network || allowed.contains(&td.name)
        })
        .collect()
} else {
    tools_defs
};
```

**Step 4: Run tests**

Run: `cargo test -- --test-threads=1`
Expected: PASS

**Step 5: Commit**

```bash
git add src/config/schema/tools.rs src/agent/loop/
git commit -m "feat(exfil): replace blocked_tools list with network_outbound capability"
```

---

### Task 7: Rewire MCP Shadow Protection and AttenuatedMcpTool

Replace `PROTECTED_TOOL_NAMES` with `built_in` capability. Extend `AttenuatedMcpTool` to override `capabilities()`.

**Files:**
- Modify: `src/agent/tools/setup.rs:17-32, 359-364, 379-410` (delete consts, update `create_mcp()`)
- Modify: `src/agent/tools/mcp/proxy.rs:160-200` (extend `AttenuatedMcpTool`)

**Step 1: Write failing tests**

In `src/agent/tools/setup.rs` tests:

```rust
#[test]
fn test_mcp_shadow_uses_built_in_capability() {
    // Register a built-in tool, try to register MCP tool with same name
    // Verify it's rejected based on capabilities().built_in
}
```

In `src/agent/tools/mcp/proxy.rs` tests:

```rust
#[test]
fn test_attenuated_mcp_capabilities_override() {
    // Verify AttenuatedMcpTool forces built_in: false, subagent_access: Denied
}
```

**Step 2: Extend `AttenuatedMcpTool`**

Add `assigned_subagent_access: SubagentAccess` field. Update constructor:

```rust
pub fn new(inner: Arc<dyn Tool>, subagent_access: SubagentAccess) -> Self {
    Self { inner, assigned_subagent_access: subagent_access }
}
```

Add `capabilities()` override:

```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        built_in: false,
        network_outbound: true,
        subagent_access: self.assigned_subagent_access,
        actions: vec![],
    }
}
```

Update `description()` return type to `&str`.

**Step 3: Update `create_mcp()` in `setup.rs`**

Replace shadow check:
```rust
// Old: if PROTECTED_TOOL_NAMES.contains(...)
// New:
if let Some(existing) = tools.get(&name) {
    if existing.capabilities().built_in {
        warn!("MCP tool '{}' rejected: shadows a built-in tool", name);
        continue;
    }
}
```

Update `AttenuatedMcpTool::new()` calls to pass `SubagentAccess::Denied`.

Move `COMMUNITY_SAFE_KEYWORDS` to a function (no longer a module-level const):

```rust
fn is_community_safe(tool_name: &str) -> bool {
    const SAFE_KEYWORDS: &[&str] = &[
        "read", "list", "get", "search", "find", "query", "fetch", "view", "show", "count",
    ];
    let name_lower = tool_name.to_lowercase();
    SAFE_KEYWORDS.iter().any(|kw| name_lower.contains(kw))
}
```

Delete `PROTECTED_TOOL_NAMES` const and `COMMUNITY_SAFE_KEYWORDS` const.

**Step 4: Run tests**

Run: `cargo test -- --test-threads=1`
Expected: PASS

**Step 5: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

**Step 6: Commit**

```bash
git add src/agent/tools/setup.rs src/agent/tools/mcp/proxy.rs
git commit -m "feat(mcp): use built_in capability for shadow protection, extend AttenuatedMcpTool"
```

---

### Task 8: Delete Hardcoded Lists and Final Cleanup

Remove all remaining hardcoded list artifacts. Delete `default_exfil_blocked_tools()`. Remove old tool-construction imports from subagent module. Update existing tests.

**Files:**
- Modify: `src/config/schema/tools.rs` (remove `default_exfil_blocked_tools()`)
- Modify: `src/agent/subagent/mod.rs` (remove unused tool imports)
- Modify: `src/agent/tools/setup.rs` (verify consts are gone, update tests)
- Modify: `src/config/schema/tests.rs` (update config example test if needed)

**Step 1: Delete `default_exfil_blocked_tools()`**

In `src/config/schema/tools.rs`, remove the function and update `ExfiltrationGuardConfig::default()`:

```rust
impl Default for ExfiltrationGuardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            blocked_tools: vec![],  // legacy field, empty by default
            allow_tools: vec![],
        }
    }
}
```

**Step 2: Remove old imports from subagent/mod.rs**

Remove these imports (no longer needed since tools come from main registry):
```rust
use crate::agent::tools::filesystem::{ListDirTool, ReadFileTool, WriteFileTool};
use crate::agent::tools::shell::ExecTool;
use crate::agent::tools::web::{WebFetchTool, WebSearchTool};
```

**Step 3: Verify PROTECTED_TOOL_NAMES and COMMUNITY_SAFE_KEYWORDS are gone**

Grep for them — should find no references:

```bash
grep -r "PROTECTED_TOOL_NAMES\|COMMUNITY_SAFE_KEYWORDS" src/
```

**Step 4: Update config example test**

If `test_config_example_is_up_to_date` in `src/config/schema/tests.rs` needs updating for the new `ExfiltrationGuardConfig` fields, update `config.example.json` accordingly.

**Step 5: Run full test suite**

Run: `cargo test -- --test-threads=1`
Expected: PASS

**Step 6: Run clippy + fmt**

Run: `cargo fmt -- --check && cargo clippy --all-targets --all-features -- -D warnings`
Expected: PASS

**Step 7: Commit**

```bash
git add src/ tests/ config.example.json
git commit -m "refactor(tools): delete hardcoded tool lists, complete capability migration"
```

---

## Verification Checklist (run after all tasks)

```bash
# Full test suite
cargo test -- --test-threads=1

# Clippy clean
cargo clippy --all-targets --all-features -- -D warnings

# Format clean
cargo fmt -- --check

# Grep for deleted artifacts (should return nothing)
grep -r "PROTECTED_TOOL_NAMES" src/
grep -r "COMMUNITY_SAFE_KEYWORDS" src/
grep -r "default_exfil_blocked_tools" src/

# Verify subagent now has github read-only
cargo test --lib test_subagent_has_github_read_only
```
