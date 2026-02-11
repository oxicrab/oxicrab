use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

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

    async fn list_tasks(&self, project_id: Option<&str>, filter: Option<&str>) -> Result<String> {
        let mut query: Vec<(&str, &str)> = Vec::new();
        if let Some(pid) = project_id {
            query.push(("project_id", pid));
        }
        if let Some(f) = filter {
            query.push(("filter", f));
        }

        let resp = self
            .client
            .get(format!("{}/tasks", TODOIST_API))
            .query(&query)
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

        let tasks = body.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
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
                let priority_str = match priority {
                    4 => " !!!",
                    3 => " !!",
                    2 => " !",
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
        let url = body["url"].as_str().unwrap_or("");
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
        let resp = self
            .client
            .get(format!("{}/projects", TODOIST_API))
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

        let projects = body.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
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
                    "description": "Task priority (1=normal, 4=urgent)"
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
}
