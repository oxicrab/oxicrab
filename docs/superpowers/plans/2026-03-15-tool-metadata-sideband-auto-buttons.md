# Tool Metadata Sideband & Auto-Buttons Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a generic structured metadata channel to tool results (invisible to the LLM) and use it to automatically attach interactive buttons to task tool responses, eliminating reliance on the LLM calling `add_buttons`.

**Architecture:** `ToolResult` gains an `Option<HashMap<String, Value>>` metadata field that flows through the execution pipeline but is stripped before entering LLM context. Tools populate it in `execute()`. After the agent loop, a consumer merges `suggested_buttons` from tool metadata with any LLM-added buttons (tool-suggested are unconditional and take priority). Button click feedback is added to Slack's interactive payload handler.

**Tech Stack:** Rust, serde_json, existing ToolResult/ToolRegistry/AgentLoop infrastructure.

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `src/agent/tools/base/mod.rs` | Modify | Add `metadata` field to `ToolResult`, builder method |
| `src/agent/loop/helpers.rs` | Modify | Return `ToolResult` from `execute_tool_call()` instead of tuple |
| `src/agent/loop/iteration.rs` | Modify | Collect tool metadata sideband, merge into response_metadata |
| `src/agent/tools/todoist/mod.rs` | Modify | Add `suggested_buttons` to `list_tasks`/`get_task` results |
| `src/agent/tools/google_tasks/mod.rs` | Modify | Add `suggested_buttons` to `list_tasks`/`get_task` results |
| `src/channels/slack/mod.rs` | Modify | Add thinking emoji to `handle_interactive_payload()` |

---

## Chunk 1: ToolResult Metadata Sideband

### Task 1: Add metadata field to ToolResult

**Files:**
- Modify: `src/agent/tools/base/mod.rs:4-34`
- Test: `src/agent/tools/base/tests.rs`

- [ ] **Step 1: Write failing tests for ToolResult metadata**

In `src/agent/tools/base/tests.rs`, add:

```rust
#[test]
fn test_tool_result_new_has_no_metadata() {
    let result = ToolResult::new("hello");
    assert!(result.metadata.is_none());
    assert!(!result.is_error);
}

#[test]
fn test_tool_result_with_metadata() {
    use std::collections::HashMap;
    let mut meta = HashMap::new();
    meta.insert("key".to_string(), serde_json::json!("value"));
    let result = ToolResult::new("hello").with_metadata(meta);
    assert!(result.metadata.is_some());
    assert_eq!(result.metadata.as_ref().unwrap()["key"], "value");
}

#[test]
fn test_tool_result_error_has_no_metadata() {
    let result = ToolResult::error("fail");
    assert!(result.metadata.is_none());
    assert!(result.is_error);
}

#[test]
fn test_tool_result_from_result_has_no_metadata() {
    let result = ToolResult::from_result(Ok("ok".to_string()), "test");
    assert!(result.metadata.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib test_tool_result_new_has_no_metadata test_tool_result_with_metadata test_tool_result_error_has_no_metadata test_tool_result_from_result_has_no_metadata`
Expected: FAIL — `metadata` field doesn't exist.

- [ ] **Step 3: Add metadata field and builder method to ToolResult**

In `src/agent/tools/base/mod.rs`, update `ToolResult`:

```rust
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    /// Structured metadata for internal consumption (never sent to LLM).
    /// Tools populate this to communicate structured data to the agent loop
    /// (e.g. suggested buttons, structured result data).
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}
```

Update constructors to add `metadata: None`:
- `ToolResult::new()` — add `metadata: None`
- `ToolResult::error()` — add `metadata: None`
- `ToolResult::from_result()` — both arms get `metadata: None`

Add builder method:

```rust
/// Attach structured metadata to this result. Metadata is collected by the
/// agent loop but never included in LLM context.
pub fn with_metadata(mut self, metadata: HashMap<String, serde_json::Value>) -> Self {
    self.metadata = Some(metadata);
    self
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib test_tool_result_new_has_no_metadata test_tool_result_with_metadata test_tool_result_error_has_no_metadata test_tool_result_from_result_has_no_metadata`
Expected: PASS

- [ ] **Step 5: Fix any compilation errors across the codebase**

Adding a field to `ToolResult` may break pattern matches or struct literals elsewhere. Search for:
- `ToolResult {` struct literals (non-constructor usage)
- Any destructuring of `ToolResult`

Run: `cargo build 2>&1 | head -50`

Fix any errors by adding `metadata: None` to struct literals.

**Verified safe (no code changes needed):**
- `ToolMiddleware::after_execute` receives `&mut ToolResult` — middleware can read/modify metadata but none currently do. `TruncationMiddleware` only modifies `.content` (safe). `CacheMiddleware` clones the full `ToolResult` including metadata (correct — cached results retain their metadata). `LoggingMiddleware` doesn't modify results.
- `CachedResult` stores `result: ToolResult` and calls `.clone()` — `HashMap<String, Value>` implements `Clone`, so this works.

- [ ] **Step 6: Run full test suite**

Run: `cargo test --lib`
Expected: PASS (all existing tests still work)

- [ ] **Step 7: Commit**

```bash
git add src/agent/tools/base/mod.rs src/agent/tools/base/tests.rs
git commit -m "feat(tools): add metadata sideband field to ToolResult"
```

---

### Task 2: Update execute_tool_call to preserve metadata

**Files:**
- Modify: `src/agent/loop/helpers.rs:142-210`

The current function returns `(String, bool)`, discarding the `ToolResult` object. Change it to return the full `ToolResult` so metadata survives.

- [ ] **Step 1: Change return type of execute_tool_call**

In `src/agent/loop/helpers.rs`, change the function signature from:

```rust
pub(super) async fn execute_tool_call(
    // ... params ...
) -> (String, bool) {
```

to:

```rust
pub(super) async fn execute_tool_call(
    // ... params ...
) -> ToolResult {
```

Add import: `use crate::agent::tools::base::ToolResult;`

Update all early-return error paths (exfil guard, unknown tool, approval gate, validation) from:
```rust
return ("Error: ...".to_string(), true);
```
to:
```rust
return ToolResult::error("Error: ...");
```

Update the success path from:
```rust
Ok(result) => (result.content, result.is_error),
```
to:
```rust
Ok(result) => result,
```

Update the error path from:
```rust
Err(e) => {
    let msg = ...;
    (msg, true)
}
```
to:
```rust
Err(e) => {
    let msg = ...;
    ToolResult::error(msg)
}
```

- [ ] **Step 2: Update execute_tools to collect ToolResult**

In `src/agent/loop/iteration.rs`, find `execute_tools()` (around line 380). Change its return type from `Vec<(String, bool)>` to `Vec<ToolResult>`.

For the single-tool fast path, the result is already a `ToolResult`. Wrap in `vec![result]`.

For the parallel path, each spawned task returns a `ToolResult`. Collect them into `Vec<ToolResult>`.

**Explicitly update the panic handler**: In the `Err(join_err)` arm (around line 434), change:
```rust
("Tool crashed unexpectedly".to_string(), true)
```
to:
```rust
ToolResult::error("Tool crashed unexpectedly")
```

- [ ] **Step 3: Update handle_tool_results to accept Vec\<ToolResult\>**

In `src/agent/loop/iteration.rs`, update `handle_tool_results()` signature. Change:

```rust
results: Vec<(String, bool)>,
```

to:

```rust
results: Vec<ToolResult>,
```

Add a new parameter for collecting metadata:

```rust
collected_tool_metadata: &mut Vec<HashMap<String, serde_json::Value>>,
```

Update the loop body from:

```rust
for (tc, (result_str, is_error)) in tool_calls.iter().zip(results) {
    if !is_error {
        collected_media.extend(extract_media_paths(&result_str));
    }
    ContextBuilder::add_tool_result(messages, &tc.id, &tc.name, &result_str, is_error);
}
```

to:

```rust
for (tc, result) in tool_calls.iter().zip(results) {
    if !result.is_error {
        collected_media.extend(extract_media_paths(&result.content));
    }
    // Collect metadata sideband (stripped from LLM context)
    if let Some(meta) = result.metadata {
        collected_tool_metadata.push(meta);
    }
    ContextBuilder::add_tool_result(messages, &tc.id, &tc.name, &result.content, result.is_error);
}
```

The padding logic for mismatched lengths should use `ToolResult::error(...)` instead of tuple construction.

- [ ] **Step 4: Wire collected_tool_metadata in the agent loop**

In `run_agent_loop_with_overrides()` (around line 27 in iteration.rs):

Declare the accumulator alongside `collected_media`:
```rust
let mut collected_tool_metadata: Vec<HashMap<String, serde_json::Value>> = Vec::new();
```

Pass `&mut collected_tool_metadata` to every `handle_tool_results()` call.

- [ ] **Step 5: Build and run tests**

Run: `cargo build && cargo test --lib`
Expected: PASS — all existing behavior preserved, metadata just flows through unused.

- [ ] **Step 6: Commit**

```bash
git add src/agent/loop/helpers.rs src/agent/loop/iteration.rs
git commit -m "refactor(loop): propagate ToolResult through execution pipeline

Replaces (String, bool) tuples with full ToolResult, preserving the
metadata sideband through execute_tool_call → execute_tools →
handle_tool_results. Metadata is collected but not yet consumed."
```

---

## Chunk 2: Auto-Buttons Consumer

### Task 3: Merge tool-suggested buttons into response_metadata

**Files:**
- Modify: `src/agent/loop/iteration.rs` (post-loop logic, around lines 310, 342)
- Test: `src/agent/loop/iteration.rs` or a new test module

The merge logic runs after the agent loop completes, at the exit points where `take_pending_buttons_metadata()` is called. Tool-suggested buttons are unconditional (always present). LLM-added buttons are additive. Combined total capped at 5. Tool-suggested take priority on ID conflict.

**Exit points:** There are three `AgentLoopResult` constructions: line ~310 (`TextAction::Return` — early exit on text response), line ~357 (post-loop summary), and line ~368 (final fallback). Lines 357 and 368 share the `response_metadata` built at line ~342. So only **two** `merge_suggested_buttons` calls are needed: one at line ~310 and one at line ~342.

**Multi-iteration dedup:** If the same tool is called across multiple iterations (e.g., `list_tasks` → `complete_task` → `list_tasks`), `collected_tool_metadata` accumulates buttons from all iterations. The merge function deduplicates by ID (last-seen wins within suggested, then LLM-added are appended).

- [ ] **Step 1: Write unit tests for merge logic (TDD: tests first)**

Test cases:
1. No suggested buttons, no LLM buttons → empty metadata
2. Suggested buttons only → all present (up to 5)
3. LLM buttons only → all present (unchanged behavior)
4. Both, no ID conflict → merged, tool-suggested first, capped at 5
5. Both, ID conflict → tool-suggested wins
6. More than 5 total → truncated to 5
7. Duplicate IDs across iterations → deduplicated (last wins)

```rust
#[cfg(test)]
mod merge_tests {
    use super::*;

    #[test]
    fn test_no_buttons() {
        let mut meta = HashMap::new();
        let tool_meta: Vec<HashMap<String, serde_json::Value>> = vec![];
        merge_suggested_buttons(&mut meta, &tool_meta);
        assert!(!meta.contains_key(crate::bus::meta::BUTTONS));
    }

    #[test]
    fn test_suggested_only() {
        let mut meta = HashMap::new();
        let tool_meta = vec![HashMap::from([(
            "suggested_buttons".to_string(),
            serde_json::json!([
                {"id": "complete-1", "label": "Complete Task", "style": "primary",
                 "context": "{\"task_id\":\"1\"}"}
            ]),
        )])];
        merge_suggested_buttons(&mut meta, &tool_meta);
        let buttons = meta[crate::bus::meta::BUTTONS].as_array().unwrap();
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0]["id"], "complete-1");
    }

    #[test]
    fn test_llm_only_unchanged() {
        let mut meta = HashMap::from([(
            crate::bus::meta::BUTTONS.to_string(),
            serde_json::json!([{"id": "custom", "label": "Custom", "style": "secondary"}]),
        )]);
        let tool_meta: Vec<HashMap<String, serde_json::Value>> = vec![];
        merge_suggested_buttons(&mut meta, &tool_meta);
        let buttons = meta[crate::bus::meta::BUTTONS].as_array().unwrap();
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0]["id"], "custom");
    }

    #[test]
    fn test_merge_no_conflict() {
        let mut meta = HashMap::from([(
            crate::bus::meta::BUTTONS.to_string(),
            serde_json::json!([{"id": "snooze", "label": "Snooze", "style": "secondary"}]),
        )]);
        let tool_meta = vec![HashMap::from([(
            "suggested_buttons".to_string(),
            serde_json::json!([
                {"id": "complete-1", "label": "Complete", "style": "primary"}
            ]),
        )])];
        merge_suggested_buttons(&mut meta, &tool_meta);
        let buttons = meta[crate::bus::meta::BUTTONS].as_array().unwrap();
        assert_eq!(buttons.len(), 2);
        // Tool-suggested first
        assert_eq!(buttons[0]["id"], "complete-1");
        assert_eq!(buttons[1]["id"], "snooze");
    }

    #[test]
    fn test_id_conflict_tool_wins() {
        let mut meta = HashMap::from([(
            crate::bus::meta::BUTTONS.to_string(),
            serde_json::json!([{"id": "complete-1", "label": "LLM Complete", "style": "danger"}]),
        )]);
        let tool_meta = vec![HashMap::from([(
            "suggested_buttons".to_string(),
            serde_json::json!([
                {"id": "complete-1", "label": "Tool Complete", "style": "primary"}
            ]),
        )])];
        merge_suggested_buttons(&mut meta, &tool_meta);
        let buttons = meta[crate::bus::meta::BUTTONS].as_array().unwrap();
        assert_eq!(buttons.len(), 1);
        // Tool-suggested wins
        assert_eq!(buttons[0]["label"], "Tool Complete");
    }

    #[test]
    fn test_cap_at_five() {
        let tool_meta = vec![HashMap::from([(
            "suggested_buttons".to_string(),
            serde_json::json!([
                {"id": "a", "label": "A", "style": "primary"},
                {"id": "b", "label": "B", "style": "primary"},
                {"id": "c", "label": "C", "style": "primary"},
                {"id": "d", "label": "D", "style": "primary"},
                {"id": "e", "label": "E", "style": "primary"},
            ]),
        )])];
        let mut meta = HashMap::from([(
            crate::bus::meta::BUTTONS.to_string(),
            serde_json::json!([{"id": "f", "label": "F", "style": "secondary"}]),
        )]);
        merge_suggested_buttons(&mut meta, &tool_meta);
        let buttons = meta[crate::bus::meta::BUTTONS].as_array().unwrap();
        assert_eq!(buttons.len(), 5);
        // LLM button "f" should be dropped (over cap)
    }

    #[test]
    fn test_dedup_across_iterations() {
        // Simulates: iteration 1 lists tasks [1,2], iteration 2 lists tasks [2,3]
        let tool_meta = vec![
            HashMap::from([(
                "suggested_buttons".to_string(),
                serde_json::json!([
                    {"id": "complete-1", "label": "Complete: Task 1", "style": "primary"},
                    {"id": "complete-2", "label": "Complete: Task 2 (old)", "style": "primary"},
                ]),
            )]),
            HashMap::from([(
                "suggested_buttons".to_string(),
                serde_json::json!([
                    {"id": "complete-2", "label": "Complete: Task 2 (new)", "style": "primary"},
                    {"id": "complete-3", "label": "Complete: Task 3", "style": "primary"},
                ]),
            )]),
        ];
        let mut meta = HashMap::new();
        merge_suggested_buttons(&mut meta, &tool_meta);
        let buttons = meta[crate::bus::meta::BUTTONS].as_array().unwrap();
        assert_eq!(buttons.len(), 3);
        // complete-2 should use the latest version
        let b2 = buttons.iter().find(|b| b["id"] == "complete-2").unwrap();
        assert_eq!(b2["label"], "Complete: Task 2 (new)");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib merge_tests`
Expected: FAIL — `merge_suggested_buttons` doesn't exist.

- [ ] **Step 3: Write the merge function**

Add a helper function in `src/agent/loop/iteration.rs` (or a small submodule):

```rust
use std::collections::HashSet;

/// Merge tool-suggested buttons with LLM-added buttons.
///
/// Tool-suggested buttons are unconditional (always appear).
/// LLM-added buttons are appended if no ID conflict.
/// Deduplicates by ID (last occurrence wins for multi-iteration accumulation).
/// Total capped at 5 (Slack/Discord limitation).
fn merge_suggested_buttons(
    response_metadata: &mut HashMap<String, serde_json::Value>,
    collected_tool_metadata: &[HashMap<String, serde_json::Value>],
) {
    // Collect all suggested_buttons from tool metadata, dedup by ID (last wins)
    let mut seen_ids = HashSet::new();
    let mut suggested: Vec<serde_json::Value> = Vec::new();
    // Iterate in reverse so we can keep the LAST occurrence of each ID
    let all_buttons: Vec<serde_json::Value> = collected_tool_metadata
        .iter()
        .filter_map(|meta| meta.get("suggested_buttons")?.as_array())
        .flatten()
        .cloned()
        .collect();
    for b in all_buttons.into_iter().rev() {
        if let Some(id) = b["id"].as_str() {
            if seen_ids.insert(id.to_string()) {
                suggested.push(b);
            }
        }
    }
    suggested.reverse(); // Restore original order
    if suggested.is_empty() {
        return;
    }

    // Get any LLM-added buttons (from add_buttons tool)
    let llm_buttons = response_metadata
        .get(crate::bus::meta::BUTTONS)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Merge: tool-suggested first, then LLM-added (no ID conflicts)
    let mut final_buttons = suggested;
    for b in llm_buttons {
        if let Some(id) = b["id"].as_str() {
            if !seen_ids.contains(id) {
                final_buttons.push(b);
            }
        }
    }
    final_buttons.truncate(5);

    response_metadata.insert(
        crate::bus::meta::BUTTONS.to_string(),
        serde_json::Value::Array(final_buttons),
    );
}
```

- [ ] **Step 4: Call merge at both exit points**

At both places where `take_pending_buttons_metadata()` is called (line ~310 and line ~342), add:

```rust
let mut response_metadata = self.take_pending_buttons_metadata();
merge_suggested_buttons(&mut response_metadata, &collected_tool_metadata);
```

Line ~357 and ~368 already use the `response_metadata` from line ~342, so no additional merge calls needed.

- [ ] **Step 5: Run tests**

Run: `cargo test --lib merge_tests`
Expected: PASS

- [ ] **Step 6: Run full test suite**

Run: `cargo test --lib`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/agent/loop/iteration.rs
git commit -m "feat(loop): auto-merge tool-suggested buttons into response metadata

Tool-suggested buttons are unconditional and take priority over
LLM-added buttons. Merged at both loop exit points. Capped at 5."
```

---

## Chunk 3: Todoist & Google Tasks Auto-Buttons

### Task 4: Add suggested_buttons to Todoist tool

**Files:**
- Modify: `src/agent/tools/todoist/mod.rs`
- Test: `src/agent/tools/todoist/tests.rs`

The `list_tasks` and `get_task` actions should return `suggested_buttons` in their `ToolResult.metadata` for incomplete tasks.

**Key design constraint:** The private `list_tasks()` method returns `Result<String>` — it formats task data into text and discards the raw JSON. The conversion to `ToolResult` happens in `execute()` at line 632 via `ToolResult::from_result(result, "Todoist")`. Button logic needs access to raw task data, so we must refactor `list_tasks()` to also return the raw task array.

- [ ] **Step 1: Write failing test for list_tasks suggested buttons**

In `src/agent/tools/todoist/tests.rs`, add a test using the existing `wiremock` pattern (see `test_list_tasks_success` for reference). Note: mock path is `/tasks` (not `/rest/v2/tasks` — base URL already has the version prefix). Constructor: `TodoistTool::with_base_url(String, String)`.

```rust
#[tokio::test]
async fn test_list_tasks_returns_suggested_buttons() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tasks"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [
                {
                    "id": "123",
                    "content": "Buy groceries",
                    "is_completed": false,
                    "priority": 1,
                    "due": {"string": "today"},
                    "labels": []
                },
                {
                    "id": "456",
                    "content": "Already done",
                    "is_completed": true,
                    "priority": 1,
                    "due": null,
                    "labels": []
                }
            ],
            "next_cursor": null
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "list_tasks"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    // Should have suggested buttons for incomplete tasks only
    let meta = result.metadata.expect("should have metadata");
    let buttons = meta["suggested_buttons"].as_array().expect("should have buttons");
    assert_eq!(buttons.len(), 1); // only the incomplete task
    assert_eq!(buttons[0]["id"], "complete-123");
    assert!(buttons[0]["label"].as_str().unwrap().contains("Buy groceries"));
    assert_eq!(buttons[0]["style"], "primary");
    let ctx_str = buttons[0]["context"].as_str().unwrap();
    let ctx_val: serde_json::Value = serde_json::from_str(ctx_str).unwrap();
    assert_eq!(ctx_val["task_id"], "123");
    assert_eq!(ctx_val["action"], "complete");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib test_list_tasks_returns_suggested_buttons`
Expected: FAIL — metadata is None.

- [ ] **Step 3: Refactor list_tasks to return raw task data**

Change the private `list_tasks()` method signature from:
```rust
async fn list_tasks(&self, project_id: Option<&str>, filter: Option<&str>) -> Result<String>
```
to:
```rust
async fn list_tasks(&self, project_id: Option<&str>, filter: Option<&str>) -> Result<(String, Vec<serde_json::Value>)>
```

Return both the formatted string AND the raw tasks array: `Ok((formatted_output, tasks))`.

- [ ] **Step 4: Update execute() to build buttons from raw task data**

In the `execute()` method, change the `"list_tasks"` match arm from:
```rust
"list_tasks" => {
    self.list_tasks(params["project_id"].as_str(), params["filter"].as_str())
        .await
}
```
to:
```rust
"list_tasks" => {
    let (text, tasks) = self
        .list_tasks(params["project_id"].as_str(), params["filter"].as_str())
        .await?;
    let buttons = build_task_buttons(&tasks);
    let mut result = ToolResult::new(text);
    if !buttons.is_empty() {
        result = result.with_metadata(std::collections::HashMap::from([(
            "suggested_buttons".to_string(),
            serde_json::Value::Array(buttons),
        )]));
    }
    return Ok(result);
}
```

Add a helper function (private, in the same module):

```rust
/// Build suggested "Complete" buttons for incomplete Todoist tasks (max 5).
fn build_task_buttons(tasks: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut buttons = Vec::new();
    for task in tasks {
        if buttons.len() >= 5 {
            break;
        }
        if task["is_completed"].as_bool().unwrap_or(false) {
            continue;
        }
        let task_id = task["id"].as_str().unwrap_or_default();
        let task_content = task["content"].as_str().unwrap_or("task");
        if task_id.is_empty() {
            continue;
        }
        // UTF-8 safe truncation for button labels
        let label = {
            let truncated: String = task_content.chars().take(25).collect();
            if truncated.len() < task_content.len() {
                format!(
                    "Complete: {}...",
                    task_content.chars().take(22).collect::<String>()
                )
            } else {
                format!("Complete: {task_content}")
            }
        };
        buttons.push(serde_json::json!({
            "id": format!("complete-{task_id}"),
            "label": label,
            "style": "primary",
            "context": serde_json::json!({
                "tool": "todoist",
                "task_id": task_id,
                "action": "complete"
            }).to_string()
        }));
    }
    buttons
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --lib test_list_tasks_returns_suggested_buttons`
Expected: PASS

- [ ] **Step 6: Write and implement get_task suggested buttons**

Similar refactoring for `get_task` — change it to return `Result<(String, serde_json::Value)>` so the raw task JSON is available. In `execute()`, build a single "Complete" button if the task's `is_completed` is false.

Test (using wiremock, matching existing patterns):
```rust
#[tokio::test]
async fn test_get_task_returns_suggested_buttons() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tasks/task123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "task123",
            "content": "Buy milk",
            "is_completed": false,
            "priority": 1,
            "due": {"string": "today"},
            "labels": []
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "get_task", "task_id": "task123"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    let meta = result.metadata.expect("should have metadata");
    let buttons = meta["suggested_buttons"].as_array().unwrap();
    assert_eq!(buttons.len(), 1);
    assert_eq!(buttons[0]["id"], "complete-task123");
}
```

- [ ] **Step 6: Run all todoist tests**

Run: `cargo test --lib todoist`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src/agent/tools/todoist/mod.rs src/agent/tools/todoist/tests.rs
git commit -m "feat(todoist): add auto-suggested buttons for task actions

list_tasks and get_task return suggested_buttons metadata for
incomplete tasks. Buttons bypass LLM — deterministic attachment."
```

---

### Task 5: Add suggested_buttons to Google Tasks tool

**Files:**
- Modify: `src/agent/tools/google_tasks/mod.rs`
- Test: `src/agent/tools/google_tasks/tests.rs`

Same pattern as Todoist. `list_tasks` and `get_task` return `suggested_buttons` for incomplete tasks (status != "completed").

**Test infrastructure note:** `GoogleTasksTool` has no `with_base_url()` constructor and uses `GoogleApiClient` internally. Existing tests are purely unit tests (formatting, validation) — no mock server tests exist. For button tests, use the same unit-test approach: test the `build_task_buttons` helper directly with synthetic JSON, rather than requiring a full mock server setup. Add a `build_google_task_buttons()` helper (analogous to todoist's `build_task_buttons()`) and test it in isolation.

- [ ] **Step 1: Write failing test for button builder**

```rust
#[test]
fn test_build_google_task_buttons_filters_completed() {
    let tasks = vec![
        serde_json::json!({"id": "t1", "title": "Incomplete", "status": "needsAction"}),
        serde_json::json!({"id": "t2", "title": "Done", "status": "completed"}),
        serde_json::json!({"id": "t3", "title": "Also incomplete", "status": "needsAction"}),
    ];
    let buttons = build_google_task_buttons(&tasks, "tasklist1");
    assert_eq!(buttons.len(), 2);
    assert_eq!(buttons[0]["id"], "complete-t1");
    assert_eq!(buttons[1]["id"], "complete-t3");
    // Verify context includes tasklist_id
    let ctx: serde_json::Value =
        serde_json::from_str(buttons[0]["context"].as_str().unwrap()).unwrap();
    assert_eq!(ctx["tasklist_id"], "tasklist1");
}
```

- [ ] **Step 2: Implement build_google_task_buttons and wire into execute()**

Same structure as Todoist. Google Tasks uses `status: "needsAction"` for incomplete (not `is_completed: bool`). The `list_tasks` action receives `tasklist_id` as a parameter — include it in the button context.

Context payload:
```json
{"tool": "google_tasks", "task_id": "abc", "tasklist_id": "xyz", "action": "complete"}
```

Refactor `list_tasks()` to return raw task data alongside formatted string (same pattern as todoist). Button ID format: `complete-{task_id}`.

- [ ] **Step 3: Run all google_tasks tests**

Run: `cargo test --lib google_tasks`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/agent/tools/google_tasks/mod.rs src/agent/tools/google_tasks/tests.rs
git commit -m "feat(google_tasks): add auto-suggested buttons for task actions

list_tasks and get_task return suggested_buttons metadata for
incomplete tasks, matching todoist pattern."
```

---

## Chunk 4: Button Click Feedback

### Task 6: Add thinking emoji to Slack button clicks

**Files:**
- Modify: `src/channels/slack/mod.rs` (lines 721-739, 1185-1274)

Currently `handle_interactive_payload()` doesn't add the thinking emoji when a button is clicked, unlike `handle_slack_event()` which does. The button's `message_ts` is already extracted at line 1212.

- [ ] **Step 1: Add thinking_emoji parameter to handle_interactive_payload**

Change the function signature from:

```rust
async fn handle_interactive_payload(
    payload: &Value,
    inbound_tx: &Arc<mpsc::Sender<InboundMessage>>,
    allow_from: &[String],
    allow_groups: &[String],
    dm_policy: &crate::config::DmPolicy,
    bot_token: &str,
    client: &reqwest::Client,
) -> Result<()> {
```

to:

```rust
async fn handle_interactive_payload(
    payload: &Value,
    inbound_tx: &Arc<mpsc::Sender<InboundMessage>>,
    allow_from: &[String],
    allow_groups: &[String],
    dm_policy: &crate::config::DmPolicy,
    bot_token: &str,
    client: &reqwest::Client,
    thinking_emoji: &str,
) -> Result<()> {
```

- [ ] **Step 2: Add reactions.add call after inbound message is sent**

After `inbound_tx.send(inbound_msg).await` (line 1268), add the thinking emoji reaction to the **original message** (the one containing the buttons):

```rust
// Add thinking reaction to acknowledge button click (fire-and-forget)
if !message_ts.is_empty() {
    let react_client = client.clone();
    let react_token = bot_token.to_string();
    let react_channel = channel_id.to_string();
    let react_ts = message_ts.to_string();
    let emoji = thinking_emoji.to_string();
    tokio::spawn(async move {
        let _ = react_client
            .post("https://slack.com/api/reactions.add")
            .form(&[
                ("token", react_token.as_str()),
                ("channel", react_channel.as_str()),
                ("timestamp", react_ts.as_str()),
                ("name", emoji.as_str()),
            ])
            .send()
            .await;
    });
}
```

- [ ] **Step 3: Update the call site to pass thinking_emoji**

In the Socket Mode event loop (around line 724), add `&thinking_emoji` to the `handle_interactive_payload()` call. The `thinking_emoji` variable is already in scope (cloned at line 559).

- [ ] **Step 4: Build and test**

Run: `cargo build && cargo test --lib slack`
Expected: PASS — compilation succeeds, existing tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/channels/slack/mod.rs
git commit -m "fix(slack): add thinking emoji on button click

handle_interactive_payload now adds the thinking reaction to the
message when a button is clicked, matching the regular message
flow. Provides visual feedback while the agent processes."
```

---

## Chunk 5: Documentation & Cleanup

### Task 7: Update CLAUDE.md and system prompt

**Files:**
- Modify: `CLAUDE.md`
- Modify: `src/agent/context/mod.rs` (system prompt, around line 236)

- [ ] **Step 1: Update CLAUDE.md**

Add a new bullet to Common Pitfalls:

```
- **Tool result metadata sideband**: `ToolResult.metadata: Option<HashMap<String, Value>>` carries structured data (e.g. `suggested_buttons`) that is collected by the agent loop but never sent to the LLM. Tools populate it in `execute()`. After the loop, `merge_suggested_buttons()` combines tool-suggested buttons with LLM-added buttons (tool-suggested are unconditional, take priority on ID conflict, total capped at 5). Currently used by: todoist (`list_tasks`, `get_task`), google_tasks (`list_tasks`, `get_task`).
```

- [ ] **Step 2: Update system prompt Interactive Buttons section**

In `src/agent/context/mod.rs`, update the Interactive Buttons guidance to note that task tools auto-attach buttons:

Add after the existing button guidance:
```
Note: Task tools (todoist, google_tasks) automatically attach Complete buttons — you don't need to call add_buttons for basic task completion. Use add_buttons only for additional buttons (e.g., Snooze, Edit) beyond what the tool provides automatically.
```

- [ ] **Step 3: Run docs build**

Run: `python3 docs/build.py`
(Only if any docs/_pages/ files were changed — in this case they weren't.)

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md src/agent/context/mod.rs
git commit -m "docs: document tool metadata sideband and auto-buttons"
```

---

## Future Work (Not In This Plan)

These are noted for follow-up plans:

1. **Auto-buttons for other tools**: google_calendar (RSVP), google_mail (Reply/Archive), github (Approve/Close/Merge), cron (Pause/Remove), media (Add to library), obsidian (Edit/Append)
2. **Discord button context propagation**: Discord doesn't forward `ButtonSpec.context` on click (only `custom_id`). Needs an LRU cache mapping `custom_id → context` in the Discord channel handler.
3. **Discord thinking feedback**: Consider adding reaction-based feedback similar to Slack's emoji lifecycle.
4. **Deterministic button click handling**: Instead of relying on the LLM to interpret `[button:{id}]\nButton context: {...}`, directly dispatch the action based on the structured context. Would eliminate LLM from the button-click-to-action path entirely.
