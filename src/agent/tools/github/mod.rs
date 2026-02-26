use crate::actions;
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use anyhow::Result;
use async_trait::async_trait;
use base64::Engine;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tracing::warn;

const GITHUB_API: &str = "https://api.github.com";

/// Validate a GitHub owner or repo name: alphanumeric, hyphens, dots, underscores only.
fn is_valid_github_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_')
}

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
            client: crate::utils::http::default_http_client(),
        }
    }

    #[cfg(test)]
    fn with_base_url(token: String, base_url: String) -> Self {
        Self {
            token,
            base_url,
            client: crate::utils::http::default_http_client(),
        }
    }

    /// Log a warning when GitHub rate limit is running low.
    fn check_rate_limit(resp: &reqwest::Response) {
        let remaining = resp
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let limit = resp
            .headers()
            .get("x-ratelimit-limit")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        if let (Some(rem), Some(lim)) = (remaining, limit)
            && rem < lim / 10
        {
            warn!("GitHub API rate limit low: {}/{} remaining", rem, lim);
        }
    }

    /// Extract error message from GitHub API response, sanitizing to prevent
    /// token leakage if the API echoes back auth details.
    fn sanitize_api_error(body: &Value) -> String {
        let msg = body["message"].as_str().unwrap_or("unknown error");
        // Don't include the raw message if it might contain auth details
        if msg.to_lowercase().contains("bearer")
            || msg.to_lowercase().contains("token")
            || msg.to_lowercase().contains("credential")
        {
            return "authentication error (check token)".to_string();
        }
        msg.to_string()
    }

    fn github_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "oxicrab")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .timeout(Duration::from_secs(15))
    }

    /// Send a request, check rate limits, and parse the JSON response.
    async fn api_send(&self, req: reqwest::RequestBuilder) -> Result<Value> {
        let resp = req.send().await?;
        let status = resp.status();
        Self::check_rate_limit(&resp);
        if status.as_u16() == 429 {
            anyhow::bail!("GitHub API rate limit exceeded, try again later");
        }
        let body: Value = resp.json().await?;
        if !status.is_success() {
            anyhow::bail!("GitHub API {}: {}", status, Self::sanitize_api_error(&body));
        }
        Ok(body)
    }

    async fn api_get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let req = self.github_headers(
            self.client
                .get(format!("{}{}", self.base_url, path))
                .query(query),
        );
        self.api_send(req).await
    }

    async fn api_post(&self, path: &str, body: &Value) -> Result<Value> {
        let req = self.github_headers(
            self.client
                .post(format!("{}{}", self.base_url, path))
                .json(body),
        );
        self.api_send(req).await
    }

    async fn api_post_no_content(&self, path: &str, body: &Value) -> Result<()> {
        let req = self.github_headers(
            self.client
                .post(format!("{}{}", self.base_url, path))
                .json(body),
        );
        let resp = req.send().await?;
        let status = resp.status();
        Self::check_rate_limit(&resp);
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&text).map_or_else(
                |_| "Unknown error".to_string(),
                |v| Self::sanitize_api_error(&v),
            );
            anyhow::bail!("GitHub API {}: {}", status, msg);
        }
        Ok(())
    }

    async fn list_issues(
        &self,
        owner: &str,
        repo: &str,
        state: &str,
        page: &str,
        per_page: &str,
    ) -> Result<String> {
        let json = self
            .api_get(
                &format!("/repos/{}/{}/issues", owner, repo),
                &[("state", state), ("page", page), ("per_page", per_page)],
            )
            .await?;

        let issues = json.as_array().map(Vec::as_slice).unwrap_or_default();
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
            "Issues ({}) in {}/{} (page {}):\n{}",
            state,
            owner,
            repo,
            page,
            lines.join("\n")
        ))
    }

    async fn create_issue(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: Option<&str>,
        labels: Option<&Vec<&str>>,
    ) -> Result<String> {
        let mut payload = serde_json::json!({ "title": title });
        if let Some(b) = body {
            payload["body"] = Value::String(b.to_string());
        }
        if let Some(l) = labels {
            payload["labels"] = serde_json::json!(l);
        }

        let result = self
            .api_post(&format!("/repos/{}/{}/issues", owner, repo), &payload)
            .await?;

        let number = result["number"].as_u64().unwrap_or(0);
        let url = result["html_url"].as_str().unwrap_or("");
        Ok(format!("Created issue #{}: {}", number, url))
    }

    async fn list_prs(
        &self,
        owner: &str,
        repo: &str,
        state: &str,
        page: &str,
        per_page: &str,
    ) -> Result<String> {
        let json = self
            .api_get(
                &format!("/repos/{}/{}/pulls", owner, repo),
                &[("state", state), ("page", page), ("per_page", per_page)],
            )
            .await?;

        let prs = json.as_array().map(Vec::as_slice).unwrap_or_default();
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
            "Pull requests ({}) in {}/{} (page {}):\n{}",
            state,
            owner,
            repo,
            page,
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
        let checks_str = if sha.is_empty() {
            "CI: unknown".to_string()
        } else {
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
        };

        let status_str = if merged { "merged" } else { state };

        Ok(format!(
            "PR #{} — {} ({})\nBy: {} | {} → {} | {}\n+{} −{} in {} files\n\n{}",
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
            crate::utils::truncate_chars(body, 500, "...")
        ))
    }

    async fn get_issue(&self, owner: &str, repo: &str, number: u64) -> Result<String> {
        let issue = self
            .api_get(&format!("/repos/{}/{}/issues/{}", owner, repo, number), &[])
            .await?;

        let title = issue["title"].as_str().unwrap_or("");
        let state = issue["state"].as_str().unwrap_or("");
        let user = issue["user"]["login"].as_str().unwrap_or("?");
        let body = issue["body"].as_str().unwrap_or("(no description)");
        let comments = issue["comments"].as_u64().unwrap_or(0);
        let url = issue["html_url"].as_str().unwrap_or("");

        let labels: Vec<&str> = issue["labels"]
            .as_array()
            .map(|a| a.iter().filter_map(|l| l["name"].as_str()).collect())
            .unwrap_or_default();
        let label_str = if labels.is_empty() {
            String::new()
        } else {
            format!("\nLabels: {}", labels.join(", "))
        };

        let assignees: Vec<&str> = issue["assignees"]
            .as_array()
            .map(|a| a.iter().filter_map(|u| u["login"].as_str()).collect())
            .unwrap_or_default();
        let assignee_str = if assignees.is_empty() {
            String::new()
        } else {
            format!("\nAssignees: {}", assignees.join(", "))
        };

        Ok(format!(
            "Issue #{} — {} ({})\nBy: {} | {} comments{}{}\n{}\n\n{}",
            number,
            title,
            state,
            user,
            comments,
            label_str,
            assignee_str,
            url,
            crate::utils::truncate_chars(body, 500, "...")
        ))
    }

    async fn get_pr_files(&self, owner: &str, repo: &str, number: u64) -> Result<String> {
        let json = self
            .api_get(
                &format!("/repos/{}/{}/pulls/{}/files", owner, repo, number),
                &[("per_page", "100")],
            )
            .await?;

        let files = json.as_array().map(Vec::as_slice).unwrap_or_default();
        if files.is_empty() {
            return Ok(format!("No files changed in PR #{}", number));
        }

        let lines: Vec<String> = files
            .iter()
            .map(|f| {
                let filename = f["filename"].as_str().unwrap_or("?");
                let status = f["status"].as_str().unwrap_or("?");
                let additions = f["additions"].as_u64().unwrap_or(0);
                let deletions = f["deletions"].as_u64().unwrap_or(0);
                let patch: String = f["patch"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(200)
                    .collect();
                let patch_str = if patch.is_empty() {
                    String::new()
                } else {
                    format!("\n  {}", patch.replace('\n', "\n  "))
                };
                format!(
                    "{} ({}) +{} −{}{}",
                    filename, status, additions, deletions, patch_str
                )
            })
            .collect();

        Ok(format!(
            "Files in PR #{} ({} files):\n{}",
            number,
            files.len(),
            lines.join("\n\n")
        ))
    }

    async fn create_pr_review(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        event: &str,
        body: &str,
    ) -> Result<String> {
        let payload = serde_json::json!({
            "event": event,
            "body": body,
        });

        let result = self
            .api_post(
                &format!("/repos/{}/{}/pulls/{}/reviews", owner, repo, number),
                &payload,
            )
            .await?;

        let review_id = result["id"].as_u64().unwrap_or(0);
        let state = result["state"].as_str().unwrap_or("");
        Ok(format!(
            "Created review #{} on PR #{}: {}",
            review_id, number, state
        ))
    }

    async fn get_file_content(
        &self,
        owner: &str,
        repo: &str,
        file_path: &str,
        git_ref: Option<&str>,
    ) -> Result<String> {
        let mut query: Vec<(&str, &str)> = Vec::new();
        if let Some(r) = git_ref {
            query.push(("ref", r));
        }

        // URL-encode each path segment to handle spaces, ?, # and other special chars
        let encoded_path: String = file_path
            .split('/')
            .map(|seg| urlencoding::encode(seg))
            .collect::<Vec<_>>()
            .join("/");
        let json = self
            .api_get(
                &format!("/repos/{}/{}/contents/{}", owner, repo, encoded_path),
                &query,
            )
            .await?;

        // Handle directory listing (array response)
        if let Some(entries) = json.as_array() {
            let lines: Vec<String> = entries
                .iter()
                .map(|e| {
                    let name = e["name"].as_str().unwrap_or("?");
                    let kind = e["type"].as_str().unwrap_or("?");
                    let size = e["size"].as_u64().unwrap_or(0);
                    format!("{} ({}, {} bytes)", name, kind, size)
                })
                .collect();
            return Ok(format!(
                "Directory {}/{}:\n{}",
                owner,
                file_path,
                lines.join("\n")
            ));
        }

        // Handle file content
        let encoding = json["encoding"].as_str().unwrap_or("");
        let content_b64 = json["content"].as_str().unwrap_or("");
        let name = json["name"].as_str().unwrap_or(file_path);
        let size = json["size"].as_u64().unwrap_or(0);

        if encoding != "base64" {
            return Ok(format!(
                "File {} ({} bytes, encoding: {})",
                name, size, encoding
            ));
        }

        // Decode base64 content (GitHub adds newlines in the base64)
        let cleaned: String = content_b64.chars().filter(|c| !c.is_whitespace()).collect();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&cleaned)
            .map_err(|e| anyhow::anyhow!("Failed to decode base64: {}", e))?;
        let text = String::from_utf8_lossy(&decoded);

        // Truncate at 10k chars
        let truncated: String = text.chars().take(10_000).collect();
        let suffix = if text.chars().count() > 10_000 {
            "\n\n... (truncated)"
        } else {
            ""
        };

        Ok(format!(
            "File: {} ({} bytes)\n\n{}{}",
            name, size, truncated, suffix
        ))
    }

    async fn trigger_workflow(
        &self,
        owner: &str,
        repo: &str,
        workflow_id: &str,
        git_ref: &str,
        inputs: Option<&Value>,
    ) -> Result<String> {
        let mut payload = serde_json::json!({ "ref": git_ref });
        if let Some(inp) = inputs {
            payload["inputs"] = inp.clone();
        }

        self.api_post_no_content(
            &format!(
                "/repos/{}/{}/actions/workflows/{}/dispatches",
                owner, repo, workflow_id
            ),
            &payload,
        )
        .await?;

        Ok(format!(
            "Triggered workflow {} on {}/{} (ref: {})",
            workflow_id, owner, repo, git_ref
        ))
    }

    async fn get_workflow_runs(
        &self,
        owner: &str,
        repo: &str,
        workflow_id: Option<&str>,
    ) -> Result<String> {
        let path = match workflow_id {
            Some(wid) => format!("/repos/{}/{}/actions/workflows/{}/runs", owner, repo, wid),
            None => format!("/repos/{}/{}/actions/runs", owner, repo),
        };

        let json = self.api_get(&path, &[("per_page", "10")]).await?;

        let runs = json["workflow_runs"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();

        if runs.is_empty() {
            return Ok(format!("No workflow runs found in {}/{}.", owner, repo));
        }

        let lines: Vec<String> = runs
            .iter()
            .map(|r| {
                let id = r["id"].as_u64().unwrap_or(0);
                let name = r["name"].as_str().unwrap_or("?");
                let status = r["status"].as_str().unwrap_or("?");
                let conclusion = r["conclusion"].as_str().unwrap_or("pending");
                let branch = r["head_branch"].as_str().unwrap_or("?");
                let created = r["created_at"].as_str().unwrap_or("?");
                let url = r["html_url"].as_str().unwrap_or("");
                format!(
                    "#{} {} — {} ({}) on {} [{}]\n  {}",
                    id, name, status, conclusion, branch, created, url
                )
            })
            .collect();

        Ok(format!(
            "Workflow runs in {}/{} ({} shown):\n{}",
            owner,
            repo,
            runs.len(),
            lines.join("\n")
        ))
    }

    async fn list_notifications(&self) -> Result<String> {
        let json = self
            .api_get("/notifications", &[("per_page", "15")])
            .await?;

        let notifs = json.as_array().map(Vec::as_slice).unwrap_or_default();
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
    fn name(&self) -> &'static str {
        "github"
    }

    fn description(&self) -> &'static str {
        "Interact with GitHub. Actions: list_issues, create_issue, get_issue, list_prs, get_pr, \
         get_pr_files, create_pr_review, get_file_content, trigger_workflow, get_workflow_runs, \
         notifications."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 1, 0)
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: actions![
                list_issues: ro,
                create_issue,
                get_issue: ro,
                list_prs: ro,
                get_pr: ro,
                get_pr_files: ro,
                create_pr_review,
                get_file_content: ro,
                trigger_workflow,
                get_workflow_runs: ro,
                notifications: ro,
            ],
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "list_issues", "create_issue", "get_issue",
                        "list_prs", "get_pr", "get_pr_files", "create_pr_review",
                        "get_file_content", "trigger_workflow", "get_workflow_runs",
                        "notifications"
                    ],
                    "description": "Action to perform"
                },
                "owner": {
                    "type": "string",
                    "description": "Repository owner (e.g. 'jamtur01')"
                },
                "repo": {
                    "type": "string",
                    "description": "Repository name (e.g. 'oxicrab')"
                },
                "state": {
                    "type": "string",
                    "enum": ["open", "closed", "all"],
                    "default": "open",
                    "description": "Filter by state (for list_issues/list_prs)"
                },
                "number": {
                    "type": "integer",
                    "description": "Issue or PR number"
                },
                "title": {
                    "type": "string",
                    "description": "Issue title (for create_issue)"
                },
                "body": {
                    "type": "string",
                    "description": "Issue/review body text"
                },
                "labels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Labels to add (for create_issue)"
                },
                "path": {
                    "type": "string",
                    "description": "File path in repo (for get_file_content)"
                },
                "ref": {
                    "type": "string",
                    "description": "Git ref — branch, tag, or SHA (for get_file_content, trigger_workflow)"
                },
                "event": {
                    "type": "string",
                    "enum": ["APPROVE", "REQUEST_CHANGES", "COMMENT"],
                    "description": "Review event type (for create_pr_review)"
                },
                "workflow_id": {
                    "type": "string",
                    "description": "Workflow ID or filename (for trigger_workflow, get_workflow_runs)"
                },
                "inputs": {
                    "type": "object",
                    "description": "Workflow dispatch inputs (for trigger_workflow)"
                },
                "page": {
                    "type": "integer",
                    "default": 1,
                    "description": "Page number for pagination (for list_issues/list_prs)"
                },
                "per_page": {
                    "type": "integer",
                    "default": 10,
                    "description": "Results per page, max 100 (for list_issues/list_prs)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        match action {
            "list_issues" | "list_prs" | "create_issue" | "get_pr" | "get_issue"
            | "get_pr_files" | "create_pr_review" | "get_file_content" | "trigger_workflow"
            | "get_workflow_runs" => {
                let Some(owner) = params["owner"].as_str() else {
                    return Ok(ToolResult::error("missing 'owner' parameter".to_string()));
                };
                let Some(repo) = params["repo"].as_str() else {
                    return Ok(ToolResult::error("missing 'repo' parameter".to_string()));
                };

                // Validate owner/repo — only alphanumeric, hyphens, dots, underscores
                if !is_valid_github_name(owner) || !is_valid_github_name(repo) {
                    return Ok(ToolResult::error(
                        "owner and repo must contain only alphanumeric characters, hyphens, dots, or underscores".to_string(),
                    ));
                }

                // Extract pagination params with defaults and cap
                let page_num = params["page"].as_u64().unwrap_or(1).max(1);
                let per_page_num = params["per_page"].as_u64().unwrap_or(10).clamp(1, 100);
                let page = page_num.to_string();
                let per_page = per_page_num.to_string();

                let result = match action {
                    "list_issues" => {
                        let state = params["state"].as_str().unwrap_or("open");
                        if !matches!(state, "open" | "closed" | "all") {
                            return Ok(ToolResult::error(format!(
                                "invalid state '{}', must be open, closed, or all",
                                state
                            )));
                        }
                        self.list_issues(owner, repo, state, &page, &per_page).await
                    }
                    "list_prs" => {
                        let state = params["state"].as_str().unwrap_or("open");
                        if !matches!(state, "open" | "closed" | "all") {
                            return Ok(ToolResult::error(format!(
                                "invalid state '{}', must be open, closed, or all",
                                state
                            )));
                        }
                        self.list_prs(owner, repo, state, &page, &per_page).await
                    }
                    "create_issue" => {
                        let Some(title) = params["title"].as_str() else {
                            return Ok(ToolResult::error("missing 'title' parameter".to_string()));
                        };
                        let labels: Option<Vec<&str>> = params["labels"]
                            .as_array()
                            .map(|a| a.iter().filter_map(|v| v.as_str()).collect());
                        self.create_issue(
                            owner,
                            repo,
                            title,
                            params["body"].as_str(),
                            labels.as_ref(),
                        )
                        .await
                    }
                    "get_pr" => {
                        let Some(number) = params["number"].as_u64() else {
                            return Ok(ToolResult::error("missing 'number' parameter".to_string()));
                        };
                        self.get_pr(owner, repo, number).await
                    }
                    "get_issue" => {
                        let Some(number) = params["number"].as_u64() else {
                            return Ok(ToolResult::error("missing 'number' parameter".to_string()));
                        };
                        self.get_issue(owner, repo, number).await
                    }
                    "get_pr_files" => {
                        let Some(number) = params["number"].as_u64() else {
                            return Ok(ToolResult::error("missing 'number' parameter".to_string()));
                        };
                        self.get_pr_files(owner, repo, number).await
                    }
                    "create_pr_review" => {
                        let Some(number) = params["number"].as_u64() else {
                            return Ok(ToolResult::error("missing 'number' parameter".to_string()));
                        };
                        let Some(event) = params["event"].as_str() else {
                            return Ok(ToolResult::error("missing 'event' parameter".to_string()));
                        };
                        if !matches!(event, "APPROVE" | "REQUEST_CHANGES" | "COMMENT") {
                            return Ok(ToolResult::error(format!(
                                "invalid event '{}'. Must be APPROVE, REQUEST_CHANGES, or COMMENT",
                                event
                            )));
                        }
                        let body = params["body"].as_str().unwrap_or("");
                        self.create_pr_review(owner, repo, number, event, body)
                            .await
                    }
                    "get_file_content" => {
                        let Some(file_path) = params["path"].as_str() else {
                            return Ok(ToolResult::error("missing 'path' parameter".to_string()));
                        };
                        self.get_file_content(owner, repo, file_path, params["ref"].as_str())
                            .await
                    }
                    "trigger_workflow" => {
                        let Some(workflow_id) = params["workflow_id"].as_str() else {
                            return Ok(ToolResult::error(
                                "missing 'workflow_id' parameter".to_string(),
                            ));
                        };
                        let git_ref = params["ref"].as_str().unwrap_or("main");
                        let inputs = if params["inputs"].is_null() {
                            None
                        } else {
                            Some(&params["inputs"])
                        };
                        self.trigger_workflow(owner, repo, workflow_id, git_ref, inputs)
                            .await
                    }
                    "get_workflow_runs" => {
                        self.get_workflow_runs(owner, repo, params["workflow_id"].as_str())
                            .await
                    }
                    other => {
                        return Ok(ToolResult::error(format!(
                            "unknown repo action: '{}'",
                            other
                        )));
                    }
                };

                Ok(ToolResult::from_result(result, "GitHub"))
            }
            "notifications" => Ok(ToolResult::from_result(
                self.list_notifications().await,
                "GitHub",
            )),
            _ => Ok(ToolResult::error(format!("unknown action: {}", action))),
        }
    }
}

#[cfg(test)]
mod tests;
