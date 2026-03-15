use super::*;
use std::collections::HashMap;

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

#[test]
fn test_tool_result_new_has_no_metadata() {
    let result = ToolResult::new("hello");
    assert_eq!(result.content, "hello");
    assert!(!result.is_error);
    assert!(result.metadata.is_none());
}

#[test]
fn test_tool_result_with_metadata() {
    let mut meta = HashMap::new();
    meta.insert(
        "buttons".to_string(),
        serde_json::json!([{"id": "btn1", "label": "Click me"}]),
    );
    let result = ToolResult::new("done").with_metadata(meta.clone());
    assert!(result.metadata.is_some());
    let got = result.metadata.unwrap();
    assert_eq!(got["buttons"], meta["buttons"]);
}

#[test]
fn test_tool_result_error_has_no_metadata() {
    let result = ToolResult::error("something failed");
    assert_eq!(result.content, "something failed");
    assert!(result.is_error);
    assert!(result.metadata.is_none());
}

#[test]
fn test_tool_result_from_result_has_no_metadata() {
    let ok_result = ToolResult::from_result(Ok("success".to_string()), "Test");
    assert!(ok_result.metadata.is_none());

    let err_result = ToolResult::from_result(Err(anyhow::anyhow!("boom")), "Test");
    assert!(err_result.metadata.is_none());
}
