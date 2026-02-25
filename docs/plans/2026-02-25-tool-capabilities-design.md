# Tool Capability Metadata Design

**Date**: 2026-02-25
**Status**: Approved
**Scope**: Replace all hardcoded tool filtering with intrinsic capability metadata on the Tool trait

## Problem

Tool behavior is gated by five hardcoded lists scattered across the codebase:

| Hardcoded list | Location | What it gates |
|---|---|---|
| `PROTECTED_TOOL_NAMES` (14 names) | `setup.rs` | MCP tools can't shadow built-ins |
| `COMMUNITY_SAFE_KEYWORDS` (10 keywords) | `setup.rs` | Community MCP tools filtered by name |
| `default_exfil_blocked_tools` (3 names) | `config/schema/tools.rs` | Exfil guard hides tools from LLM |
| `build_subagent_tools()` (6 tools) | `subagent/mod.rs` | Hardcoded subagent tool whitelist |
| `AttenuatedMcpTool` wrapper | `mcp/proxy.rs` | Forces `requires_approval()` for untrusted MCP |

This causes bugs: subagents can't use the GitHub tool because it's not in the hardcoded whitelist, even though read-only GitHub actions (list_prs, get_issue) would be perfectly safe.

## Design

### Core Types

New types in `src/agent/tools/base.rs`:

```rust
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

#[derive(Debug, Clone)]
pub struct ActionDescriptor {
    pub name: &'static str,
    pub read_only: bool,
}

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
```

Safe defaults (deny by default):

```rust
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

One new method on the `Tool` trait:

```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities::default()
}
```

### Tool Trait Changes

- Add `fn capabilities(&self) -> ToolCapabilities` with default impl
- Change `fn description(&self) -> &'static str` to `fn description(&self) -> &str` (needed by `ReadOnlyToolWrapper` to return a dynamically generated description; all existing impls returning `&'static str` continue to compile)

### Per-Tool Declarations

**Single-purpose tools:**

| Tool | `built_in` | `network_outbound` | `subagent_access` |
|---|---|---|---|
| `read_file` | true | false | Full |
| `write_file` | true | false | Full |
| `edit_file` | true | false | Denied |
| `list_dir` | true | false | Full |
| `exec` | true | false | Full |
| `web_search` | true | true | Full |
| `web_fetch` | true | true | Full |
| `http` | true | true | Denied |
| `spawn` | true | false | Denied |
| `subagent_control` | true | false | Denied |
| `memory_search` | true | false | Denied |
| `tmux` | true | false | Denied |
| `image_gen` | true | true | Denied |
| `reddit` | true | true | ReadOnly |
| `weather` | true | true | ReadOnly |

**Action-based tools** (all `built_in: true`, `network_outbound: true`, `subagent_access: ReadOnly`):

| Tool | Read-only actions | Mutating actions |
|---|---|---|
| `github` | list_issues, get_issue, list_prs, get_pr, get_pr_files, get_file_content, get_workflow_runs, notifications | create_issue, create_pr_review, trigger_workflow |
| `google_mail` | search, read, list_labels | send, reply, label |
| `google_calendar` | list_events, get_event, list_calendars | create_event, update_event, delete_event |
| `todoist` | list_tasks, get_task, list_comments, list_projects | create_task, update_task, complete_task, delete_task, add_comment |
| `cron` | list, dlq_list | add, remove, run, dlq_replay, dlq_clear |
| `media` | search_movie, get_movie, list_movies, search_series, get_series, list_series, profiles, root_folders | add_movie, add_series |
| `obsidian` | read, search, list | write, append |
| `browser` | snapshot, get | open, click, type, fill, screenshot, eval, scroll, wait, close, navigate |

### ReadOnlyToolWrapper

New file: `src/agent/tools/read_only_wrapper.rs`

Wraps action-based tools to expose only read-only actions. Dual enforcement:

1. **Schema filtering** (belt): `parameters()` returns a modified JSON schema with the action enum filtered to read-only names only. `description()` is updated to list only available actions.
2. **Execution-time rejection** (suspenders): `execute()` checks the action parameter against the read-only set before delegating. Returns `ToolResult::error` for mutating actions.

Construction:
- Takes an `Arc<dyn Tool>`, reads `capabilities().actions`
- Extracts read-only actions
- Pre-computes filtered schema and description
- Returns `None` if no read-only actions exist

### Subagent Tool Building

`build_subagent_tools()` changes from a hardcoded whitelist to querying the main registry:

```rust
fn build_subagent_tools(main_registry: &ToolRegistry, config: &SubagentInner) -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    for (name, tool) in main_registry.iter() {
        let caps = tool.capabilities();
        match caps.subagent_access {
            SubagentAccess::Full => {
                if caps.network_outbound
                    && config.exfil_blocked_tools.contains(&name.to_string()) {
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

This requires passing the main `ToolRegistry` (or a reference) to the subagent builder, replacing the current approach of constructing tools from scratch.

### MCP Tool Handling

MCP tools' capabilities are **assigned** based on trust level, not self-reported:

| Trust | `built_in` | `requires_approval` | `subagent_access` | Registration gate |
|---|---|---|---|---|
| `local` | forced false | false (pass-through) | tool's self-report | Shadow check only |
| `verified` | forced false | forced true | forced Denied | Shadow check |
| `community` | forced false | forced true | forced Denied | Shadow check + safe-keyword heuristic |

`AttenuatedMcpTool` extends to override `capabilities()` in addition to `requires_approval()`.

Shadow protection becomes a registry lookup instead of a const list:

```rust
if let Some(existing) = tools.get(&name) {
    if existing.capabilities().built_in {
        warn!("MCP tool '{}' rejected: shadows a built-in tool", name);
        continue;
    }
}
```

The `COMMUNITY_SAFE_KEYWORDS` heuristic remains (can't trust community tools' self-reported metadata) but moves from a module-level const to a registry utility method.

### Exfiltration Guard Changes

The guard queries `network_outbound` from capabilities instead of checking a hardcoded name list:

```rust
let tools_defs = if self.exfiltration_guard.enabled {
    let allowed = &self.exfiltration_guard.allow_tools;
    tools_defs.into_iter()
        .filter(|td| {
            let caps = self.tools.get(&td.name)
                .map(|t| t.capabilities())
                .unwrap_or_default();
            !caps.network_outbound || allowed.contains(&td.name)
        })
        .collect()
} else {
    tools_defs
};
```

Config simplifies:

```rust
pub struct ExfiltrationGuardConfig {
    pub enabled: bool,
    /// Force-allow specific network_outbound tools even when guard is enabled
    #[serde(default, rename = "allowTools")]
    pub allow_tools: Vec<String>,
}
```

The old `blocked_tools` field is accepted with a deprecation warning for backwards compatibility.

### ToolRegistry Additions

```rust
impl ToolRegistry {
    /// Sorted list of all registered tool names.
    pub fn tool_names(&self) -> Vec<String>;

    /// Iterate over all registered tools.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Arc<dyn Tool>)>;
}
```

### What Gets Deleted

- `PROTECTED_TOOL_NAMES` const (14 entries)
- `default_exfil_blocked_tools()` function
- `ExfiltrationGuardConfig.blocked_tools` field (replaced by `allow_tools`)
- `COMMUNITY_SAFE_KEYWORDS` const (moves to method)
- Hardcoded tool construction in `build_subagent_tools()` body
- Direct imports of individual tool types in `subagent/mod.rs`

## Testing Strategy

### Contract Tests (per-tool)

Every tool gets a test verifying its capabilities declaration:

```rust
#[test]
fn test_github_capabilities() {
    let tool = GitHubTool::new("fake-token".to_string());
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::ReadOnly);
    // verify specific actions...
}
```

### Completeness Tests (action-based tools)

Verify capabilities actions match the schema action enum exactly:

```rust
#[test]
fn test_github_actions_match_schema() {
    // Extract action enum from parameters() JSON
    // Extract actions from capabilities()
    // Assert they match exactly — catches forgotten declarations
}
```

Every action-based tool gets this test. This is the key safety net against someone adding a new action to the schema without declaring it in capabilities.

### ReadOnlyToolWrapper Tests

- Wrapper filters action enum in parameters schema
- Wrapper rejects mutating actions at execute() time
- Wrapper allows read-only actions through to inner tool
- Wrapper returns None when no read-only actions exist

### Integration Tests

- Subagent registry built from capabilities contains expected tools
- Exfil guard filters by `network_outbound` capability
- Exfil guard `allow_tools` overrides work
- MCP shadow protection uses `built_in` capability
- Regression: subagent has github with read-only actions only

## Security Analysis

**Strengths:**
- Defaults are deny-all for subagents (`SubagentAccess::Denied`) — tools must explicitly opt in
- Dual enforcement: schema filtering + execution-time rejection
- MCP tools get capabilities assigned (not self-reported) based on trust level
- `built_in` flag prevents MCP tools from claiming built-in status
- All enforcement is in Rust code, not LLM prompts — can't be prompt-injected around

**Attack surface:**
- A tool author incorrectly marks a mutating action as `read_only: true` — mitigated by completeness tests and code review
- A local-trust MCP tool lies about its capabilities — accepted risk (local trust means the user trusts the server)

## Migration

Incremental rollout:
1. Add types and trait method (backwards compatible — default impl)
2. Annotate all built-in tools with capabilities
3. Add completeness tests
4. Implement ReadOnlyToolWrapper
5. Rewire subagent builder to use capabilities
6. Rewire exfil guard to use capabilities
7. Rewire MCP shadow protection to use capabilities
8. Delete hardcoded lists
