use super::*;
use oxicrab_core::tools::base::ExecutionContext;
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
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("action"));
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
    // Error message is sanitized to prevent token leakage — the raw "Bad credentials"
    // message from GitHub is replaced with a safe alternative
    assert!(result.content.contains("authentication error"));
}

#[test]
fn test_github_capabilities() {
    use oxicrab_core::tools::base::SubagentAccess;
    let tool = GitHubTool::new("fake".to_string());
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
    assert!(read_only.contains(&"list_prs"));
    assert!(read_only.contains(&"list_issues"));
    assert!(read_only.contains(&"get_issue"));
    assert!(read_only.contains(&"get_pr"));
    assert!(read_only.contains(&"get_pr_files"));
    assert!(read_only.contains(&"get_file_content"));
    assert!(read_only.contains(&"get_workflow_runs"));
    assert!(read_only.contains(&"notifications"));
    assert!(mutating.contains(&"create_issue"));
    assert!(mutating.contains(&"create_pr_review"));
    assert!(mutating.contains(&"trigger_workflow"));
}

#[test]
fn test_github_actions_match_schema() {
    let tool = GitHubTool::new("fake".to_string());
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

// --- Suggested buttons tests ---

#[test]
fn test_build_issue_buttons_open_issues() {
    let issues = vec![
        serde_json::json!({
            "number": 42,
            "title": "Fix the widget",
            "state": "open"
        }),
        serde_json::json!({
            "number": 43,
            "title": "Closed bug",
            "state": "closed"
        }),
        serde_json::json!({
            "number": 44,
            "title": "Another open issue",
            "state": "open"
        }),
    ];
    let buttons = build_issue_buttons(&issues, "octo", "repo");
    assert_eq!(buttons.len(), 2); // only open issues
    assert_eq!(buttons[0]["id"], "view-issue-42");
    assert_eq!(buttons[0]["style"], "primary");
    assert!(buttons[0]["label"].as_str().unwrap().starts_with("View: "));
    assert_eq!(buttons[1]["id"], "view-issue-44");

    // Verify context uses ActionDispatchPayload format
    let ctx: serde_json::Value =
        serde_json::from_str(buttons[0]["context"].as_str().unwrap()).unwrap();
    assert_eq!(ctx["tool"], "github");
    assert_eq!(ctx["params"]["owner"], "octo");
    assert_eq!(ctx["params"]["repo"], "repo");
    assert_eq!(ctx["params"]["number"], 42);
    assert_eq!(ctx["params"]["action"], "get_issue");
}

#[test]
fn test_build_issue_buttons_max_five() {
    let issues: Vec<serde_json::Value> = (0..10)
        .map(|i| {
            serde_json::json!({
                "number": i + 1,
                "title": format!("Issue {i}"),
                "state": "open"
            })
        })
        .collect();
    let buttons = build_issue_buttons(&issues, "octo", "repo");
    assert_eq!(buttons.len(), 5);
}

#[test]
fn test_build_issue_buttons_no_open() {
    let issues = vec![serde_json::json!({
        "number": 1,
        "title": "Closed",
        "state": "closed"
    })];
    let buttons = build_issue_buttons(&issues, "octo", "repo");
    assert!(buttons.is_empty());
}

#[test]
fn test_build_pr_list_buttons_open_prs() {
    let prs = vec![
        serde_json::json!({
            "number": 10,
            "title": "Add tests",
            "state": "open"
        }),
        serde_json::json!({
            "number": 11,
            "title": "Merged PR",
            "state": "closed"
        }),
    ];
    let buttons = build_pr_list_buttons(&prs, "octo", "repo");
    assert_eq!(buttons.len(), 1);
    assert_eq!(buttons[0]["id"], "approve-pr-10");
    assert_eq!(buttons[0]["style"], "primary");

    let ctx: serde_json::Value =
        serde_json::from_str(buttons[0]["context"].as_str().unwrap()).unwrap();
    assert_eq!(ctx["tool"], "github");
    assert_eq!(ctx["params"]["owner"], "octo");
    assert_eq!(ctx["params"]["repo"], "repo");
    assert_eq!(ctx["params"]["number"], 10);
    assert_eq!(ctx["params"]["event"], "APPROVE");
    assert_eq!(ctx["params"]["action"], "create_pr_review");
}

#[test]
fn test_build_pr_list_buttons_max_five() {
    let prs: Vec<serde_json::Value> = (0..10)
        .map(|i| {
            serde_json::json!({
                "number": i + 1,
                "title": format!("PR {i}"),
                "state": "open"
            })
        })
        .collect();
    let buttons = build_pr_list_buttons(&prs, "octo", "repo");
    assert_eq!(buttons.len(), 5);
}

#[test]
fn test_build_pr_detail_buttons_open_pr() {
    let pr = serde_json::json!({
        "number": 10,
        "state": "open",
        "merged": false
    });
    let buttons = build_pr_detail_buttons(&pr, "octo", "repo");
    assert_eq!(buttons.len(), 2);
    assert_eq!(buttons[0]["id"], "approve-pr-10");
    assert_eq!(buttons[0]["label"], "Approve");
    assert_eq!(buttons[0]["style"], "primary");
    assert_eq!(buttons[1]["id"], "request-changes-pr-10");
    assert_eq!(buttons[1]["label"], "Request Changes");
    assert_eq!(buttons[1]["style"], "danger");

    let ctx0: serde_json::Value =
        serde_json::from_str(buttons[0]["context"].as_str().unwrap()).unwrap();
    assert_eq!(ctx0["params"]["action"], "create_pr_review");
    assert_eq!(ctx0["params"]["owner"], "octo");
    assert_eq!(ctx0["params"]["repo"], "repo");
    assert_eq!(ctx0["params"]["number"], 10);
    assert_eq!(ctx0["params"]["event"], "APPROVE");
    // "Request Changes" uses a plain string context (routes through LLM for body input)
    assert_eq!(
        buttons[1]["context"].as_str().unwrap(),
        "Request changes on PR #10 in octo/repo"
    );
}

#[test]
fn test_build_pr_detail_buttons_closed_pr() {
    let pr = serde_json::json!({
        "number": 10,
        "state": "closed",
        "merged": false
    });
    let buttons = build_pr_detail_buttons(&pr, "octo", "repo");
    assert!(buttons.is_empty());
}

#[test]
fn test_build_pr_detail_buttons_merged_pr() {
    let pr = serde_json::json!({
        "number": 10,
        "state": "closed",
        "merged": true
    });
    let buttons = build_pr_detail_buttons(&pr, "octo", "repo");
    assert!(buttons.is_empty());
}

#[test]
fn test_truncate_label_short() {
    assert_eq!(truncate_label("Close: ", "Short", 22), "Close: Short");
}

#[test]
fn test_truncate_label_long() {
    let long_title = "This is a very long issue title that exceeds the limit";
    let label = truncate_label("Close: ", long_title, 22);
    assert!(label.ends_with("..."));
    assert!(label.starts_with("Close: "));
    // Verify total text portion is truncated
    assert!(label.len() < long_title.len() + 7);
}

#[test]
fn test_truncate_label_unicode() {
    let title = "Fix emoji handling \u{1F600}\u{1F601}\u{1F602}\u{1F603}\u{1F604}\u{1F605}\u{1F606}\u{1F607}\u{1F608}\u{1F609}";
    let label = truncate_label("Close: ", title, 22);
    // Should not panic on UTF-8 boundary
    assert!(label.starts_with("Close: "));
    assert!(label.ends_with("..."));
}

#[tokio::test]
async fn test_list_issues_returns_suggested_buttons() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/issues"))
        .and(query_param("state", "open"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 42,
                "title": "Fix the widget",
                "state": "open",
                "user": {"login": "alice"},
                "labels": [{"name": "bug"}]
            },
            {
                "number": 43,
                "title": "Closed issue",
                "state": "closed",
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
    let meta = result.metadata.expect("should have metadata");
    let buttons = meta["suggested_buttons"]
        .as_array()
        .expect("should have buttons");
    assert_eq!(buttons.len(), 1); // only open issue
    assert_eq!(buttons[0]["id"], "view-issue-42");
}

#[tokio::test]
async fn test_get_issue_returns_suggested_buttons() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/issues/42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "number": 42,
            "title": "Widget is broken",
            "state": "open",
            "user": {"login": "alice"},
            "body": "It doesn't work",
            "comments": 5,
            "html_url": "https://github.com/octo/repo/issues/42",
            "labels": [],
            "assignees": []
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
    // get_issue returns "Close" button for open issues
    let meta = result.metadata.expect("should have metadata");
    let buttons = meta["suggested_buttons"].as_array().unwrap();
    assert_eq!(buttons.len(), 1);
    assert_eq!(buttons[0]["id"], "close-issue-42");
    assert!(buttons[0]["label"].as_str().unwrap().starts_with("Close: "));
}

#[tokio::test]
async fn test_get_issue_no_buttons_when_closed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/issues/99"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "number": 99,
            "title": "Already fixed",
            "state": "closed",
            "user": {"login": "alice"},
            "body": "Fixed",
            "comments": 0,
            "html_url": "https://github.com/octo/repo/issues/99",
            "labels": [],
            "assignees": []
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
                "number": 99
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.metadata.is_none());
}

#[tokio::test]
async fn test_list_prs_returns_suggested_buttons() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 10,
                "title": "Add tests",
                "state": "open",
                "user": {"login": "alice"},
                "draft": false,
                "mergeable_state": "clean"
            },
            {
                "number": 11,
                "title": "Merged PR",
                "state": "closed",
                "user": {"login": "bob"},
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
                "repo": "repo"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    let meta = result.metadata.expect("should have metadata");
    let buttons = meta["suggested_buttons"]
        .as_array()
        .expect("should have buttons");
    assert_eq!(buttons.len(), 1); // only open PR
    assert_eq!(buttons[0]["id"], "approve-pr-10");
}

#[tokio::test]
async fn test_get_pr_returns_approve_and_request_changes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/pulls/10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "number": 10,
            "title": "Add tests",
            "state": "open",
            "merged": false,
            "user": {"login": "alice"},
            "body": "This PR adds tests",
            "additions": 50,
            "deletions": 10,
            "changed_files": 3,
            "head": {"ref": "feature/tests", "sha": ""},
            "base": {"ref": "main"}
        })))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "get_pr",
                "owner": "octo",
                "repo": "repo",
                "number": 10
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    let meta = result.metadata.expect("should have metadata");
    let buttons = meta["suggested_buttons"].as_array().unwrap();
    assert_eq!(buttons.len(), 2);
    assert_eq!(buttons[0]["id"], "approve-pr-10");
    assert_eq!(buttons[0]["label"], "Approve");
    assert_eq!(buttons[1]["id"], "request-changes-pr-10");
    assert_eq!(buttons[1]["label"], "Request Changes");
}

#[tokio::test]
async fn test_get_pr_no_buttons_when_merged() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/repo/pulls/20"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "number": 20,
            "title": "Old PR",
            "state": "closed",
            "merged": true,
            "user": {"login": "alice"},
            "body": "Merged",
            "additions": 1,
            "deletions": 0,
            "changed_files": 1,
            "head": {"ref": "old-branch", "sha": ""},
            "base": {"ref": "main"}
        })))
        .mount(&server)
        .await;

    let tool = GitHubTool::with_base_url("test_token".to_string(), server.uri());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "get_pr",
                "owner": "octo",
                "repo": "repo",
                "number": 20
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.metadata.is_none());
}

#[tokio::test]
async fn test_list_issues_no_buttons_when_empty() {
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
    assert!(result.metadata.is_none());
}
