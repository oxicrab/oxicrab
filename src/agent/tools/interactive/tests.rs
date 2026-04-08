use super::*;

fn test_ctx(request_id: &str) -> ExecutionContext {
    ExecutionContext {
        metadata: HashMap::from([(
            REQUEST_ID_META_KEY.to_string(),
            Value::String(request_id.to_string()),
        )]),
        ..ExecutionContext::default()
    }
}

#[test]
fn test_add_buttons_stores_specs() {
    let pending = new_pending_buttons();
    let tool = AddButtonsTool::new(pending.clone());
    let params = serde_json::json!({
        "buttons": [
            {"id": "yes", "label": "Yes", "style": "primary", "context": "{\"task_id\": \"123\"}"},
            {"id": "no", "label": "No", "style": "danger"}
        ]
    });
    let ctx = test_ctx("req-1");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(params, &ctx)).unwrap();
    assert!(!result.is_error);

    let specs = pending.take("req-1").unwrap();
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].id, "yes");
    assert_eq!(specs[0].label, "Yes");
    assert_eq!(specs[0].style, "primary");
    assert_eq!(specs[0].context.as_deref(), Some("{\"task_id\": \"123\"}"));
    assert_eq!(specs[1].id, "no");
    assert!(specs[1].context.is_none());
}

#[test]
fn test_pending_buttons_cleared_after_take() {
    let pending = new_pending_buttons();
    pending.store(
        "req-1",
        vec![ButtonSpec {
            id: "x".into(),
            label: "X".into(),
            style: "primary".into(),
            context: None,
        }],
    );
    let taken = pending.take("req-1");
    assert!(taken.is_some());
    assert!(pending.take("req-1").is_none());
}

#[test]
fn test_add_buttons_empty_array_rejected() {
    let pending = new_pending_buttons();
    let tool = AddButtonsTool::new(pending);
    let params = serde_json::json!({"buttons": []});
    let ctx = test_ctx("req-empty");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(params, &ctx)).unwrap();
    assert!(result.is_error);
}

#[test]
fn test_add_buttons_too_many_rejected() {
    let pending = new_pending_buttons();
    let tool = AddButtonsTool::new(pending);
    let params = serde_json::json!({
        "buttons": [
            {"id": "1", "label": "1"},
            {"id": "2", "label": "2"},
            {"id": "3", "label": "3"},
            {"id": "4", "label": "4"},
            {"id": "5", "label": "5"},
            {"id": "6", "label": "6"},
        ]
    });
    let ctx = test_ctx("req-many");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(params, &ctx)).unwrap();
    assert!(result.is_error);
}

#[test]
fn test_add_buttons_invalid_id_rejected() {
    let pending = new_pending_buttons();
    let tool = AddButtonsTool::new(pending);
    let ctx = test_ctx("req-invalid");
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Control characters in ID
    let params = serde_json::json!({"buttons": [{"id": "ok\ninjected", "label": "OK"}]});
    let result = rt.block_on(tool.execute(params, &ctx)).unwrap();
    assert!(result.is_error);

    // Spaces in ID
    let params = serde_json::json!({"buttons": [{"id": "has space", "label": "X"}]});
    let result = rt.block_on(tool.execute(params, &ctx)).unwrap();
    assert!(result.is_error);

    // Valid ID with hyphens/underscores
    let pending2 = new_pending_buttons();
    let tool2 = AddButtonsTool::new(pending2);
    let params = serde_json::json!({"buttons": [{"id": "confirm-yes_1", "label": "OK"}]});
    let result = rt
        .block_on(tool2.execute(params, &test_ctx("req-valid")))
        .unwrap();
    assert!(!result.is_error);
}

#[test]
fn test_add_buttons_empty_id_rejected() {
    let pending = new_pending_buttons();
    let tool = AddButtonsTool::new(pending);
    let params = serde_json::json!({"buttons": [{"id": "", "label": "Empty"}]});
    let ctx = test_ctx("req-empty-id");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(params, &ctx)).unwrap();
    assert!(result.is_error);
}

#[test]
fn test_add_buttons_context_truncated_at_2000() {
    let pending = new_pending_buttons();
    let tool = AddButtonsTool::new(pending.clone());
    let long_context = "x".repeat(3000);
    let params = serde_json::json!({
        "buttons": [{"id": "ok", "label": "OK", "context": long_context}]
    });
    let ctx = test_ctx("req-long-context");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(params, &ctx)).unwrap();
    assert!(!result.is_error);

    let specs = pending.take("req-long-context").unwrap();
    assert_eq!(specs[0].context.as_ref().unwrap().len(), 2000);
}

#[test]
fn test_add_buttons_no_context_is_none() {
    let pending = new_pending_buttons();
    let tool = AddButtonsTool::new(pending.clone());
    let params = serde_json::json!({
        "buttons": [{"id": "ok", "label": "OK"}]
    });
    let ctx = test_ctx("req-no-context");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(params, &ctx)).unwrap();
    assert!(!result.is_error);

    let specs = pending.take("req-no-context").unwrap();
    assert!(specs[0].context.is_none());
}
