use super::*;
use crate::agent::tools::base::ExecutionContext;

fn create_tool() -> MemorySearchTool {
    let tmp = tempfile::TempDir::new().unwrap();
    let memory = Arc::new(MemoryStore::new(tmp.path()).unwrap());
    MemorySearchTool::new(memory)
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
    assert_eq!(caps.actions.len(), 2);
    assert!(caps.actions.iter().all(|a| a.read_only));
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
