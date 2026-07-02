use super::*;
use crate::agent::tools::base::ExecutionContext;

fn create_tool() -> MemorySearchTool {
    let tmp = tempfile::TempDir::new().unwrap();
    let memory = Arc::new(MemoryStore::new(tmp.path()).unwrap());
    let leak_detector = Arc::new(crate::safety::LeakDetector::new());
    MemorySearchTool::new(memory, leak_detector)
}

/// Like [`create_tool`] but also hands back the underlying store (so a test
/// can seed entries) plus the temp-dir guard. The guard MUST stay bound for
/// the duration of the test: the DB runs in WAL mode, so writes need the
/// on-disk directory to outlive the connection.
fn create_tool_with_store() -> (MemorySearchTool, Arc<MemoryStore>, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let memory = Arc::new(MemoryStore::new(tmp.path()).unwrap());
    let leak_detector = Arc::new(crate::safety::LeakDetector::new());
    let tool = MemorySearchTool::new(memory.clone(), leak_detector);
    (tool, memory, tmp)
}

#[tokio::test]
async fn test_memory_search_missing_query() {
    let tool = create_tool();
    let result = tool
        .execute(serde_json::json!({}), &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("query"));
}

#[tokio::test]
async fn test_memory_search_empty_query() {
    let tool = create_tool();
    let result = tool
        .execute(
            serde_json::json!({"query": "  "}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("query"));
}

#[tokio::test]
async fn test_memory_search_empty_result() {
    let tool = create_tool();
    let result = tool
        .execute(
            serde_json::json!({"query": "nonexistent topic xyz"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    // Should not error - either returns results or a friendly message
    assert!(!result.is_error);
}

#[test]
fn test_tool_metadata() {
    let tool = create_tool();
    assert_eq!(tool.name(), "memory_search");
    assert!(tool.cacheable());
    assert!(tool.description().contains("memory"));
}

#[test]
fn test_memory_search_capabilities() {
    use crate::agent::tools::base::SubagentAccess;
    let tool = create_tool();
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(!caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::ReadOnly);
    assert_eq!(caps.actions.len(), 4);
    // search, explain_last, list_sources are read-only; delete is not
    assert!(caps.actions.iter().filter(|a| a.read_only).count() >= 3);
    assert!(
        caps.actions
            .iter()
            .any(|a| a.name == "delete" && !a.read_only)
    );
}

#[test]
fn test_memory_search_actions_match_schema() {
    let tool = create_tool();
    let caps = tool.capabilities();
    let params = tool.parameters();
    let schema_actions: Vec<String> = params["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let cap_actions: Vec<String> = caps.actions.iter().map(|a| a.name.to_string()).collect();
    for action in &schema_actions {
        assert!(
            cap_actions.contains(action),
            "action '{action}' in schema but not in capabilities()"
        );
    }
    for action in &cap_actions {
        assert!(
            schema_actions.contains(action),
            "action '{action}' in capabilities() but not in schema"
        );
    }
}

#[tokio::test]
async fn test_memory_search_excludes_daily_in_group_chat() {
    let (tool, store, _tmp) = create_tool_with_store();

    // Seed a personal `daily:` note and a shared non-daily entry. Both carry
    // the distinctive token "zorptok" so a single query matches both through
    // either keyword backend (FTS5 MATCH or the LIKE fallback).
    store
        .db()
        .insert_memory("daily:2020-01-01", "zorptok reminder PERSONAL_DAILY_SECRET_XYZ")
        .unwrap();
    store
        .db()
        .insert_memory("knowledge:proj", "zorptok project SHARED_FACT_ABC")
        .unwrap();

    // Group chat: personal daily notes must never surface in a shared context.
    let group_ctx = ExecutionContext {
        metadata: std::collections::HashMap::from([(
            crate::bus::meta::IS_GROUP.to_string(),
            serde_json::Value::Bool(true),
        )]),
        ..Default::default()
    };
    let group_result = tool
        .execute(serde_json::json!({"query": "zorptok"}), &group_ctx)
        .await
        .unwrap();
    assert!(!group_result.is_error);
    // The personal daily secret is filtered out ...
    assert!(
        !group_result.content.contains("PERSONAL_DAILY_SECRET_XYZ"),
        "group-chat search leaked a personal daily note: {}",
        group_result.content
    );
    // ... while the query itself still works: the shared fact is retrieved,
    // proving the daily note was excluded by group scoping, not merely missed
    // by a query that matched nothing.
    assert!(
        group_result.content.contains("SHARED_FACT_ABC"),
        "group-chat search failed to retrieve the shared fact: {}",
        group_result.content
    );

    // Non-group chat: the same seed + query CAN surface the personal daily
    // note, proving the exclusion is conditional on the group flag rather than
    // an unconditional filter (or a query that never matched the daily entry).
    let personal_result = tool
        .execute(
            serde_json::json!({"query": "zorptok"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!personal_result.is_error);
    assert!(
        personal_result.content.contains("PERSONAL_DAILY_SECRET_XYZ"),
        "non-group search should surface personal daily notes: {}",
        personal_result.content
    );
}
