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
