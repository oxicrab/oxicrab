use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const GITHUB_API: &str = "https://api.github.com";

pub struct GitHubTool {
    token: String,
    base_url: String,
    client: Client,
}

impl GitHubTool {
    pub fn new(token: String) -> Self {
        Self {
            token,
            base_url: GITHUB_API.to_string(),
            client: Client::new(),
        }
    }

    #[cfg(test)]
    fn with_base_url(token: String, base_url: String) -> Self {
        Self {
            token,
            base_url,
            client: Client::new(),
        }
    }

    async fn api_get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let resp = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .query(query)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "nanobot")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .timeout(Duration::from_secs(15))
            .send()
            .await?;

        let status = resp.status();
        let body: Value = resp.json().await?;
        if !status.is_success() {
            let msg = body["message"].as_str().unwrap_or("Unknown error");
            anyhow::bail!("GitHub API {}: {}", status, msg);
        }
        Ok(body)
    }

    async fn api_post(&self, path: &str, body: &Value) -> Result<Value> {
        let resp = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .json(body)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "nanobot")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .timeout(Duration::from_secs(15))
            .send()
            .await?;

        let status = resp.status();
        let result: Value = resp.json().await?;
        if !status.is_success() {
            let msg = result["message"].as_str().unwrap_or("Unknown error");
            anyhow::bail!("GitHub API {}: {}", status, msg);
        }
        Ok(result)
    }

    async fn list_issues(&self, owner: &str, repo: &str, state: &str) -> Result<String> {
        let json = self
            .api_get(
                &format!("/repos/{}/{}/issues", owner, repo),
                &[("state", state), ("per_page", "10")],
            )
            .await?;

        let issues = json.as_array().map(|arr| arr.as_slice()).unwrap_or(&[]);
        if issues.is_empty() {
            return Ok(format!("No {} issues in {}/{}.", state, owner, repo));
        }

        let lines: Vec<String> = issues
            .iter()
            .filter(|i| i.get("pull_request").is_none()) // Exclude PRs
            .map(|i| {
                let number = i["number"].as_u64().unwrap_or(0);
                let title = i["title"].as_str().unwrap_or("");
                let user = i["user"]["login"].as_str().unwrap_or("?");
                let labels: Vec<&str> = i["labels"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|l| l["name"].as_str()).collect())
                    .unwrap_or_default();
                let label_str = if labels.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", labels.join(", "))
                };
                format!("#{} {} (by {}){}", number, title, user, label_str)
            })
            .collect();

        Ok(format!(
            "Issues ({}) in {}/{}:\n{}",
            state,
            owner,
            repo,
            lines.join("\n")
        ))
    }

    async fn create_issue(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: Option<&str>,
    ) -> Result<String> {
        let mut payload = serde_json::json!({ "title": title });
        if let Some(b) = body {
            payload["body"] = Value::String(b.to_string());
        }

        let result = self
            .api_post(&format!("/repos/{}/{}/issues", owner, repo), &payload)
            .await?;

        let number = result["number"].as_u64().unwrap_or(0);
        let url = result["html_url"].as_str().unwrap_or("");
        Ok(format!("Created issue #{}: {}", number, url))
    }

    async fn list_prs(&self, owner: &str, repo: &str, state: &str) -> Result<String> {
        let json = self
            .api_get(
                &format!("/repos/{}/{}/pulls", owner, repo),
                &[("state", state), ("per_page", "10")],
            )
            .await?;

        let prs = json.as_array().map(|arr| arr.as_slice()).unwrap_or(&[]);
        if prs.is_empty() {
            return Ok(format!("No {} PRs in {}/{}.", state, owner, repo));
        }

        let lines: Vec<String> = prs
            .iter()
            .map(|pr| {
                let number = pr["number"].as_u64().unwrap_or(0);
                let title = pr["title"].as_str().unwrap_or("");
                let user = pr["user"]["login"].as_str().unwrap_or("?");
                let draft = if pr["draft"].as_bool().unwrap_or(false) {
                    " (draft)"
                } else {
                    ""
                };
                let mergeable_state = pr["mergeable_state"].as_str().unwrap_or("");
                let state_str = if mergeable_state.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", mergeable_state)
                };
                format!("#{} {} (by {}){}{}", number, title, user, draft, state_str)
            })
            .collect();

        Ok(format!(
            "Pull requests ({}) in {}/{}:\n{}",
            state,
            owner,
            repo,
            lines.join("\n")
        ))
    }

    async fn get_pr(&self, owner: &str, repo: &str, number: u64) -> Result<String> {
        let pr = self
            .api_get(&format!("/repos/{}/{}/pulls/{}", owner, repo, number), &[])
            .await?;

        let title = pr["title"].as_str().unwrap_or("");
        let state = pr["state"].as_str().unwrap_or("");
        let user = pr["user"]["login"].as_str().unwrap_or("?");
        let body = pr["body"].as_str().unwrap_or("(no description)");
        let merged = pr["merged"].as_bool().unwrap_or(false);
        let additions = pr["additions"].as_u64().unwrap_or(0);
        let deletions = pr["deletions"].as_u64().unwrap_or(0);
        let changed_files = pr["changed_files"].as_u64().unwrap_or(0);
        let head = pr["head"]["ref"].as_str().unwrap_or("?");
        let base = pr["base"]["ref"].as_str().unwrap_or("?");

        // Fetch checks status
        let sha = pr["head"]["sha"].as_str().unwrap_or("");
        let checks_str = if !sha.is_empty() {
            match self
                .api_get(
                    &format!("/repos/{}/{}/commits/{}/status", owner, repo, sha),
                    &[],
                )
                .await
            {
                Ok(status) => {
                    let state = status["state"].as_str().unwrap_or("unknown");
                    let total = status["total_count"].as_u64().unwrap_or(0);
                    format!("CI: {} ({} checks)", state, total)
                }
                Err(_) => "CI: unknown".to_string(),
            }
        } else {
            "CI: unknown".to_string()
        };

        let status_str = if merged { "merged" } else { state };

        // Truncate body
        let body_preview: String = body.chars().take(500).collect();
        let body_truncated = if body_preview.len() < body.len() {
            format!("{}...", body_preview)
        } else {
            body_preview
        };

        Ok(format!(
            "PR #{} — {} ({})\nBy: {} | {} → {} | {}\n+{} −{} in {} files\n{}\n\n{}",
            number,
            title,
            status_str,
            user,
            head,
            base,
            checks_str,
            additions,
            deletions,
            changed_files,
            checks_str,
            body_truncated
        ))
    }

    async fn list_notifications(&self) -> Result<String> {
        let json = self
            .api_get("/notifications", &[("per_page", "15")])
            .await?;

        let notifs = json.as_array().map(|arr| arr.as_slice()).unwrap_or(&[]);
        if notifs.is_empty() {
            return Ok("No unread notifications.".to_string());
        }

        let lines: Vec<String> = notifs
            .iter()
            .map(|n| {
                let reason = n["reason"].as_str().unwrap_or("?");
                let title = n["subject"]["title"].as_str().unwrap_or("");
                let kind = n["subject"]["type"].as_str().unwrap_or("");
                let repo = n["repository"]["full_name"].as_str().unwrap_or("");
                format!("[{}] {} — {} ({})", reason, title, repo, kind)
            })
            .collect();

        Ok(format!(
            "Unread notifications ({}):\n{}",
            notifs.len(),
            lines.join("\n")
        ))
    }
}

#[async_trait]
impl Tool for GitHubTool {
    fn name(&self) -> &str {
        "github"
    }

    fn description(&self) -> &str {
        "Interact with GitHub. Actions: list_issues, create_issue, list_prs, get_pr, notifications."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_issues", "create_issue", "list_prs", "get_pr", "notifications"],
                    "description": "Action to perform"
                },
                "owner": {
                    "type": "string",
                    "description": "Repository owner (e.g. 'jamtur01')"
                },
                "repo": {
                    "type": "string",
                    "description": "Repository name (e.g. 'nanobot-rust')"
                },
                "state": {
                    "type": "string",
                    "enum": ["open", "closed", "all"],
                    "default": "open",
                    "description": "Filter by state (for list_issues/list_prs)"
                },
                "number": {
                    "type": "integer",
                    "description": "Issue or PR number (for get_pr)"
                },
                "title": {
                    "type": "string",
                    "description": "Issue title (for create_issue)"
                },
                "body": {
                    "type": "string",
                    "description": "Issue body (for create_issue)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        match action {
            "list_issues" | "list_prs" | "create_issue" | "get_pr" => {
                let owner = match params["owner"].as_str() {
                    Some(o) => o,
                    None => return Ok(ToolResult::error("Missing 'owner' parameter".to_string())),
                };
                let repo = match params["repo"].as_str() {
                    Some(r) => r,
                    None => return Ok(ToolResult::error("Missing 'repo' parameter".to_string())),
                };

                let result = match action {
                    "list_issues" => {
                        let state = params["state"].as_str().unwrap_or("open");
                        self.list_issues(owner, repo, state).await
                    }
                    "list_prs" => {
                        let state = params["state"].as_str().unwrap_or("open");
                        self.list_prs(owner, repo, state).await
                    }
                    "create_issue" => {
                        let title = match params["title"].as_str() {
                            Some(t) => t,
                            None => {
                                return Ok(ToolResult::error(
                                    "Missing 'title' parameter".to_string(),
                                ))
                            }
                        };
                        self.create_issue(owner, repo, title, params["body"].as_str())
                            .await
                    }
                    "get_pr" => {
                        let number = match params["number"].as_u64() {
                            Some(n) => n,
                            None => {
                                return Ok(ToolResult::error(
                                    "Missing 'number' parameter".to_string(),
                                ))
                            }
                        };
                        self.get_pr(owner, repo, number).await
                    }
                    _ => unreachable!(),
                };

                match result {
                    Ok(content) => Ok(ToolResult::new(content)),
                    Err(e) => Ok(ToolResult::error(format!("GitHub error: {}", e))),
                }
            }
            "notifications" => match self.list_notifications().await {
                Ok(content) => Ok(ToolResult::new(content)),
                Err(e) => Ok(ToolResult::error(format!("GitHub error: {}", e))),
            },
            _ => Ok(ToolResult::error(format!("Unknown action: {}", action))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn tool() -> GitHubTool {
        GitHubTool::new("fake_token".to_string())
    }

    // --- Validation tests ---

    #[tokio::test]
    async fn test_missing_action() {
        let result = tool().execute(serde_json::json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let result = tool()
            .execute(serde_json::json!({"action": "bogus"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_list_issues_missing_owner() {
        let result = tool()
            .execute(serde_json::json!({"action": "list_issues", "repo": "nanobot"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("owner"));
    }

    #[tokio::test]
    async fn test_list_issues_missing_repo() {
        let result = tool()
            .execute(serde_json::json!({"action": "list_issues", "owner": "alice"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("repo"));
    }

    #[tokio::test]
    async fn test_create_issue_missing_title() {
        let result = tool()
            .execute(serde_json::json!({"action": "create_issue", "owner": "a", "repo": "b"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("title"));
    }

    #[tokio::test]
    async fn test_get_pr_missing_number() {
        let result = tool()
            .execute(serde_json::json!({"action": "get_pr", "owner": "a", "repo": "b"}))
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
            .execute(serde_json::json!({
                "action": "list_issues",
                "owner": "octo",
                "repo": "repo"
            }))
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
            .execute(serde_json::json!({
                "action": "list_issues",
                "owner": "octo",
                "repo": "repo"
            }))
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
            .execute(serde_json::json!({
                "action": "list_issues",
                "owner": "octo",
                "repo": "repo"
            }))
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
            .execute(serde_json::json!({
                "action": "create_issue",
                "owner": "octo",
                "repo": "repo",
                "title": "New bug"
            }))
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
            .execute(serde_json::json!({
                "action": "list_prs",
                "owner": "octo",
                "repo": "repo"
            }))
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
            .execute(serde_json::json!({"action": "notifications"}))
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
            .execute(serde_json::json!({"action": "notifications"}))
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
            .execute(serde_json::json!({
                "action": "list_issues",
                "owner": "octo",
                "repo": "missing"
            }))
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
            .execute(serde_json::json!({
                "action": "list_issues",
                "owner": "octo",
                "repo": "repo"
            }))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("Bad credentials"));
    }
}
