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
