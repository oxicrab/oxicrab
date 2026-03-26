use oxicrab_core::actions;
use oxicrab_core::require_param;
use oxicrab_core::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory};
use oxicrab_core::tools::base::{Tool, ToolResult};
use oxicrab_core::utils::url_params::{validate_identifier, validate_url_segment};

use anyhow::Result;
use async_trait::async_trait;
use base64::Engine;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tracing::{debug, warn};

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

    async fn api_get_inner(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let req = self.github_headers(
            self.client
                .get(format!("{}{}", self.base_url, path))
                .query(query),
        );
        self.api_send(req).await
    }

    async fn api_get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        match self.api_get_inner(path, query).await {
            Ok(v) => Ok(v),
            Err(e) if e.to_string().contains("GitHub API 5") => {
                debug!("GitHub API server error, retrying once after 2s");
                tokio::time::sleep(Duration::from_secs(2)).await;
                self.api_get_inner(path, query).await
            }
            Err(e) => Err(e),
        }
    }

    async fn api_post_inner(&self, path: &str, body: &Value) -> Result<Value> {
        let req = self.github_headers(
            self.client
                .post(format!("{}{}", self.base_url, path))
                .json(body),
        );
        self.api_send(req).await
    }

    async fn api_post(&self, path: &str, body: &Value) -> Result<Value> {
        match self.api_post_inner(path, body).await {
            Ok(v) => Ok(v),
            Err(e) if e.to_string().contains("GitHub API 5") => {
                debug!("GitHub API server error, retrying once after 2s");
                tokio::time::sleep(Duration::from_secs(2)).await;
                self.api_post_inner(path, body).await
            }
            Err(e) => Err(e),
        }
    }

    async fn api_patch(&self, path: &str, body: &Value) -> Result<Value> {
        let req = self.github_headers(
            self.client
                .patch(format!("{}{}", self.base_url, path))
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
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            anyhow::bail!("GitHub API rate limit exceeded, try again later");
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&text).map_or_else(
                |_| "Unknown error".to_string(),
                |v| Self::sanitize_api_error(&v),
            );
            anyhow::bail!("GitHub API {status}: {msg}");
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
    ) -> Result<(String, Vec<Value>)> {
        let json = self
            .api_get(
                &format!("/repos/{owner}/{repo}/issues"),
                &[("state", state), ("page", page), ("per_page", per_page)],
            )
            .await?;

        let issues = json.as_array().map(Vec::as_slice).unwrap_or_default();
        if issues.is_empty() {
            return Ok((format!("No {state} issues in {owner}/{repo}."), vec![]));
        }

        // Filter out PRs (GitHub issues API includes PRs)
        let real_issues: Vec<Value> = issues
            .iter()
            .filter(|i| i.get("pull_request").is_none())
            .cloned()
            .collect();

        let lines: Vec<String> = real_issues
            .iter()
            .map(|i| {
                let number = i["number"].as_u64().unwrap_or(0);
                let title = i["title"].as_str().unwrap_or_default();
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
                format!("#{number} {title} (by {user}){label_str}")
            })
            .collect();

        Ok((
            format!(
                "Issues ({}) in {}/{} (page {}):\n{}",
                state,
                owner,
                repo,
                page,
                lines.join("\n")
            ),
            real_issues,
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
            .api_post(&format!("/repos/{owner}/{repo}/issues"), &payload)
            .await?;

        let number = result["number"].as_u64().unwrap_or(0);
        let url = result["html_url"].as_str().unwrap_or_default();
        Ok(format!("Created issue #{number}: {url}"))
    }

    async fn close_issue(&self, owner: &str, repo: &str, number: u64) -> Result<String> {
        let result = self
            .api_patch(
                &format!("/repos/{owner}/{repo}/issues/{number}"),
                &serde_json::json!({"state": "closed"}),
            )
            .await?;

        let title = result["title"].as_str().unwrap_or_default();
        let url = result["html_url"].as_str().unwrap_or_default();
        Ok(format!("Closed issue #{number} ({title}): {url}"))
    }

    async fn reopen_issue(&self, owner: &str, repo: &str, number: u64) -> Result<String> {
        let result = self
            .api_patch(
                &format!("/repos/{owner}/{repo}/issues/{number}"),
                &serde_json::json!({"state": "open"}),
            )
            .await?;

        let title = result["title"].as_str().unwrap_or_default();
        let url = result["html_url"].as_str().unwrap_or_default();
        Ok(format!("Reopened issue #{number} ({title}): {url}"))
    }

    async fn comment_on_issue(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> Result<String> {
        let result = self
            .api_post(
                &format!("/repos/{owner}/{repo}/issues/{number}/comments"),
                &serde_json::json!({"body": body}),
            )
            .await?;

        let url = result["html_url"].as_str().unwrap_or_default();
        Ok(format!("Comment added to #{number}: {url}"))
    }

    async fn close_pr(&self, owner: &str, repo: &str, number: u64) -> Result<String> {
        let result = self
            .api_patch(
                &format!("/repos/{owner}/{repo}/pulls/{number}"),
                &serde_json::json!({"state": "closed"}),
            )
            .await?;

        let title = result["title"].as_str().unwrap_or_default();
        Ok(format!("Closed PR #{number} ({title})"))
    }

    async fn merge_pr(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        method: Option<&str>,
    ) -> Result<String> {
        let mut payload = serde_json::json!({});
        if let Some(m) = method {
            payload["merge_method"] = Value::String(m.to_string());
        }

        self.client
            .put(format!(
                "{}/repos/{owner}/{repo}/pulls/{number}/merge",
                self.base_url
            ))
            .header("Authorization", format!("token {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "oxicrab")
            .json(&payload)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("GitHub API error: {e}"))?;

        Ok(format!("Merged PR #{number}"))
    }

    async fn list_prs(
        &self,
        owner: &str,
        repo: &str,
        state: &str,
        page: &str,
        per_page: &str,
    ) -> Result<(String, Vec<Value>)> {
        let json = self
            .api_get(
                &format!("/repos/{owner}/{repo}/pulls"),
                &[("state", state), ("page", page), ("per_page", per_page)],
            )
            .await?;

        let prs = json.as_array().cloned().unwrap_or_default();
        if prs.is_empty() {
            return Ok((format!("No {state} PRs in {owner}/{repo}."), vec![]));
        }

        let lines: Vec<String> = prs
            .iter()
            .map(|pr| {
                let number = pr["number"].as_u64().unwrap_or(0);
                let title = pr["title"].as_str().unwrap_or_default();
                let user = pr["user"]["login"].as_str().unwrap_or("?");
                let draft = if pr["draft"].as_bool().unwrap_or_default() {
                    " (draft)"
                } else {
                    ""
                };
                let mergeable_state = pr["mergeable_state"].as_str().unwrap_or_default();
                let state_str = if mergeable_state.is_empty() {
                    String::new()
                } else {
                    format!(" [{mergeable_state}]")
                };
                format!("#{number} {title} (by {user}){draft}{state_str}")
            })
            .collect();

        Ok((
            format!(
                "Pull requests ({}) in {}/{} (page {}):\n{}",
                state,
                owner,
                repo,
                page,
                lines.join("\n")
            ),
            prs,
        ))
    }

    async fn get_pr(&self, owner: &str, repo: &str, number: u64) -> Result<(String, Value)> {
        let pr = self
            .api_get(&format!("/repos/{owner}/{repo}/pulls/{number}"), &[])
            .await?;

        let title = pr["title"].as_str().unwrap_or_default();
        let state = pr["state"].as_str().unwrap_or_default();
        let user = pr["user"]["login"].as_str().unwrap_or("?");
        let body = pr["body"].as_str().unwrap_or("(no description)");
        let merged = pr["merged"].as_bool().unwrap_or_default();
        let additions = pr["additions"].as_u64().unwrap_or(0);
        let deletions = pr["deletions"].as_u64().unwrap_or(0);
        let changed_files = pr["changed_files"].as_u64().unwrap_or(0);
        let head = pr["head"]["ref"].as_str().unwrap_or("?");
        let base = pr["base"]["ref"].as_str().unwrap_or("?");

        // Fetch checks status
        let sha = pr["head"]["sha"].as_str().unwrap_or_default();
        let checks_str = if sha.is_empty() {
            "CI: unknown".to_string()
        } else {
            match self
                .api_get(&format!("/repos/{owner}/{repo}/commits/{sha}/status"), &[])
                .await
            {
                Ok(status) => {
                    let state = status["state"].as_str().unwrap_or("unknown");
                    let total = status["total_count"].as_u64().unwrap_or(0);
                    format!("CI: {state} ({total} checks)")
                }
                Err(_) => "CI: unknown".to_string(),
            }
        };

        let status_str = if merged { "merged" } else { state };

        Ok((
            format!(
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
            ),
            pr,
        ))
    }

    async fn get_issue(&self, owner: &str, repo: &str, number: u64) -> Result<(String, Value)> {
        let issue = self
            .api_get(&format!("/repos/{owner}/{repo}/issues/{number}"), &[])
            .await?;

        let title = issue["title"].as_str().unwrap_or_default();
        let state = issue["state"].as_str().unwrap_or_default();
        let user = issue["user"]["login"].as_str().unwrap_or("?");
        let body = issue["body"].as_str().unwrap_or("(no description)");
        let comments = issue["comments"].as_u64().unwrap_or(0);
        let url = issue["html_url"].as_str().unwrap_or_default();

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

        Ok((
            format!(
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
            ),
            issue,
        ))
    }

    async fn get_pr_files(&self, owner: &str, repo: &str, number: u64) -> Result<String> {
        let per_page = 100;
        let max_pages = 3;
        let mut all_files: Vec<Value> = Vec::new();
        let mut truncated = false;

        for page in 1..=max_pages {
            let page_str = page.to_string();
            let per_page_str = per_page.to_string();
            let json = self
                .api_get(
                    &format!("/repos/{owner}/{repo}/pulls/{number}/files"),
                    &[("per_page", &per_page_str), ("page", &page_str)],
                )
                .await?;

            let files = json.as_array().map(Vec::as_slice).unwrap_or_default();
            all_files.extend(files.iter().cloned());

            if files.len() < per_page {
                break;
            }
            if page == max_pages {
                truncated = true;
            }
        }

        if all_files.is_empty() {
            return Ok(format!("No files changed in PR #{number}"));
        }

        let lines: Vec<String> = all_files
            .iter()
            .map(|f| {
                let filename = f["filename"].as_str().unwrap_or("?");
                let status = f["status"].as_str().unwrap_or("?");
                let additions = f["additions"].as_u64().unwrap_or(0);
                let deletions = f["deletions"].as_u64().unwrap_or(0);
                let patch: String = f["patch"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(200)
                    .collect();
                let patch_str = if patch.is_empty() {
                    String::new()
                } else {
                    format!("\n  {}", patch.replace('\n', "\n  "))
                };
                format!("{filename} ({status}) +{additions} −{deletions}{patch_str}")
            })
            .collect();

        let truncation_note = if truncated {
            format!(
                "\n\n(showing first {} files, more may be available)",
                all_files.len()
            )
        } else {
            String::new()
        };

        Ok(format!(
            "Files in PR #{} ({} files):\n{}{}",
            number,
            all_files.len(),
            lines.join("\n\n"),
            truncation_note
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
                &format!("/repos/{owner}/{repo}/pulls/{number}/reviews"),
                &payload,
            )
            .await?;

        let review_id = result["id"].as_u64().unwrap_or(0);
        let state = result["state"].as_str().unwrap_or_default();
        Ok(format!(
            "Created review #{review_id} on PR #{number}: {state}"
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
                &format!("/repos/{owner}/{repo}/contents/{encoded_path}"),
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
                    format!("{name} ({kind}, {size} bytes)")
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
        let encoding = json["encoding"].as_str().unwrap_or_default();
        let content_b64 = json["content"].as_str().unwrap_or_default();
        let name = json["name"].as_str().unwrap_or(file_path);
        let size = json["size"].as_u64().unwrap_or(0);

        if encoding != "base64" {
            return Ok(format!("File {name} ({size} bytes, encoding: {encoding})"));
        }

        // Decode base64 content (GitHub adds newlines in the base64)
        let cleaned: String = content_b64.chars().filter(|c| !c.is_whitespace()).collect();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&cleaned)
            .map_err(|e| anyhow::anyhow!("Failed to decode base64: {e}"))?;

        // Detect binary files: if >10% of bytes are non-printable (excluding common whitespace), skip content
        let non_printable = decoded
            .iter()
            .filter(|&&b| b < 0x20 && b != b'\n' && b != b'\r' && b != b'\t')
            .count();
        if decoded.len() > 32 && non_printable * 10 > decoded.len() {
            return Ok(format!(
                "Binary file: {} ({} bytes, encoding: {})",
                file_path,
                decoded.len(),
                encoding
            ));
        }

        let text = String::from_utf8_lossy(&decoded);

        // Truncate at 10k chars
        let truncated: String = text.chars().take(10_000).collect();
        let suffix = if text.chars().count() > 10_000 {
            "\n\n... (truncated)"
        } else {
            ""
        };

        Ok(format!(
            "File: {name} ({size} bytes)\n\n{truncated}{suffix}"
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
            &format!("/repos/{owner}/{repo}/actions/workflows/{workflow_id}/dispatches"),
            &payload,
        )
        .await?;

        Ok(format!(
            "Triggered workflow {workflow_id} on {owner}/{repo} (ref: {git_ref})"
        ))
    }

    async fn get_workflow_runs(
        &self,
        owner: &str,
        repo: &str,
        workflow_id: Option<&str>,
        page: u64,
    ) -> Result<String> {
        let path = match workflow_id {
            Some(wid) => format!("/repos/{owner}/{repo}/actions/workflows/{wid}/runs"),
            None => format!("/repos/{owner}/{repo}/actions/runs"),
        };

        let page_str = page.to_string();
        let json = self
            .api_get(&path, &[("per_page", "10"), ("page", &page_str)])
            .await?;

        let total_count = json["total_count"].as_u64().unwrap_or(0);
        let runs = json["workflow_runs"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();

        if runs.is_empty() {
            return Ok(format!("No workflow runs found in {owner}/{repo}."));
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
                let url = r["html_url"].as_str().unwrap_or_default();
                format!("#{id} {name} — {status} ({conclusion}) on {branch} [{created}]\n  {url}")
            })
            .collect();

        Ok(format!(
            "Workflow runs in {}/{} (page {}, {} shown, {} total):\n{}",
            owner,
            repo,
            page,
            runs.len(),
            total_count,
            lines.join("\n")
        ))
    }

    async fn list_notifications(&self, page: u64) -> Result<String> {
        let page_str = page.to_string();
        let json = self
            .api_get("/notifications", &[("per_page", "15"), ("page", &page_str)])
            .await?;

        let notifs = json.as_array().map(Vec::as_slice).unwrap_or_default();
        if notifs.is_empty() {
            return Ok("No unread notifications.".to_string());
        }

        let lines: Vec<String> = notifs
            .iter()
            .map(|n| {
                let reason = n["reason"].as_str().unwrap_or("?");
                let title = n["subject"]["title"].as_str().unwrap_or_default();
                let kind = n["subject"]["type"].as_str().unwrap_or_default();
                let repo = n["repository"]["full_name"].as_str().unwrap_or_default();
                format!("[{reason}] {title} — {repo} ({kind})")
            })
            .collect();

        Ok(format!(
            "Unread notifications (page {}, {} shown):\n{}",
            page,
            notifs.len(),
            lines.join("\n")
        ))
    }
}

/// UTF-8 safe label truncation for button labels.
fn truncate_label(prefix: &str, text: &str, max_text_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_text_chars {
        format!("{prefix}{text}")
    } else {
        let truncated: String = text
            .chars()
            .take(max_text_chars.saturating_sub(3))
            .collect();
        format!("{prefix}{truncated}...")
    }
}

/// Build suggested "View" buttons for open issues (max 5).
fn build_issue_buttons(issues: &[Value], owner: &str, repo: &str) -> Vec<Value> {
    let mut buttons = Vec::new();
    for issue in issues {
        if buttons.len() >= 5 {
            break;
        }
        let state = issue["state"].as_str().unwrap_or_default();
        if state != "open" {
            continue;
        }
        let number = issue["number"].as_u64().unwrap_or(0);
        if number == 0 {
            continue;
        }
        let title = issue["title"].as_str().unwrap_or("issue");
        let label = truncate_label("View: ", title, 22);
        buttons.push(serde_json::json!({
            "id": format!("view-issue-{number}"),
            "label": label,
            "style": "primary",
            "context": serde_json::json!({
                "tool": "github",
                "params": {
                    "action": "get_issue",
                    "owner": owner,
                    "repo": repo,
                    "number": number
                }
            }).to_string()
        }));
    }
    buttons
}

/// Build contextual buttons for a single issue detail view.
/// Open issues get "Close", closed issues get "Reopen".
fn build_issue_detail_buttons(issue: &Value, owner: &str, repo: &str) -> Vec<Value> {
    let state = issue["state"].as_str().unwrap_or_default();
    let number = issue["number"].as_u64().unwrap_or(0);
    if number == 0 {
        return vec![];
    }
    let title = issue["title"].as_str().unwrap_or("issue");
    if state == "open" {
        let label = truncate_label("Close: ", title, 22);
        vec![serde_json::json!({
            "id": format!("close-issue-{number}"),
            "label": label,
            "style": "danger",
            "context": serde_json::json!({
                "tool": "github",
                "params": {"action": "close_issue", "owner": owner, "repo": repo, "number": number}
            }).to_string()
        })]
    } else {
        let label = truncate_label("Reopen: ", title, 22);
        vec![serde_json::json!({
            "id": format!("reopen-issue-{number}"),
            "label": label,
            "style": "success",
            "context": serde_json::json!({
                "tool": "github",
                "params": {"action": "reopen_issue", "owner": owner, "repo": repo, "number": number}
            }).to_string()
        })]
    }
}

/// Build suggested "Approve" buttons for open PRs (max 5).
fn build_pr_list_buttons(prs: &[Value], owner: &str, repo: &str) -> Vec<Value> {
    let mut buttons = Vec::new();
    for pr in prs {
        if buttons.len() >= 5 {
            break;
        }
        let state = pr["state"].as_str().unwrap_or_default();
        if state != "open" {
            continue;
        }
        let number = pr["number"].as_u64().unwrap_or(0);
        if number == 0 {
            continue;
        }
        let title = pr["title"].as_str().unwrap_or("PR");
        let label = truncate_label("Approve: ", title, 20);
        buttons.push(serde_json::json!({
            "id": format!("approve-pr-{number}"),
            "label": label,
            "style": "primary",
            "context": serde_json::json!({
                "tool": "github",
                "params": {
                    "action": "create_pr_review",
                    "owner": owner,
                    "repo": repo,
                    "number": number,
                    "event": "APPROVE",
                    "body": ""
                }
            }).to_string()
        }));
    }
    buttons
}

/// Build "Approve" and "Request Changes" buttons for a single open PR.
fn build_pr_detail_buttons(pr: &Value, owner: &str, repo: &str) -> Vec<Value> {
    let state = pr["state"].as_str().unwrap_or_default();
    let merged = pr["merged"].as_bool().unwrap_or_default();
    if state != "open" || merged {
        return vec![];
    }
    let number = pr["number"].as_u64().unwrap_or(0);
    if number == 0 {
        return vec![];
    }
    vec![
        serde_json::json!({
            "id": format!("merge-pr-{number}"),
            "label": "Merge",
            "style": "success",
            "context": serde_json::json!({
                "tool": "github",
                "params": {
                    "action": "merge_pr",
                    "owner": owner,
                    "repo": repo,
                    "number": number
                }
            }).to_string()
        }),
        serde_json::json!({
            "id": format!("approve-pr-{number}"),
            "label": "Approve",
            "style": "primary",
            "context": serde_json::json!({
                "tool": "github",
                "params": {
                    "action": "create_pr_review",
                    "owner": owner,
                    "repo": repo,
                    "number": number,
                    "event": "APPROVE",
                    "body": ""
                }
            }).to_string()
        }),
    ]
}

#[async_trait]
impl Tool for GitHubTool {
    fn name(&self) -> &'static str {
        "github"
    }

    fn description(&self) -> &'static str {
        "Interact with GitHub. Actions: list_issues, create_issue, get_issue, close_issue, \
         reopen_issue, comment_on_issue, list_prs, get_pr, close_pr, merge_pr, get_pr_files, \
         create_pr_review, get_file_content, trigger_workflow, get_workflow_runs, notifications."
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
                close_issue,
                reopen_issue,
                comment_on_issue,
                list_prs: ro,
                get_pr: ro,
                close_pr,
                merge_pr,
                get_pr_files: ro,
                create_pr_review,
                get_file_content: ro,
                trigger_workflow,
                get_workflow_runs: ro,
                notifications: ro,
            ],
            category: ToolCategory::Development,
        }
    }

    fn requires_approval_for_action(&self, action: &str) -> bool {
        matches!(
            action,
            "create_issue"
                | "close_issue"
                | "reopen_issue"
                | "comment_on_issue"
                | "close_pr"
                | "merge_pr"
                | "create_pr_review"
                | "trigger_workflow"
        )
    }

    fn usage_examples(&self) -> Vec<oxicrab_core::tools::base::ToolExample> {
        vec![
            oxicrab_core::tools::base::ToolExample {
                user_request: "show open issues in myorg/myrepo".into(),
                params: serde_json::json!({"action": "list_issues", "owner": "myorg", "repo": "myrepo"}),
            },
            oxicrab_core::tools::base::ToolExample {
                user_request: "list open pull requests".into(),
                params: serde_json::json!({"action": "list_prs", "owner": "myorg", "repo": "myrepo"}),
            },
            oxicrab_core::tools::base::ToolExample {
                user_request: "get PR #42 details".into(),
                params: serde_json::json!({"action": "get_pr", "owner": "myorg", "repo": "myrepo", "number": 42}),
            },
        ]
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "list_issues", "create_issue", "get_issue", "close_issue",
                        "reopen_issue", "comment_on_issue",
                        "list_prs", "get_pr", "close_pr", "merge_pr",
                        "get_pr_files", "create_pr_review",
                        "get_file_content", "trigger_workflow", "get_workflow_runs",
                        "notifications"
                    ],
                    "description": "Action to perform. 'get_pr' returns PR metadata \
                     (title, description, reviewers, status). 'get_pr_files' returns the \
                     changed files and diff. 'notifications' lists unread notifications."
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
                    "description": "Body text (for create_issue, comment_on_issue, create_pr_review)"
                },
                "merge_method": {
                    "type": "string",
                    "enum": ["merge", "squash", "rebase"],
                    "description": "Merge method for merge_pr (default: merge)"
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
                    "description": "Page number for pagination (for list_issues, list_prs, get_workflow_runs, notifications)"
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
        let action = require_param!(params, "action");

        match action {
            "list_issues" | "list_prs" | "create_issue" | "get_pr" | "get_issue"
            | "close_issue" | "reopen_issue" | "comment_on_issue" | "close_pr" | "merge_pr"
            | "get_pr_files" | "create_pr_review" | "get_file_content" | "trigger_workflow"
            | "get_workflow_runs" => {
                let Some(owner) = params["owner"].as_str() else {
                    return Ok(ToolResult::error("missing 'owner' parameter".to_string()));
                };
                let Some(repo) = params["repo"].as_str() else {
                    return Ok(ToolResult::error("missing 'repo' parameter".to_string()));
                };

                if let Err(e) = validate_identifier(owner, "owner") {
                    return Ok(ToolResult::error(e));
                }
                if let Err(e) = validate_identifier(repo, "repo") {
                    return Ok(ToolResult::error(e));
                }

                // Extract pagination params with defaults and cap
                let page_num = params["page"].as_u64().unwrap_or(1).max(1);
                let per_page_num = params["per_page"].as_u64().unwrap_or(10).clamp(1, 100);
                let page = page_num.to_string();
                let per_page = per_page_num.to_string();

                // Actions that produce suggested buttons
                match action {
                    "list_issues" => {
                        let state = params["state"].as_str().unwrap_or("open");
                        if !matches!(state, "open" | "closed" | "all") {
                            return Ok(ToolResult::error(format!(
                                "invalid state '{state}', must be open, closed, or all"
                            )));
                        }
                        return match self.list_issues(owner, repo, state, &page, &per_page).await {
                            Ok((text, issues)) => {
                                let buttons = build_issue_buttons(&issues, owner, repo);
                                Ok(ToolResult::new(text).with_buttons(buttons))
                            }
                            Err(e) => Ok(ToolResult::error(format!("GitHub error: {e}"))),
                        };
                    }
                    "get_issue" => {
                        let Some(number) = params["number"].as_u64() else {
                            return Ok(ToolResult::error("missing 'number' parameter".to_string()));
                        };
                        return match self.get_issue(owner, repo, number).await {
                            Ok((text, issue)) => {
                                let buttons = build_issue_detail_buttons(&issue, owner, repo);
                                Ok(ToolResult::new(text).with_buttons(buttons))
                            }
                            Err(e) => Ok(ToolResult::error(format!("GitHub error: {e}"))),
                        };
                    }
                    "list_prs" => {
                        let state = params["state"].as_str().unwrap_or("open");
                        if !matches!(state, "open" | "closed" | "all") {
                            return Ok(ToolResult::error(format!(
                                "invalid state '{state}', must be open, closed, or all"
                            )));
                        }
                        return match self.list_prs(owner, repo, state, &page, &per_page).await {
                            Ok((text, prs)) => {
                                let buttons = build_pr_list_buttons(&prs, owner, repo);
                                Ok(ToolResult::new(text).with_buttons(buttons))
                            }
                            Err(e) => Ok(ToolResult::error(format!("GitHub error: {e}"))),
                        };
                    }
                    "get_pr" => {
                        let Some(number) = params["number"].as_u64() else {
                            return Ok(ToolResult::error("missing 'number' parameter".to_string()));
                        };
                        return match self.get_pr(owner, repo, number).await {
                            Ok((text, pr)) => {
                                let buttons = build_pr_detail_buttons(&pr, owner, repo);
                                Ok(ToolResult::new(text).with_buttons(buttons))
                            }
                            Err(e) => Ok(ToolResult::error(format!("GitHub error: {e}"))),
                        };
                    }
                    _ => {}
                }

                // Remaining actions without buttons
                let result = match action {
                    "close_issue" => {
                        let Some(number) = params["number"].as_u64() else {
                            return Ok(ToolResult::error("missing 'number' parameter".to_string()));
                        };
                        self.close_issue(owner, repo, number).await
                    }
                    "reopen_issue" => {
                        let Some(number) = params["number"].as_u64() else {
                            return Ok(ToolResult::error("missing 'number' parameter".to_string()));
                        };
                        self.reopen_issue(owner, repo, number).await
                    }
                    "comment_on_issue" => {
                        let Some(number) = params["number"].as_u64() else {
                            return Ok(ToolResult::error("missing 'number' parameter".to_string()));
                        };
                        let Some(body) = params["body"].as_str() else {
                            return Ok(ToolResult::error("missing 'body' parameter".to_string()));
                        };
                        self.comment_on_issue(owner, repo, number, body).await
                    }
                    "close_pr" => {
                        let Some(number) = params["number"].as_u64() else {
                            return Ok(ToolResult::error("missing 'number' parameter".to_string()));
                        };
                        self.close_pr(owner, repo, number).await
                    }
                    "merge_pr" => {
                        let Some(number) = params["number"].as_u64() else {
                            return Ok(ToolResult::error("missing 'number' parameter".to_string()));
                        };
                        let method = params["merge_method"].as_str();
                        if let Some(m) = method
                            && !matches!(m, "merge" | "squash" | "rebase")
                        {
                            return Ok(ToolResult::error(format!(
                                "invalid merge_method '{m}'. Must be merge, squash, or rebase"
                            )));
                        }
                        self.merge_pr(owner, repo, number, method).await
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
                                "invalid event '{event}'. Must be APPROVE, REQUEST_CHANGES, or COMMENT"
                            )));
                        }
                        let body = params["body"].as_str().unwrap_or_default();
                        self.create_pr_review(owner, repo, number, event, body)
                            .await
                    }
                    "get_file_content" => {
                        let Some(file_path) = params["path"].as_str() else {
                            return Ok(ToolResult::error("missing 'path' parameter".to_string()));
                        };
                        if file_path.split('/').any(|seg| seg == ".." || seg == ".") {
                            return Ok(ToolResult::error(
                                "file path must not contain '.' or '..' segments".to_string(),
                            ));
                        }
                        self.get_file_content(owner, repo, file_path, params["ref"].as_str())
                            .await
                    }
                    "trigger_workflow" => {
                        let Some(workflow_id) = params["workflow_id"].as_str() else {
                            return Ok(ToolResult::error(
                                "missing 'workflow_id' parameter".to_string(),
                            ));
                        };
                        if let Err(e) = validate_url_segment(workflow_id, "workflow_id") {
                            return Ok(ToolResult::error(e));
                        }
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
                        if let Some(wid) = params["workflow_id"].as_str()
                            && let Err(e) = validate_url_segment(wid, "workflow_id")
                        {
                            return Ok(ToolResult::error(e));
                        }
                        self.get_workflow_runs(
                            owner,
                            repo,
                            params["workflow_id"].as_str(),
                            page_num,
                        )
                        .await
                    }
                    other => {
                        return Ok(ToolResult::error(format!("unknown repo action: '{other}'")));
                    }
                };

                Ok(ToolResult::from_result(result, "GitHub"))
            }
            "notifications" => {
                let page_num = params["page"].as_u64().unwrap_or(1).max(1);
                Ok(ToolResult::from_result(
                    self.list_notifications(page_num).await,
                    "GitHub",
                ))
            }
            _ => Ok(ToolResult::error(format!("unknown action: {action}"))),
        }
    }
}

#[cfg(test)]
mod tests;
