use crate::agent::tools::base::{
    ActionDescriptor, ExecutionContext, SubagentAccess, ToolCapabilities,
};
use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const TODOIST_API: &str = "https://api.todoist.com/api/v1";

pub struct TodoistTool {
    token: String,
    base_url: String,
    client: Client,
}

impl TodoistTool {
    pub fn new(token: String) -> Self {
        Self {
            token,
            base_url: TODOIST_API.to_string(),
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

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    /// Fetch all pages from a paginated v1 endpoint.
    async fn paginated_get(&self, url: &str, base_query: &[(&str, &str)]) -> Result<Vec<Value>> {
        const MAX_PAGES: usize = 10; // Safety limit
        let mut all_items: Vec<Value> = Vec::new();
        let mut cursor: Option<String> = None;

        for _ in 0..MAX_PAGES {
            let mut query: Vec<(&str, &str)> = base_query.to_vec();
            let cursor_val;
            if let Some(ref c) = cursor {
                cursor_val = c.clone();
                query.push(("cursor", &cursor_val));
            }

            let resp = self
                .client
                .get(url)
                .query(&query)
                .header("Authorization", self.auth_header())
                .timeout(Duration::from_secs(15))
                .send()
                .await?;

            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                // Truncate error body to avoid leaking sensitive API internals
                let safe_body = if text.len() > 200 {
                    format!("{}...", &text[..text.floor_char_boundary(200)])
                } else {
                    text.clone()
                };
                anyhow::bail!("Todoist API {}: {}", status, safe_body);
            }
            let body: Value = serde_json::from_str(&text).map_err(|e| {
                anyhow::anyhow!(
                    "Invalid JSON from Todoist: {} (body: {})",
                    e,
                    &text[..text.floor_char_boundary(200)]
                )
            })?;

            // v1 API returns {"results": [...], "next_cursor": ...}
            if let Some(results) = body["results"].as_array() {
                all_items.extend(results.iter().cloned());
            } else if let Some(arr) = body.as_array() {
                all_items.extend(arr.iter().cloned());
                break;
            } else {
                break;
            }

            match body["next_cursor"].as_str() {
                Some(c) if !c.is_empty() => cursor = Some(c.to_string()),
                _ => break,
            }
        }

        if cursor.is_some() {
            tracing::warn!(
                "todoist pagination limit reached ({} pages), additional results may exist",
                MAX_PAGES
            );
        }
        Ok(all_items)
    }

    async fn list_tasks(&self, project_id: Option<&str>, filter: Option<&str>) -> Result<String> {
        // v1 API: /tasks for listing, /tasks/filter for filter queries
        // limit max=200, default=50
        let tasks = if let Some(f) = filter {
            let query = vec![("query", f), ("limit", "200")];
            self.paginated_get(&format!("{}/tasks/filter", self.base_url), &query)
                .await?
        } else {
            let mut query: Vec<(&str, &str)> = vec![("limit", "200")];
            if let Some(pid) = project_id {
                query.push(("project_id", pid));
            }
            self.paginated_get(&format!("{}/tasks", self.base_url), &query)
                .await?
        };
        if tasks.is_empty() {
            return Ok("No tasks found.".to_string());
        }

        let lines: Vec<String> = tasks
            .iter()
            .map(|t| {
                let id = t["id"].as_str().unwrap_or("?");
                let content = t["content"].as_str().unwrap_or("");
                let priority = t["priority"].as_u64().unwrap_or(1);
                let due = t["due"]["string"].as_str().unwrap_or("no due date");
                let labels: Vec<&str> = t["labels"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|l| l.as_str()).collect())
                    .unwrap_or_default();
                let label_str = if labels.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", labels.join(", "))
                };
                // v1 API: priority 1=highest(urgent), 4=lowest(normal)
                let priority_str = match priority {
                    1 => " !!!",
                    2 => " !!",
                    3 => " !",
                    _ => "",
                };
                format!(
                    "- ({}) {}{} | due: {}{}",
                    id, content, priority_str, due, label_str
                )
            })
            .collect();

        Ok(format!("Tasks ({}):\n{}", tasks.len(), lines.join("\n")))
    }

    async fn create_task(
        &self,
        content: &str,
        description: Option<&str>,
        project_id: Option<&str>,
        due_string: Option<&str>,
        priority: Option<u64>,
        labels: Option<Vec<&str>>,
    ) -> Result<String> {
        let mut payload = serde_json::json!({ "content": content });
        if let Some(d) = description {
            payload["description"] = Value::String(d.to_string());
        }
        if let Some(pid) = project_id {
            payload["project_id"] = Value::String(pid.to_string());
        }
        if let Some(due) = due_string {
            payload["due_string"] = Value::String(due.to_string());
        }
        if let Some(p) = priority {
            payload["priority"] = Value::Number(p.into());
        }
        if let Some(l) = labels {
            payload["labels"] = Value::Array(
                l.into_iter()
                    .map(|s| Value::String(s.to_string()))
                    .collect(),
            );
        }

        let resp = self
            .client
            .post(format!("{}/tasks", self.base_url))
            .json(&payload)
            .header("Authorization", self.auth_header())
            .timeout(Duration::from_secs(15))
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("Todoist API {}: {}", status, text);
        }
        let body: Value = serde_json::from_str(&text).map_err(|e| {
            anyhow::anyhow!(
                "Invalid JSON from Todoist: {} (body: {})",
                e,
                &text[..text.floor_char_boundary(200)]
            )
        })?;

        let id = body["id"].as_str().unwrap_or("?");
        // v1 API removed url from response; construct it per migration guide
        let url = format!("https://app.todoist.com/app/task/{}", id);
        Ok(format!("Created task ({}): {} — {}", id, content, url))
    }

    async fn complete_task(&self, task_id: &str) -> Result<String> {
        let resp = self
            .client
            .post(format!(
                "{}/tasks/{}/close",
                self.base_url,
                urlencoding::encode(task_id)
            ))
            .header("Authorization", self.auth_header())
            .timeout(Duration::from_secs(15))
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(format!("Task {} completed.", task_id))
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Todoist API {}: {}", status, body);
        }
    }

    async fn list_projects(&self) -> Result<String> {
        let projects = self
            .paginated_get(&format!("{}/projects", self.base_url), &[("limit", "200")])
            .await?;
        if projects.is_empty() {
            return Ok("No projects found.".to_string());
        }

        let lines: Vec<String> = projects
            .iter()
            .map(|p| {
                let id = p["id"].as_str().unwrap_or("?");
                let name = p["name"].as_str().unwrap_or("");
                let color = p["color"].as_str().unwrap_or("");
                let is_fav = p["is_favorite"].as_bool().unwrap_or(false);
                let fav_str = if is_fav { " *" } else { "" };
                format!("- ({}) {}{} [{}]", id, name, fav_str, color)
            })
            .collect();

        Ok(format!(
            "Projects ({}):\n{}",
            projects.len(),
            lines.join("\n")
        ))
    }

    async fn get_task(&self, task_id: &str) -> Result<String> {
        let resp = self
            .client
            .get(format!(
                "{}/tasks/{}",
                self.base_url,
                urlencoding::encode(task_id)
            ))
            .header("Authorization", self.auth_header())
            .timeout(Duration::from_secs(15))
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("Todoist API {}: {}", status, text);
        }
        let t: Value = serde_json::from_str(&text).map_err(|e| {
            anyhow::anyhow!(
                "Invalid JSON from Todoist: {} (body: {})",
                e,
                &text[..text.floor_char_boundary(200)]
            )
        })?;

        let id = t["id"].as_str().unwrap_or("?");
        let content = t["content"].as_str().unwrap_or("");
        let description = t["description"].as_str().unwrap_or("");
        let priority = t["priority"].as_u64().unwrap_or(1);
        let due = t["due"]["string"].as_str().unwrap_or("no due date");
        let labels: Vec<&str> = t["labels"]
            .as_array()
            .map(|a| a.iter().filter_map(|l| l.as_str()).collect())
            .unwrap_or_default();
        let project_id = t["project_id"].as_str().unwrap_or("?");
        let is_completed = t["is_completed"].as_bool().unwrap_or(false);
        let url = format!("https://app.todoist.com/app/task/{}", id);

        let label_str = if labels.is_empty() {
            String::new()
        } else {
            format!("\nLabels: {}", labels.join(", "))
        };
        let desc_str = if description.is_empty() {
            String::new()
        } else {
            format!("\nDescription: {}", description)
        };
        let status_str = if is_completed { "completed" } else { "open" };

        Ok(format!(
            "Task ({id}): {content}{desc_str}\nPriority: {priority}\nDue: {due}{label_str}\nProject: {project_id}\nStatus: {status_str}\nURL: {url}"
        ))
    }

    async fn update_task(
        &self,
        task_id: &str,
        content: Option<&str>,
        description: Option<&str>,
        due_string: Option<&str>,
        priority: Option<u64>,
        labels: Option<Vec<&str>>,
    ) -> Result<String> {
        let mut payload = serde_json::json!({});
        if let Some(c) = content {
            payload["content"] = Value::String(c.to_string());
        }
        if let Some(d) = description {
            payload["description"] = Value::String(d.to_string());
        }
        if let Some(due) = due_string {
            payload["due_string"] = Value::String(due.to_string());
        }
        if let Some(p) = priority {
            payload["priority"] = Value::Number(p.into());
        }
        if let Some(l) = labels {
            payload["labels"] = Value::Array(
                l.into_iter()
                    .map(|s| Value::String(s.to_string()))
                    .collect(),
            );
        }

        let resp = self
            .client
            .post(format!(
                "{}/tasks/{}",
                self.base_url,
                urlencoding::encode(task_id)
            ))
            .json(&payload)
            .header("Authorization", self.auth_header())
            .timeout(Duration::from_secs(15))
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("Todoist API {}: {}", status, text);
        }

        Ok(format!("Task {} updated.", task_id))
    }

    async fn delete_task(&self, task_id: &str) -> Result<String> {
        let resp = self
            .client
            .delete(format!(
                "{}/tasks/{}",
                self.base_url,
                urlencoding::encode(task_id)
            ))
            .header("Authorization", self.auth_header())
            .timeout(Duration::from_secs(15))
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(format!("Task {} deleted.", task_id))
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Todoist API {}: {}", status, body);
        }
    }

    async fn add_comment(&self, task_id: &str, content: &str) -> Result<String> {
        let payload = serde_json::json!({
            "task_id": task_id,
            "content": content,
        });

        let resp = self
            .client
            .post(format!("{}/comments", self.base_url))
            .json(&payload)
            .header("Authorization", self.auth_header())
            .timeout(Duration::from_secs(15))
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("Todoist API {}: {}", status, text);
        }
        let body: Value = serde_json::from_str(&text).map_err(|e| {
            anyhow::anyhow!(
                "Invalid JSON from Todoist: {} (body: {})",
                e,
                &text[..text.floor_char_boundary(200)]
            )
        })?;

        let id = body["id"].as_str().unwrap_or("?");
        Ok(format!("Comment ({}) added to task {}.", id, task_id))
    }

    async fn list_comments(&self, task_id: &str) -> Result<String> {
        let query = vec![("task_id", task_id), ("limit", "200")];
        let comments = self
            .paginated_get(&format!("{}/comments", self.base_url), &query)
            .await?;

        if comments.is_empty() {
            return Ok(format!("No comments on task {}.", task_id));
        }

        let lines: Vec<String> = comments
            .iter()
            .map(|c| {
                let id = c["id"].as_str().unwrap_or("?");
                let content = c["content"].as_str().unwrap_or("");
                let posted = c["posted_at"].as_str().unwrap_or("?");
                format!("- ({}) [{}] {}", id, posted, content)
            })
            .collect();

        Ok(format!(
            "Comments on task {} ({}):\n{}",
            task_id,
            comments.len(),
            lines.join("\n")
        ))
    }
}

#[async_trait]
impl Tool for TodoistTool {
    fn name(&self) -> &'static str {
        "todoist"
    }

    fn description(&self) -> &'static str {
        "Manage Todoist tasks and projects. Actions: list_tasks, get_task, create_task, update_task, complete_task, delete_task, add_comment, list_comments, list_projects."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 1, 0)
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: vec![
                ActionDescriptor {
                    name: "list_tasks",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "get_task",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "create_task",
                    read_only: false,
                },
                ActionDescriptor {
                    name: "update_task",
                    read_only: false,
                },
                ActionDescriptor {
                    name: "complete_task",
                    read_only: false,
                },
                ActionDescriptor {
                    name: "delete_task",
                    read_only: false,
                },
                ActionDescriptor {
                    name: "add_comment",
                    read_only: false,
                },
                ActionDescriptor {
                    name: "list_comments",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "list_projects",
                    read_only: true,
                },
            ],
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_tasks", "get_task", "create_task", "update_task", "complete_task", "delete_task", "add_comment", "list_comments", "list_projects"],
                    "description": "Action to perform"
                },
                "content": {
                    "type": "string",
                    "description": "Task content/title (for create_task, update_task)"
                },
                "description": {
                    "type": "string",
                    "description": "Task description (for create_task, update_task)"
                },
                "project_id": {
                    "type": "string",
                    "description": "Project ID to filter tasks or assign new task to"
                },
                "filter": {
                    "type": "string",
                    "description": "Todoist filter query (for list_tasks), e.g. 'today', 'overdue', '#Work'"
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID (for get_task, update_task, complete_task, delete_task, add_comment, list_comments)"
                },
                "due_string": {
                    "type": "string",
                    "description": "Due date in natural language, e.g. 'tomorrow', 'every friday', 'Jan 15'"
                },
                "priority": {
                    "type": "integer",
                    "enum": [1, 2, 3, 4],
                    "description": "Task priority (1=urgent, 4=normal)"
                },
                "labels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Labels to apply to the task"
                },
                "comment_content": {
                    "type": "string",
                    "description": "Comment text (for add_comment)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        let result = match action {
            "list_tasks" => {
                self.list_tasks(params["project_id"].as_str(), params["filter"].as_str())
                    .await
            }
            "get_task" => {
                let Some(task_id) = params["task_id"].as_str() else {
                    return Ok(ToolResult::error("missing 'task_id' parameter".to_string()));
                };
                self.get_task(task_id).await
            }
            "create_task" => {
                let Some(content) = params["content"].as_str() else {
                    return Ok(ToolResult::error("missing 'content' parameter".to_string()));
                };
                let priority = params["priority"].as_u64();
                if let Some(p) = priority
                    && !(1..=4).contains(&p)
                {
                    return Ok(ToolResult::error(
                        "priority must be 1 (normal) to 4 (urgent)".to_string(),
                    ));
                }
                let labels: Option<Vec<&str>> = params["labels"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect());
                self.create_task(
                    content,
                    params["description"].as_str(),
                    params["project_id"].as_str(),
                    params["due_string"].as_str(),
                    priority,
                    labels,
                )
                .await
            }
            "update_task" => {
                let Some(task_id) = params["task_id"].as_str() else {
                    return Ok(ToolResult::error("missing 'task_id' parameter".to_string()));
                };
                let labels: Option<Vec<&str>> = params["labels"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect());
                self.update_task(
                    task_id,
                    params["content"].as_str(),
                    params["description"].as_str(),
                    params["due_string"].as_str(),
                    params["priority"].as_u64(),
                    labels,
                )
                .await
            }
            "complete_task" => {
                let Some(task_id) = params["task_id"].as_str() else {
                    return Ok(ToolResult::error("missing 'task_id' parameter".to_string()));
                };
                self.complete_task(task_id).await
            }
            "delete_task" => {
                let Some(task_id) = params["task_id"].as_str() else {
                    return Ok(ToolResult::error("missing 'task_id' parameter".to_string()));
                };
                self.delete_task(task_id).await
            }
            "add_comment" => {
                let Some(task_id) = params["task_id"].as_str() else {
                    return Ok(ToolResult::error("missing 'task_id' parameter".to_string()));
                };
                let Some(content) = params["comment_content"].as_str() else {
                    return Ok(ToolResult::error(
                        "missing 'comment_content' parameter".to_string(),
                    ));
                };
                self.add_comment(task_id, content).await
            }
            "list_comments" => {
                let Some(task_id) = params["task_id"].as_str() else {
                    return Ok(ToolResult::error("missing 'task_id' parameter".to_string()));
                };
                self.list_comments(task_id).await
            }
            "list_projects" => self.list_projects().await,
            _ => return Ok(ToolResult::error(format!("unknown action: {}", action))),
        };

        match result {
            Ok(content) => Ok(ToolResult::new(content)),
            Err(e) => Ok(ToolResult::error(format!("Todoist error: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests;
