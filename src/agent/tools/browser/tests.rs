use super::*;

#[tokio::test]
async fn test_open_ssrf_blocked() {
    let tool = BrowserTool::for_testing();
    let params = serde_json::json!({
        "action": "open",
        "url": "http://169.254.169.254/latest/meta-data"
    });
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("security policy") || result.content.contains("blocked"));
}

#[tokio::test]
async fn test_unknown_action() {
    let tool = BrowserTool::for_testing();
    let params = serde_json::json!({"action": "destroy"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("unknown browser action"));
}

#[tokio::test]
async fn test_missing_action() {
    let tool = BrowserTool::for_testing();
    let params = serde_json::json!({});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("action"));
}

#[tokio::test]
async fn test_open_missing_url() {
    let tool = BrowserTool::for_testing();
    let params = serde_json::json!({"action": "open"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("url"));
}

#[tokio::test]
async fn test_click_missing_selector() {
    let tool = BrowserTool::for_testing();
    let params = serde_json::json!({"action": "click"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("selector"));
}

#[tokio::test]
async fn test_type_missing_params() {
    let tool = BrowserTool::for_testing();

    let params = serde_json::json!({"action": "type"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("selector"));

    let params = serde_json::json!({"action": "type", "selector": "#input"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("text"));
}

#[tokio::test]
async fn test_fill_missing_params() {
    let tool = BrowserTool::for_testing();

    let params = serde_json::json!({"action": "fill"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("selector"));

    let params = serde_json::json!({"action": "fill", "selector": "#input"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("text"));
}

#[tokio::test]
async fn test_eval_missing_javascript() {
    let tool = BrowserTool::for_testing();
    let params = serde_json::json!({"action": "eval"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("javascript"));
}

#[tokio::test]
async fn test_get_missing_what() {
    let tool = BrowserTool::for_testing();
    let params = serde_json::json!({"action": "get"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("what"));
}

#[tokio::test]
async fn test_scroll_missing_direction() {
    let tool = BrowserTool::for_testing();
    let params = serde_json::json!({"action": "scroll"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("direction"));
}

#[tokio::test]
async fn test_navigate_missing_param() {
    let tool = BrowserTool::for_testing();
    let params = serde_json::json!({"action": "navigate"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("navigation"));
}

#[tokio::test]
async fn test_wait_missing_params() {
    let tool = BrowserTool::for_testing();
    let params = serde_json::json!({"action": "wait"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("wait"));
}

#[tokio::test]
async fn test_no_session_errors() {
    let tool = BrowserTool::for_testing();

    // Actions that need an active session should return an error
    let params = serde_json::json!({"action": "click", "selector": "#btn"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("no browser session"));

    let params = serde_json::json!({"action": "screenshot"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("no browser session"));
}

#[tokio::test]
async fn test_close_no_session() {
    let tool = BrowserTool::for_testing();
    let params = serde_json::json!({"action": "close"});
    let result = tool
        .execute(params, &ExecutionContext::default())
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("No browser session"));
}

#[test]
fn test_tool_metadata() {
    let tool = BrowserTool::for_testing();
    assert_eq!(tool.name(), "browser");
    assert!(!tool.description().is_empty());
    let params = tool.parameters();
    assert!(params["properties"]["action"].is_object());
}
