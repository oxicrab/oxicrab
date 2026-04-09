use super::*;
use std::collections::HashMap;

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
fn test_tool_concurrency_default_is_side_effect() {
    assert_eq!(ToolConcurrency::default(), ToolConcurrency::SideEffect);
}

#[test]
fn test_tool_capabilities_default_concurrency() {
    let caps = ToolCapabilities::default();
    assert_eq!(caps.concurrency, ToolConcurrency::SideEffect);
}

#[test]
fn test_tool_concurrency_variants() {
    // Ensure all variants are distinct
    assert_ne!(ToolConcurrency::ReadOnly, ToolConcurrency::SideEffect);
    assert_ne!(ToolConcurrency::SideEffect, ToolConcurrency::Exclusive);
    assert_ne!(ToolConcurrency::ReadOnly, ToolConcurrency::Exclusive);
}
