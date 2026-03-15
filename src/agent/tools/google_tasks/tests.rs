use super::*;

fn test_credentials() -> crate::auth::google::GoogleCredentials {
    crate::auth::google::GoogleCredentials {
        token: "fake".to_string(),
        refresh_token: None,
        token_uri: "https://oauth2.googleapis.com/token".to_string(),
        client_id: "fake".to_string(),
        client_secret: "fake".to_string(),
        scopes: vec![],
        expiry: None,
    }
}

#[test]
fn test_google_tasks_capabilities() {
    use crate::agent::tools::base::SubagentAccess;
    let tool = GoogleTasksTool::new(test_credentials());
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
    let mutating: Vec<&str> = caps
        .actions
        .iter()
        .filter(|a| !a.read_only)
        .map(|a| a.name)
        .collect();
    assert!(read_only.contains(&"list_task_lists"));
    assert!(read_only.contains(&"list_tasks"));
    assert!(read_only.contains(&"get_task"));
    assert!(mutating.contains(&"create_task"));
    assert!(mutating.contains(&"update_task"));
    assert!(mutating.contains(&"delete_task"));
}

#[test]
fn test_google_tasks_actions_match_schema() {
    let tool = GoogleTasksTool::new(test_credentials());
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

#[test]
fn test_format_task_detail() {
    let task = serde_json::json!({
        "id": "abc123",
        "title": "Buy groceries",
        "notes": "Milk, eggs, bread",
        "status": "needsAction",
        "due": "2026-03-10T00:00:00.000Z",
        "updated": "2026-03-06T12:00:00.000Z"
    });
    let result = format_task_detail(&task);
    assert!(result.contains("Buy groceries"));
    assert!(result.contains("abc123"));
    assert!(result.contains("Milk, eggs, bread"));
    assert!(result.contains("needsAction"));
    assert!(result.contains("2026-03-10"));
}

#[test]
fn test_format_task_detail_minimal() {
    let task = serde_json::json!({
        "id": "abc123",
        "title": "Simple task"
    });
    let result = format_task_detail(&task);
    assert!(result.contains("Simple task"));
    assert!(result.contains("abc123"));
    assert!(!result.contains("Notes:"));
    assert!(!result.contains("Due:"));
}

#[test]
fn test_format_task_detail_completed() {
    let task = serde_json::json!({
        "id": "abc123",
        "title": "Done task",
        "status": "completed",
        "completed": "2026-03-05T10:00:00.000Z"
    });
    let result = format_task_detail(&task);
    assert!(result.contains("completed"));
    assert!(result.contains("2026-03-05"));
}

#[tokio::test]
async fn test_update_task_empty_body_rejected() {
    let tool = GoogleTasksTool::new(test_credentials());
    let params = serde_json::json!({
        "action": "update_task",
        "task_id": "abc123"
    });
    let ctx = crate::agent::tools::base::ExecutionContext::default();
    let result = tool.execute(params, &ctx).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("requires at least one field"));
}

#[test]
fn test_build_google_task_buttons_filters_completed() {
    let tasks = vec![
        serde_json::json!({"id": "t1", "title": "Incomplete", "status": "needsAction"}),
        serde_json::json!({"id": "t2", "title": "Done", "status": "completed"}),
        serde_json::json!({"id": "t3", "title": "Also incomplete", "status": "needsAction"}),
    ];
    let buttons = build_google_task_buttons(&tasks, "tasklist1");
    assert_eq!(buttons.len(), 2);
    assert_eq!(buttons[0]["id"], "complete-t1");
    assert_eq!(buttons[1]["id"], "complete-t3");
    // Verify context includes tasklist_id and tool name
    let ctx: serde_json::Value =
        serde_json::from_str(buttons[0]["context"].as_str().unwrap()).unwrap();
    assert_eq!(ctx["tasklist_id"], "tasklist1");
    assert_eq!(ctx["tool"], "google_tasks");
    assert_eq!(ctx["task_id"], "t1");
    assert_eq!(ctx["action"], "complete");
}

#[test]
fn test_build_google_task_buttons_max_five() {
    let tasks: Vec<serde_json::Value> = (0..10)
        .map(|i| {
            serde_json::json!({
                "id": format!("t{i}"),
                "title": format!("Task {i}"),
                "status": "needsAction",
            })
        })
        .collect();
    let buttons = build_google_task_buttons(&tasks, "@default");
    assert_eq!(buttons.len(), 5);
}

#[test]
fn test_build_google_task_buttons_empty_list() {
    let buttons = build_google_task_buttons(&[], "@default");
    assert!(buttons.is_empty());
}

#[test]
fn test_build_google_task_buttons_all_completed() {
    let tasks = vec![
        serde_json::json!({"id": "t1", "title": "Done 1", "status": "completed"}),
        serde_json::json!({"id": "t2", "title": "Done 2", "status": "completed"}),
    ];
    let buttons = build_google_task_buttons(&tasks, "@default");
    assert!(buttons.is_empty());
}

#[test]
fn test_build_google_task_buttons_truncates_long_labels() {
    let tasks = vec![serde_json::json!({
        "id": "t1",
        "title": "This is a very long task name that exceeds the limit",
        "status": "needsAction",
    })];
    let buttons = build_google_task_buttons(&tasks, "@default");
    assert_eq!(buttons.len(), 1);
    let label = buttons[0]["label"].as_str().unwrap();
    assert!(label.ends_with("..."));
    assert!(label.len() <= 40); // "Complete: " (10) + 22 chars + "..." (3) = 35 max
}

#[test]
fn test_build_google_task_buttons_skips_empty_id() {
    let tasks = vec![
        serde_json::json!({"id": "", "title": "No ID", "status": "needsAction"}),
        serde_json::json!({"id": "t1", "title": "Has ID", "status": "needsAction"}),
    ];
    let buttons = build_google_task_buttons(&tasks, "@default");
    assert_eq!(buttons.len(), 1);
    assert_eq!(buttons[0]["id"], "complete-t1");
}

#[test]
fn test_with_buttons_empty_returns_no_metadata() {
    let result = ToolResult::new("test".to_string());
    let result = with_buttons(result, vec![]);
    assert!(result.metadata.is_none());
}

#[test]
fn test_with_buttons_non_empty_attaches_metadata() {
    let result = ToolResult::new("test".to_string());
    let buttons = vec![serde_json::json!({"id": "b1", "label": "Click"})];
    let result = with_buttons(result, buttons);
    assert!(result.metadata.is_some());
    let meta = result.metadata.unwrap();
    let suggested = meta.get("suggested_buttons").unwrap();
    assert_eq!(suggested.as_array().unwrap().len(), 1);
}
