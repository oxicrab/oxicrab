use super::*;
use crate::agent::tools::base::ExecutionContext;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tool() -> GitHubTool {
    GitHubTool::new("fake_token".to_string())
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
async fn test_list_issues_missing_owner() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "list_issues", "repo": "nanobot"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("owner"));
}

#[tokio::test]
async fn test_list_issues_missing_repo() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "list_issues", "owner": "alice"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("repo"));
}

#[tokio::test]
async fn test_create_issue_missing_title() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "create_issue", "owner": "a", "repo": "b"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("title"));
}

#[tokio::test]
async fn test_get_pr_missing_number() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "get_pr", "owner": "a", "repo": "b"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("number"));
}

// --- Wiremock tests ---

#[tokio::test]
async fn test_list_issues_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/issues"))
        .and(query_param("state", "open"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 42,
                "title": "Fix the widget",
                "user": {"login": "alice"},
                "labels": [{"name": "bug"}, {"name": "urgent"}]
            },
            {
                "number": 43,
                "title": "Add feature",
                "user": {"login": "bob"},
                "labels": []
            }
        ])))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "list_issues",
                "owner": "octo",
                "repo": "repo"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("#42"));
    assert!(result.content.contains("Fix the widget"));
    assert!(result.content.contains("alice"));
    assert!(result.content.contains("[bug, urgent]"));
    assert!(result.content.contains("#43"));
}

#[tokio::test]
async fn test_list_issues_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "list_issues",
                "owner": "octo",
                "repo": "repo"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("No open issues"));
}

#[tokio::test]
async fn test_list_issues_excludes_prs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 1,
                "title": "A real issue",
                "user": {"login": "alice"},
                "labels": []
            },
            {
                "number": 2,
                "title": "A pull request",
                "user": {"login": "bob"},
                "labels": [],
                "pull_request": {"url": "https://..."}
            }
        ])))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "list_issues",
                "owner": "octo",
                "repo": "repo"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.content.contains("#1"));
    assert!(!result.content.contains("#2")); // PR excluded
}

#[tokio::test]
async fn test_create_issue_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/octo/repo/issues"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 99,
            "html_url": "https://github.com/octo/repo/issues/99"
        })))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "create_issue",
                "owner": "octo",
                "repo": "repo",
                "title": "New bug"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("#99"));
    assert!(result
        .content
        .contains("https://github.com/octo/repo/issues/99"));
}

#[tokio::test]
async fn test_list_prs_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 10,
                "title": "Add tests",
                "user": {"login": "alice"},
                "draft": false,
                "mergeable_state": "clean"
            },
            {
                "number": 11,
                "title": "WIP: refactor",
                "user": {"login": "bob"},
                "draft": true,
                "mergeable_state": ""
            }
        ])))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "list_prs",
                "owner": "octo",
                "repo": "repo"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("#10"));
    assert!(result.content.contains("[clean]"));
    assert!(result.content.contains("#11"));
    assert!(result.content.contains("(draft)"));
}

#[tokio::test]
async fn test_notifications_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/notifications"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "reason": "mention",
                "subject": {"title": "Bug in login", "type": "Issue"},
                "repository": {"full_name": "octo/repo"}
            }
        ])))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "notifications"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("[mention]"));
    assert!(result.content.contains("Bug in login"));
    assert!(result.content.contains("octo/repo"));
}

#[tokio::test]
async fn test_notifications_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/notifications"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({"action": "notifications"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.content.contains("No unread notifications"));
}

#[tokio::test]
async fn test_api_error_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/missing/issues"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "message": "Not Found"
        })))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "list_issues",
                "owner": "octo",
                "repo": "missing"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("Not Found"));
}

#[tokio::test]
async fn test_api_error_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/issues"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "message": "Bad credentials"
        })))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("bad_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "list_issues",
                "owner": "octo",
                "repo": "repo"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("Bad credentials"));
}
