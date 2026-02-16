use super::*;
use crate::agent::tools::base::ExecutionContext;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tool() -> TodoistTool {
    TodoistTool::new("fake_token".to_string())
}

// --- Validation tests ---

#[tokio::test]
async fn test_missing_action() {
    let result = tool()
        .execute(serde_json::json!({}), &ExecutionContext::default())
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_unknown_action() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "bogus"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("Unknown action"));
}

#[tokio::test]
async fn test_create_task_missing_content() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "create_task"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("content"));
}

#[tokio::test]
async fn test_complete_task_missing_id() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "complete_task"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("task_id"));
}

// --- Wiremock tests ---

#[tokio::test]
async fn test_list_tasks_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tasks"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [
                {
                    "id": "abc123",
                    "content": "Buy groceries",
                    "priority": 1,
                    "due": {"string": "today"},
                    "labels": ["shopping"]
                },
                {
                    "id": "def456",
                    "content": "Write tests",
                    "priority": 4,
                    "due": null,
                    "labels": []
                }
            ],
            "next_cursor": null
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "list_tasks"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Buy groceries"));
    assert!(result.content.contains("!!!"));
    assert!(result.content.contains("Write tests"));
    assert!(result.content.contains("[shopping]"));
    assert!(result.content.contains("Tasks (2)"));
}

#[tokio::test]
async fn test_list_tasks_with_filter() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tasks/filter"))
        .and(query_param("query", "today"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [
                {
                    "id": "t1",
                    "content": "Morning standup",
                    "priority": 2,
                    "due": {"string": "today"},
                    "labels": ["work"]
                }
            ],
            "next_cursor": null
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "list_tasks", "filter": "today"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Morning standup"));
    assert!(result.content.contains("!!"));
}

#[tokio::test]
async fn test_list_tasks_with_project_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tasks"))
        .and(query_param("project_id", "proj_abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [
                {"id": "t1", "content": "Project task", "priority": 3, "due": null, "labels": []}
            ],
            "next_cursor": null
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "list_tasks", "project_id": "proj_abc"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Project task"));
}

#[tokio::test]
async fn test_list_tasks_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tasks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [],
            "next_cursor": null
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "list_tasks"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("No tasks found"));
}

#[tokio::test]
async fn test_list_tasks_paginated() {
    let server = MockServer::start().await;
    // Page 1
    Mock::given(method("GET"))
        .and(path("/tasks"))
        .and(wiremock::matchers::query_param_is_missing("cursor"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [
                {"id": "t1", "content": "Task 1", "priority": 4, "due": null, "labels": []}
            ],
            "next_cursor": "page2cursor"
        })))
        .mount(&server)
        .await;
    // Page 2
    Mock::given(method("GET"))
        .and(path("/tasks"))
        .and(query_param("cursor", "page2cursor"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [
                {"id": "t2", "content": "Task 2", "priority": 4, "due": null, "labels": []}
            ],
            "next_cursor": null
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "list_tasks"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Task 1"));
    assert!(result.content.contains("Task 2"));
    assert!(result.content.contains("Tasks (2)"));
}

#[tokio::test]
async fn test_create_task_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tasks"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "new_task_123",
            "content": "Write documentation",
            "priority": 2
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "create_task",
                "content": "Write documentation",
                "priority": 2,
                "due_string": "tomorrow",
                "labels": ["docs"]
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Created task"));
    assert!(result.content.contains("new_task_123"));
    assert!(result.content.contains("Write documentation"));
}

#[tokio::test]
async fn test_complete_task_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tasks/task_xyz/close"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "complete_task", "task_id": "task_xyz"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("task_xyz"));
    assert!(result.content.contains("completed"));
}

#[tokio::test]
async fn test_list_projects_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [
                {"id": "p1", "name": "Inbox", "color": "grey", "is_favorite": false},
                {"id": "p2", "name": "Work", "color": "blue", "is_favorite": true}
            ],
            "next_cursor": null
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "list_projects"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Inbox"));
    assert!(result.content.contains("Work"));
    assert!(result.content.contains(" *")); // favorite marker
    assert!(result.content.contains("Projects (2)"));
}

#[tokio::test]
async fn test_list_projects_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [],
            "next_cursor": null
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "list_projects"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("No projects found"));
}

#[tokio::test]
async fn test_api_error_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tasks"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("bad_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "list_tasks"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("401"));
}

#[tokio::test]
async fn test_api_error_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tasks"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "create_task", "content": "test"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("500"));
}

#[tokio::test]
async fn test_complete_task_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tasks/nonexistent/close"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Task not found"))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "complete_task", "task_id": "nonexistent"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("404"));
}
