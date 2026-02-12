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
mod tests;
