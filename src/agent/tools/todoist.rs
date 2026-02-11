use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tracing::debug;

const TODOIST_API: &str = "https://api.todoist.com/api/v1";

pub struct TodoistTool {
    token: String,
    client: Client,
}

impl TodoistTool {
    pub fn new(token: String) -> Self {
        Self {
            token,
            client: Client::new(),
        }
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    /// Fetch all pages from a paginated v1 endpoint.
    async fn paginated_get(&self, url: &str, base_query: &[(&str, &str)]) -> Result<Vec<Value>> {
        let mut all_items: Vec<Value> = Vec::new();
        let mut cursor: Option<String> = None;
        const MAX_PAGES: usize = 10; // Safety limit

        for page_num in 0..MAX_PAGES {
            let mut query: Vec<(&str, &str)> = base_query.to_vec();
            let cursor_val;
            if let Some(ref c) = cursor {
                cursor_val = c.clone();
                query.push(("cursor", &cursor_val));
            }

            debug!(
                "Todoist request: GET {} query={:?} page={}",
                url, query, page_num
            );

            let resp = self
                .client
                .get(url)
                .query(&query)
                .header("Authorization", self.auth_header())
                .timeout(Duration::from_secs(15))
                .send()
                .await?;

            let status = resp.status();
            let final_url = resp.url().to_string();
            let text = resp.text().await.unwrap_or_default();
            debug!(
                "Todoist response: status={}, url={}, body_len={}, body_preview={}",
                status,
                final_url,
                text.len(),
                &text[..text.len().min(500)]
            );
            if !status.is_success() {
                anyhow::bail!("Todoist API {}: {}", status, text);
            }
            let body: Value = serde_json::from_str(&text).map_err(|e| {
                anyhow::anyhow!(
                    "Invalid JSON from Todoist: {} (body: {})",
                    e,
                    &text[..text.len().min(200)]
                )
            })?;

            // v1 API returns {"results": [...], "next_cursor": ...}
            let page_count;
            if let Some(results) = body["results"].as_array() {
                page_count = results.len();
                all_items.extend(results.iter().cloned());
            } else if let Some(arr) = body.as_array() {
                // Fallback for bare arrays
                page_count = arr.len();
                all_items.extend(arr.iter().cloned());
                debug!(
                    "Todoist returned bare array ({} items), no pagination",
                    page_count
                );
                break;
            } else {
                debug!(
                    "Todoist unexpected response shape: {}",
                    &text[..text.len().min(300)]
                );
                break;
            }

            let next_raw = &body["next_cursor"];
            let next_type = if next_raw.is_null() {
                "null"
            } else if next_raw.is_string() {
                "string"
            } else {
                "other"
            };
            debug!(
                "Todoist page {}: {} items, next_cursor type={}, next_cursor raw={}, total so far={}",
                page_num, page_count, next_type, next_raw, all_items.len()
            );

            match next_raw.as_str() {
                Some(c) if !c.is_empty() && c != "null" => cursor = Some(c.to_string()),
                _ => break,
            }
        }

        Ok(all_items)
    }

    async fn list_tasks(&self, project_id: Option<&str>, filter: Option<&str>) -> Result<String> {
        // v1 API: /tasks for listing, /tasks/filter for filter queries
        // limit max=200, default=50
        let tasks = if let Some(f) = filter {
            let query = vec![("query", f), ("limit", "200")];
            self.paginated_get(&format!("{}/tasks/filter", TODOIST_API), &query)
                .await?
        } else {
            let mut query: Vec<(&str, &str)> = vec![("limit", "200")];
            if let Some(pid) = project_id {
                query.push(("project_id", pid));
            }
            self.paginated_get(&format!("{}/tasks", TODOIST_API), &query)
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
            .post(format!("{}/tasks", TODOIST_API))
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
                &text[..text.len().min(200)]
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
            .post(format!("{}/tasks/{}/close", TODOIST_API, task_id))
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
            .paginated_get(&format!("{}/projects", TODOIST_API), &[("limit", "200")])
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
}

#[async_trait]
impl Tool for TodoistTool {
    fn name(&self) -> &str {
        "todoist"
    }

    fn description(&self) -> &str {
        "Manage Todoist tasks and projects. Actions: list_tasks, create_task, complete_task, list_projects."
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
                    "enum": ["list_tasks", "create_task", "complete_task", "list_projects"],
                    "description": "Action to perform"
                },
                "content": {
                    "type": "string",
                    "description": "Task content/title (for create_task)"
                },
                "description": {
                    "type": "string",
                    "description": "Task description (for create_task)"
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
                    "description": "Task ID (for complete_task)"
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
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        let result = match action {
            "list_tasks" => {
                self.list_tasks(params["project_id"].as_str(), params["filter"].as_str())
                    .await
            }
            "create_task" => {
                let content = match params["content"].as_str() {
                    Some(c) => c,
                    None => {
                        return Ok(ToolResult::error("Missing 'content' parameter".to_string()))
                    }
                };
                let labels: Option<Vec<&str>> = params["labels"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect());
                self.create_task(
                    content,
                    params["description"].as_str(),
                    params["project_id"].as_str(),
                    params["due_string"].as_str(),
                    params["priority"].as_u64(),
                    labels,
                )
                .await
            }
            "complete_task" => {
                let task_id = match params["task_id"].as_str() {
                    Some(id) => id,
                    None => {
                        return Ok(ToolResult::error("Missing 'task_id' parameter".to_string()))
                    }
                };
                self.complete_task(task_id).await
            }
            "list_projects" => self.list_projects().await,
            _ => return Ok(ToolResult::error(format!("Unknown action: {}", action))),
        };

        match result {
            Ok(content) => Ok(ToolResult::new(content)),
            Err(e) => Ok(ToolResult::error(format!("Todoist error: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> TodoistTool {
        TodoistTool::new("fake_token".to_string())
    }

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
    async fn test_create_task_missing_content() {
        let result = tool()
            .execute(serde_json::json!({"action": "create_task"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("content"));
    }

    #[tokio::test]
    async fn test_complete_task_missing_id() {
        let result = tool()
            .execute(serde_json::json!({"action": "complete_task"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("task_id"));
    }

    #[test]
    fn test_api_base_url() {
        assert_eq!(TODOIST_API, "https://api.todoist.com/api/v1");
    }

    #[test]
    fn test_task_format_with_string_id() {
        // v1 API returns string IDs like "6fxQ8VwjqXf5gPcC"
        let task = serde_json::json!({
            "id": "6fxQ8VwjqXf5gPcC",
            "content": "Buy milk",
            "priority": 3,
            "due": {"string": "tomorrow"},
            "labels": ["groceries", "urgent"]
        });
        let id = task["id"].as_str().unwrap();
        assert_eq!(id, "6fxQ8VwjqXf5gPcC");
        let labels: Vec<&str> = task["labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|l| l.as_str())
            .collect();
        assert_eq!(labels, vec!["groceries", "urgent"]);
    }

    #[test]
    fn test_paginated_response_parsing() {
        // v1 API wraps results in {"results": [...], "next_cursor": ...}
        let response = serde_json::json!({
            "results": [
                {"id": "abc123", "content": "Task 1"},
                {"id": "def456", "content": "Task 2"}
            ],
            "next_cursor": "cursor_xyz"
        });
        let results = response["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["id"].as_str().unwrap(), "abc123");

        let cursor = response["next_cursor"].as_str().unwrap();
        assert_eq!(cursor, "cursor_xyz");
    }

    #[test]
    fn test_paginated_response_no_cursor() {
        // Last page has no next_cursor
        let response = serde_json::json!({
            "results": [{"id": "abc", "content": "Last task"}],
            "next_cursor": null
        });
        assert!(response["next_cursor"].as_str().is_none());
    }

    #[test]
    fn test_project_format_with_is_favorite() {
        // v1 API uses is_favorite (not favorite)
        let project = serde_json::json!({
            "id": "proj123",
            "name": "Shopping",
            "color": "berry_red",
            "is_favorite": true
        });
        assert!(project["is_favorite"].as_bool().unwrap());
        assert_eq!(project["color"].as_str().unwrap(), "berry_red");
    }
}
