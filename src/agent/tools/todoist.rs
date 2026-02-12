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

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    /// Fetch all pages from a paginated v1 endpoint.
    async fn paginated_get(&self, url: &str, base_query: &[(&str, &str)]) -> Result<Vec<Value>> {
        let mut all_items: Vec<Value> = Vec::new();
        let mut cursor: Option<String> = None;
        const MAX_PAGES: usize = 10; // Safety limit

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
            .post(format!("{}/tasks/{}/close", self.base_url, task_id))
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
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn tool() -> TodoistTool {
        TodoistTool::new("fake_token".to_string())
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

    // --- Wiremock tests ---

    #[tokio::test]
    async fn test_list_tasks_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/tasks"))
            .and(header("Authorization", "Bearer test_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {
                        "id": "abc123",
                        "content": "Buy groceries",
                        "priority": 1,
                        "due": {"string": "today"},
                        "labels": ["shopping"]
                    },
                    {
                        "id": "def456",
                        "content": "Write tests",
                        "priority": 4,
                        "due": null,
                        "labels": []
                    }
                ],
                "next_cursor": null
            })))
            .mount(&server)
            .await;

        let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"action": "list_tasks"}))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("Buy groceries"));
        assert!(result.content.contains("!!!"));
        assert!(result.content.contains("Write tests"));
        assert!(result.content.contains("[shopping]"));
        assert!(result.content.contains("Tasks (2)"));
    }

    #[tokio::test]
    async fn test_list_tasks_with_filter() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/tasks/filter"))
            .and(query_param("query", "today"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {
                        "id": "t1",
                        "content": "Morning standup",
                        "priority": 2,
                        "due": {"string": "today"},
                        "labels": ["work"]
                    }
                ],
                "next_cursor": null
            })))
            .mount(&server)
            .await;

        let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"action": "list_tasks", "filter": "today"}))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("Morning standup"));
        assert!(result.content.contains("!!"));
    }

    #[tokio::test]
    async fn test_list_tasks_with_project_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/tasks"))
            .and(query_param("project_id", "proj_abc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {"id": "t1", "content": "Project task", "priority": 3, "due": null, "labels": []}
                ],
                "next_cursor": null
            })))
            .mount(&server)
            .await;

        let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"action": "list_tasks", "project_id": "proj_abc"}))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("Project task"));
    }

    #[tokio::test]
    async fn test_list_tasks_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/tasks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [],
                "next_cursor": null
            })))
            .mount(&server)
            .await;

        let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"action": "list_tasks"}))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("No tasks found"));
    }

    #[tokio::test]
    async fn test_list_tasks_paginated() {
        let server = MockServer::start().await;
        // Page 1
        Mock::given(method("GET"))
            .and(path("/tasks"))
            .and(wiremock::matchers::query_param_is_missing("cursor"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {"id": "t1", "content": "Task 1", "priority": 4, "due": null, "labels": []}
                ],
                "next_cursor": "page2cursor"
            })))
            .mount(&server)
            .await;
        // Page 2
        Mock::given(method("GET"))
            .and(path("/tasks"))
            .and(query_param("cursor", "page2cursor"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {"id": "t2", "content": "Task 2", "priority": 4, "due": null, "labels": []}
                ],
                "next_cursor": null
            })))
            .mount(&server)
            .await;

        let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"action": "list_tasks"}))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("Task 1"));
        assert!(result.content.contains("Task 2"));
        assert!(result.content.contains("Tasks (2)"));
    }

    #[tokio::test]
    async fn test_create_task_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tasks"))
            .and(header("Authorization", "Bearer test_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "new_task_123",
                "content": "Write documentation",
                "priority": 2
            })))
            .mount(&server)
            .await;

        let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({
                "action": "create_task",
                "content": "Write documentation",
                "priority": 2,
                "due_string": "tomorrow",
                "labels": ["docs"]
            }))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("Created task"));
        assert!(result.content.contains("new_task_123"));
        assert!(result.content.contains("Write documentation"));
    }

    #[tokio::test]
    async fn test_complete_task_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tasks/task_xyz/close"))
            .and(header("Authorization", "Bearer test_token"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"action": "complete_task", "task_id": "task_xyz"}))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("task_xyz"));
        assert!(result.content.contains("completed"));
    }

    #[tokio::test]
    async fn test_list_projects_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/projects"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {"id": "p1", "name": "Inbox", "color": "grey", "is_favorite": false},
                    {"id": "p2", "name": "Work", "color": "blue", "is_favorite": true}
                ],
                "next_cursor": null
            })))
            .mount(&server)
            .await;

        let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"action": "list_projects"}))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("Inbox"));
        assert!(result.content.contains("Work"));
        assert!(result.content.contains(" *")); // favorite marker
        assert!(result.content.contains("Projects (2)"));
    }

    #[tokio::test]
    async fn test_list_projects_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/projects"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [],
                "next_cursor": null
            })))
            .mount(&server)
            .await;

        let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"action": "list_projects"}))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("No projects found"));
    }

    #[tokio::test]
    async fn test_api_error_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/tasks"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .mount(&server)
            .await;

        let tool = TodoistTool::with_base_url("bad_token".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"action": "list_tasks"}))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("401"));
    }

    #[tokio::test]
    async fn test_api_error_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tasks"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&server)
            .await;

        let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"action": "create_task", "content": "test"}))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("500"));
    }

    #[tokio::test]
    async fn test_complete_task_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tasks/nonexistent/close"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Task not found"))
            .mount(&server)
            .await;

        let tool = TodoistTool::with_base_url("test_token".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"action": "complete_task", "task_id": "nonexistent"}))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("404"));
    }
}
