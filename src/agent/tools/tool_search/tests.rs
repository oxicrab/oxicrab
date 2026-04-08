use super::*;

fn make_index() -> Vec<ToolIndexEntry> {
    vec![
        ToolIndexEntry {
            name: "read_file".into(),
            description: "Read a file from disk".into(),
            deferred: false,
        },
        ToolIndexEntry {
            name: "web_scrape".into(),
            description: "Scrape a web page".into(),
            deferred: true,
        },
        ToolIndexEntry {
            name: "git_log".into(),
            description: "Show git commit history".into(),
            deferred: true,
        },
    ]
}

#[tokio::test]
async fn test_search_by_keyword() {
    let activated = ActivatedTools::new();
    let tool = ToolSearchTool::new(make_index(), activated.clone());
    let result = tool
        .execute(
            serde_json::json!({"query": "web"}),
            &ExecutionContext {
                metadata: HashMap::from([(
                    REQUEST_ID_META_KEY.to_string(),
                    Value::String("req-1".to_string()),
                )]),
                ..ExecutionContext::default()
            },
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("web_scrape"));
    assert!(!result.content.contains("read_file"));
    // Deferred tool should be activated
    assert!(activated.snapshot("req-1").await.contains("web_scrape"));
}

#[tokio::test]
async fn test_search_no_results() {
    let activated = ActivatedTools::new();
    let tool = ToolSearchTool::new(make_index(), activated.clone());
    let result = tool
        .execute(
            serde_json::json!({"query": "database"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("No tools found"));
}

#[tokio::test]
async fn test_empty_query_lists_all() {
    let activated = ActivatedTools::new();
    let tool = ToolSearchTool::new(make_index(), activated.clone());
    let result = tool
        .execute(
            serde_json::json!({"query": ""}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.content.contains("Available tools (3)"));
    assert!(result.content.contains("read_file"));
    assert!(result.content.contains("web_scrape"));
}

#[tokio::test]
async fn test_non_deferred_not_activated() {
    let activated = ActivatedTools::new();
    let tool = ToolSearchTool::new(make_index(), activated.clone());
    let _ = tool
        .execute(
            serde_json::json!({"query": "read"}),
            &ExecutionContext {
                metadata: HashMap::from([(
                    REQUEST_ID_META_KEY.to_string(),
                    Value::String("req-2".to_string()),
                )]),
                ..ExecutionContext::default()
            },
        )
        .await
        .unwrap();
    // read_file is not deferred, so it shouldn't be in activated
    assert!(!activated.snapshot("req-2").await.contains("read_file"));
}
