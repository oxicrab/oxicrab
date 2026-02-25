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
async fn test_missing_required_params() {
    let tool = BrowserTool::for_testing();
    let cases: Vec<(serde_json::Value, &str)> = vec![
        (serde_json::json!({"action": "open"}), "url"),
        (serde_json::json!({"action": "click"}), "selector"),
        (serde_json::json!({"action": "type"}), "selector"),
        (
            serde_json::json!({"action": "type", "selector": "#input"}),
            "text",
        ),
        (serde_json::json!({"action": "fill"}), "selector"),
        (
            serde_json::json!({"action": "fill", "selector": "#input"}),
            "text",
        ),
        (serde_json::json!({"action": "eval"}), "javascript"),
        (serde_json::json!({"action": "get"}), "what"),
        (serde_json::json!({"action": "scroll"}), "direction"),
        (serde_json::json!({"action": "navigate"}), "navigation"),
        (serde_json::json!({"action": "wait"}), "wait"),
    ];
    for (params, expected) in cases {
        let result = tool
            .execute(params.clone(), &ExecutionContext::default())
            .await
            .unwrap();
        assert!(result.is_error, "expected error for {:?}", params);
        assert!(
            result.content.contains(expected),
            "expected '{}' in error for {:?}, got: {}",
            expected,
            params,
            result.content
        );
    }
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

#[test]
fn test_browser_capabilities() {
    use crate::agent::tools::base::SubagentAccess;
    let tool = BrowserTool::for_testing();
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::ReadOnly);
    let read_only: Vec<&str> = caps
        .actions
        .iter()
        .filter(|a| a.read_only)
        .map(|a| a.name)
        .collect();
    assert!(read_only.contains(&"snapshot"));
    assert!(read_only.contains(&"get"));
}

#[test]
fn test_browser_actions_match_schema() {
    let tool = BrowserTool::for_testing();
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
            "action '{}' in schema but not in capabilities()",
            action
        );
    }
    for action in &cap_actions {
        assert!(
            schema_actions.contains(action),
            "action '{}' in capabilities() but not in schema",
            action
        );
    }
}
