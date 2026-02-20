use super::*;
use crate::agent::tools::base::ExecutionContext;
use wiremock::matchers::{body_json, header, method, path, query_param};
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
    assert!(result.content.contains("unknown action"));
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

// --- Validation tests for new actions ---

#[tokio::test]
async fn test_get_task_missing_id() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "get_task"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("task_id"));
}

#[tokio::test]
async fn test_update_task_missing_id() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "update_task"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("task_id"));
}

#[tokio::test]
async fn test_delete_task_missing_id() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "delete_task"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("task_id"));
}

#[tokio::test]
async fn test_add_comment_missing_id() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "add_comment", "comment_content": "hello"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("task_id"));
}

#[tokio::test]
async fn test_add_comment_missing_content() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "add_comment", "task_id": "t1"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("comment_content"));
}

#[tokio::test]
async fn test_list_comments_missing_id() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "list_comments"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("task_id"));
}

// --- Wiremock tests for new actions ---

#[tokio::test]
async fn test_get_task_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tasks/task_abc"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "task_abc",
            "content": "Review PR",
            "description": "Check the new feature branch",
            "priority": 2,
            "due": {"string": "tomorrow"},
            "labels": ["code", "review"],
            "project_id": "proj_1",
            "is_completed": false
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "get_task", "task_id": "task_abc"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("task_abc"));
    assert!(result.content.contains("Review PR"));
    assert!(result.content.contains("Check the new feature branch"));
    assert!(result.content.contains("Priority: 2"));
    assert!(result.content.contains("Due: tomorrow"));
    assert!(result.content.contains("code, review"));
    assert!(result.content.contains("proj_1"));
    assert!(result.content.contains("Status: open"));
}

#[tokio::test]
async fn test_update_task_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tasks/task_abc"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "task_abc",
            "content": "Updated title",
            "priority": 1
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "update_task",
                "task_id": "task_abc",
                "content": "Updated title",
                "priority": 1
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("task_abc"));
    assert!(result.content.contains("updated"));
}

#[tokio::test]
async fn test_delete_task_success() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/tasks/task_abc"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "delete_task", "task_id": "task_abc"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("task_abc"));
    assert!(result.content.contains("deleted"));
}

#[tokio::test]
async fn test_add_comment_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/comments"))
        .and(header("Authorization", "Bearer test_token"))
        .and(body_json(serde_json::json!({
            "task_id": "task_abc",
            "content": "Looks good to me"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "comment_1",
            "task_id": "task_abc",
            "content": "Looks good to me",
            "posted_at": "2026-02-18T10:00:00Z"
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "add_comment",
                "task_id": "task_abc",
                "comment_content": "Looks good to me"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("comment_1"));
    assert!(result.content.contains("task_abc"));
}

#[tokio::test]
async fn test_list_comments_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/comments"))
        .and(query_param("task_id", "task_abc"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [
                {
                    "id": "c1",
                    "content": "First comment",
                    "posted_at": "2026-02-17T09:00:00Z"
                },
                {
                    "id": "c2",
                    "content": "Second comment",
                    "posted_at": "2026-02-18T10:00:00Z"
                }
            ],
            "next_cursor": null
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "list_comments", "task_id": "task_abc"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("First comment"));
    assert!(result.content.contains("Second comment"));
    assert!(result.content.contains("(2)"));
}

#[tokio::test]
async fn test_list_comments_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/comments"))
        .and(query_param("task_id", "task_abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [],
            "next_cursor": null
        })))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "list_comments", "task_id": "task_abc"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("No comments"));
}

// --- Error tests for new actions ---

#[tokio::test]
async fn test_get_task_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tasks/nonexistent"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Task not found"))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "get_task", "task_id": "nonexistent"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("404"));
}

#[tokio::test]
async fn test_delete_task_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/tasks/nonexistent"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Task not found"))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "delete_task", "task_id": "nonexistent"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("404"));
}

#[tokio::test]
async fn test_add_comment_bad_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/comments"))
        .respond_with(ResponseTemplate::new(400).set_body_string("Bad request"))
        .mount(&server)
        .await;

    let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "add_comment",
                "task_id": "bad_task",
                "comment_content": "test"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("400"));
}
