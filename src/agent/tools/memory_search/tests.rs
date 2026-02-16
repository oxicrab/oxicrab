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
