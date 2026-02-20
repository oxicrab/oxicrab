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
    assert!(result.content.contains("unknown action"));
}

#[tokio::test]
async fn test_list_issues_missing_owner() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "list_issues", "repo": "oxicrab"}),
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

#[tokio::test]
async fn test_get_issue_missing_number() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "get_issue", "owner": "a", "repo": "b"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("number"));
}

#[tokio::test]
async fn test_get_pr_files_missing_number() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "get_pr_files", "owner": "a", "repo": "b"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("number"));
}

#[tokio::test]
async fn test_create_pr_review_missing_number() {
    let result = tool()
        .execute(
            serde_json::json!({
                "action": "create_pr_review",
                "owner": "a",
                "repo": "b",
                "event": "APPROVE"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("number"));
}

#[tokio::test]
async fn test_create_pr_review_missing_event() {
    let result = tool()
        .execute(
            serde_json::json!({
                "action": "create_pr_review",
                "owner": "a",
                "repo": "b",
                "number": 1
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("event"));
}

#[tokio::test]
async fn test_create_pr_review_invalid_event() {
    let result = tool()
        .execute(
            serde_json::json!({
                "action": "create_pr_review",
                "owner": "a",
                "repo": "b",
                "number": 1,
                "event": "REJECT"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid event"));
}

#[tokio::test]
async fn test_get_file_content_missing_path() {
    let result = tool()
        .execute(
            serde_json::json!({
                "action": "get_file_content",
                "owner": "a",
                "repo": "b"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("path"));
}

#[tokio::test]
async fn test_trigger_workflow_missing_workflow_id() {
    let result = tool()
        .execute(
            serde_json::json!({
                "action": "trigger_workflow",
                "owner": "a",
                "repo": "b"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("workflow_id"));
}

#[tokio::test]
async fn test_get_workflow_runs_missing_owner() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "get_workflow_runs", "repo": "b"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("owner"));
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
async fn test_list_issues_pagination() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/issues"))
        .and(query_param("page", "2"))
        .and(query_param("per_page", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 6,
                "title": "Page 2 issue",
                "user": {"login": "alice"},
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
                "repo": "repo",
                "page": 2,
                "per_page": 5
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("page 2"));
    assert!(result.content.contains("#6"));
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
    assert!(
        result
            .content
            .contains("https://github.com/octo/repo/issues/99")
    );
}

#[tokio::test]
async fn test_create_issue_with_labels() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/octo/repo/issues"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 100,
            "html_url": "https://github.com/octo/repo/issues/100"
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
                "title": "Bug with labels",
                "labels": ["bug", "high-priority"]
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("#100"));
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
async fn test_list_prs_pagination() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/pulls"))
        .and(query_param("page", "3"))
        .and(query_param("per_page", "25"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 50,
                "title": "Page 3 PR",
                "user": {"login": "carol"},
                "draft": false,
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
                "repo": "repo",
                "page": 3,
                "per_page": 25
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("page 3"));
    assert!(result.content.contains("#50"));
}

#[tokio::test]
async fn test_get_issue_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/issues/42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "number": 42,
            "title": "Widget is broken",
            "state": "open",
            "user": {"login": "alice"},
            "body": "The widget does not work correctly when...",
            "comments": 5,
            "html_url": "https://github.com/octo/repo/issues/42",
            "labels": [{"name": "bug"}, {"name": "ui"}],
            "assignees": [{"login": "bob"}, {"login": "carol"}]
        })))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "get_issue",
                "owner": "octo",
                "repo": "repo",
                "number": 42
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("#42"));
    assert!(result.content.contains("Widget is broken"));
    assert!(result.content.contains("open"));
    assert!(result.content.contains("alice"));
    assert!(result.content.contains("5 comments"));
    assert!(result.content.contains("bug"));
    assert!(result.content.contains("bob"));
    assert!(result.content.contains("carol"));
}

#[tokio::test]
async fn test_get_pr_files_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/pulls/10/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "filename": "src/main.rs",
                "status": "modified",
                "additions": 10,
                "deletions": 3,
                "patch": "@@ -1,5 +1,12 @@\n+use new_crate;"
            },
            {
                "filename": "README.md",
                "status": "modified",
                "additions": 2,
                "deletions": 0,
                "patch": "@@ -1 +1,3 @@\n+New section"
            }
        ])))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "get_pr_files",
                "owner": "octo",
                "repo": "repo",
                "number": 10
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("src/main.rs"));
    assert!(result.content.contains("modified"));
    assert!(result.content.contains("+10"));
    assert!(result.content.contains("README.md"));
    assert!(result.content.contains("2 files"));
}

#[tokio::test]
async fn test_create_pr_review_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/octo/repo/pulls/10/reviews"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 777,
            "state": "APPROVED"
        })))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "create_pr_review",
                "owner": "octo",
                "repo": "repo",
                "number": 10,
                "event": "APPROVE",
                "body": "Looks good!"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("#777"));
    assert!(result.content.contains("PR #10"));
    assert!(result.content.contains("APPROVED"));
}

#[tokio::test]
async fn test_get_file_content_success() {
    let server = MockServer::start().await;
    let content_b64 = base64::engine::general_purpose::STANDARD
        .encode("fn main() {\n    println!(\"hello\");\n}\n");
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/contents/src/main.rs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "main.rs",
            "encoding": "base64",
            "content": content_b64,
            "size": 42,
            "type": "file"
        })))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "get_file_content",
                "owner": "octo",
                "repo": "repo",
                "path": "src/main.rs"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("main.rs"));
    assert!(result.content.contains("fn main()"));
    assert!(result.content.contains("hello"));
}

#[tokio::test]
async fn test_get_file_content_with_ref() {
    let server = MockServer::start().await;
    let content_b64 = base64::engine::general_purpose::STANDARD.encode("old content");
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/contents/README.md"))
        .and(query_param("ref", "v1.0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "README.md",
            "encoding": "base64",
            "content": content_b64,
            "size": 11,
            "type": "file"
        })))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "get_file_content",
                "owner": "octo",
                "repo": "repo",
                "path": "README.md",
                "ref": "v1.0"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("old content"));
}

#[tokio::test]
async fn test_get_file_content_directory() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/contents/src"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"name": "main.rs", "type": "file", "size": 100},
            {"name": "lib.rs", "type": "file", "size": 200},
            {"name": "utils", "type": "dir", "size": 0}
        ])))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "get_file_content",
                "owner": "octo",
                "repo": "repo",
                "path": "src"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Directory"));
    assert!(result.content.contains("main.rs"));
    assert!(result.content.contains("lib.rs"));
    assert!(result.content.contains("utils"));
}

#[tokio::test]
async fn test_trigger_workflow_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/octo/repo/actions/workflows/ci.yml/dispatches"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "trigger_workflow",
                "owner": "octo",
                "repo": "repo",
                "workflow_id": "ci.yml",
                "ref": "main"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Triggered workflow ci.yml"));
    assert!(result.content.contains("octo/repo"));
}

#[tokio::test]
async fn test_trigger_workflow_with_inputs() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/repos/octo/repo/actions/workflows/deploy.yml/dispatches",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "trigger_workflow",
                "owner": "octo",
                "repo": "repo",
                "workflow_id": "deploy.yml",
                "ref": "release/v2",
                "inputs": {"environment": "staging"}
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Triggered workflow deploy.yml"));
}

#[tokio::test]
async fn test_get_workflow_runs_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/actions/runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 2,
            "workflow_runs": [
                {
                    "id": 1001,
                    "name": "CI",
                    "status": "completed",
                    "conclusion": "success",
                    "head_branch": "main",
                    "created_at": "2025-01-15T10:00:00Z",
                    "html_url": "https://github.com/octo/repo/actions/runs/1001"
                },
                {
                    "id": 1002,
                    "name": "CI",
                    "status": "in_progress",
                    "conclusion": null,
                    "head_branch": "feature/x",
                    "created_at": "2025-01-15T11:00:00Z",
                    "html_url": "https://github.com/octo/repo/actions/runs/1002"
                }
            ]
        })))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "get_workflow_runs",
                "owner": "octo",
                "repo": "repo"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("#1001"));
    assert!(result.content.contains("CI"));
    assert!(result.content.contains("completed"));
    assert!(result.content.contains("success"));
    assert!(result.content.contains("main"));
    assert!(result.content.contains("#1002"));
    assert!(result.content.contains("in_progress"));
}

#[tokio::test]
async fn test_get_workflow_runs_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/actions/runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 0,
            "workflow_runs": []
        })))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "get_workflow_runs",
                "owner": "octo",
                "repo": "repo"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("No workflow runs found"));
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
