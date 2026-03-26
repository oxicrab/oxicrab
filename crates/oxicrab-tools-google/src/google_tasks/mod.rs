use crate::credentials::GoogleCredentials;
use crate::google_common::GoogleApiClient;
use anyhow::Result;
use async_trait::async_trait;
use oxicrab_core::actions;
use oxicrab_core::require_param;
use oxicrab_core::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory};
use oxicrab_core::tools::base::{Tool, ToolResult};
use oxicrab_core::utils::url_params::validate_url_segment;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct GoogleTasksTool {
    api: GoogleApiClient,
}

impl GoogleTasksTool {
    pub fn new(credentials: Arc<Mutex<GoogleCredentials>>) -> Self {
        Self {
            api: GoogleApiClient::new(credentials, "https://tasks.googleapis.com/tasks/v1"),
        }
    }
}

#[async_trait]
impl Tool for GoogleTasksTool {
    fn name(&self) -> &'static str {
        "google_tasks"
    }

    fn description(&self) -> &'static str {
        "Interact with Google Tasks. Actions: list_task_lists, list_tasks, get_task, create_task, update_task, delete_task. Tip: after listing tasks, use add_buttons to offer Complete or Delete actions."
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: actions![
                list_task_lists: ro,
                list_tasks: ro,
                get_task: ro,
                create_task,
                update_task,
                delete_task,
            ],
            category: ToolCategory::Productivity,
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_task_lists", "list_tasks", "get_task", "create_task", "update_task", "delete_task"],
                    "description": "Action to perform. 'list_task_lists' shows available \
                     task lists and their IDs. 'list_tasks' shows tasks in a list (use \
                     show_completed to include done tasks)."
                },
                "task_list_id": {
                    "type": "string",
                    "description": "Task list ID (default: '@default' for the user's primary list)"
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID (for get/update/delete)"
                },
                "title": {
                    "type": "string",
                    "description": "Task title (for create/update)"
                },
                "notes": {
                    "type": "string",
                    "description": "Task notes/description (for create/update)"
                },
                "due": {
                    "type": "string",
                    "description": "Due date in ISO 8601 format, e.g. '2026-03-10T00:00:00Z' (for create/update)"
                },
                "status": {
                    "type": "string",
                    "enum": ["needsAction", "completed"],
                    "description": "Task status (for update). 'needsAction' or 'completed'"
                },
                "show_completed": {
                    "type": "boolean",
                    "description": "Include completed tasks in list_tasks (default: true)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Max tasks to return (for list_tasks, default 20)",
                    "minimum": 1,
                    "maximum": 100
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = require_param!(params, "action");

        let list_id = params["task_list_id"].as_str().unwrap_or("@default");
        if let Err(e) = validate_url_segment(list_id, "task_list_id") {
            return Ok(ToolResult::error(e));
        }

        match action {
            "list_task_lists" => {
                let mut all_lists: Vec<Value> = Vec::new();
                let mut page_token: Option<String> = None;
                let max_pages = 3;

                for _ in 0..max_pages {
                    let mut endpoint = "users/@me/lists?maxResults=100".to_string();
                    if let Some(ref token) = page_token {
                        endpoint.push_str(&format!("&pageToken={}", urlencoding::encode(token)));
                    }

                    let result = self.api.call(&endpoint, "GET", None).await?;
                    let empty_vec: Vec<Value> = vec![];
                    let page_lists = result["items"].as_array().unwrap_or(&empty_vec);
                    all_lists.extend(page_lists.iter().cloned());

                    match result["nextPageToken"].as_str() {
                        Some(t) if !t.is_empty() => page_token = Some(t.to_string()),
                        _ => break,
                    }
                }

                if all_lists.is_empty() {
                    return Ok(ToolResult::new("No task lists found.".to_string()));
                }

                let mut lines = vec![format!("Found {} task list(s):\n", all_lists.len())];
                for list in &all_lists {
                    lines.push(format!(
                        "- {}\n  ID: {}",
                        list["title"].as_str().unwrap_or("(untitled)"),
                        list["id"].as_str().unwrap_or("?"),
                    ));
                }
                Ok(ToolResult::new(lines.join("\n")))
            }
            "list_tasks" => {
                let max_results = params["max_results"].as_u64().unwrap_or(20).min(100) as u32;
                let show_completed = params["show_completed"].as_bool().unwrap_or(true);

                let mut all_tasks: Vec<Value> = Vec::new();
                let mut page_token: Option<String> = None;
                let max_pages = 3;
                let cap: usize = 200;

                for _ in 0..max_pages {
                    let mut endpoint = format!(
                        "lists/{}/tasks?maxResults={}",
                        urlencoding::encode(list_id),
                        max_results
                    );
                    if !show_completed {
                        endpoint.push_str("&showCompleted=false&showHidden=false");
                    }
                    if let Some(ref token) = page_token {
                        endpoint.push_str(&format!("&pageToken={}", urlencoding::encode(token)));
                    }

                    let result = self.api.call(&endpoint, "GET", None).await?;
                    let empty_vec: Vec<Value> = vec![];
                    let page_tasks = result["items"].as_array().unwrap_or(&empty_vec);
                    all_tasks.extend(page_tasks.iter().cloned());

                    if all_tasks.len() >= cap {
                        all_tasks.truncate(cap);
                        break;
                    }

                    match result["nextPageToken"].as_str() {
                        Some(t) if !t.is_empty() => page_token = Some(t.to_string()),
                        _ => break,
                    }
                }

                if all_tasks.is_empty() {
                    return Ok(ToolResult::new("No tasks found.".to_string()));
                }

                let mut lines = vec![format!("Found {} task(s):\n", all_tasks.len())];
                for task in &all_tasks {
                    let title = task["title"].as_str().unwrap_or("(untitled)");
                    let status = task["status"].as_str().unwrap_or("?");
                    let status_icon = if status == "completed" { "[x]" } else { "[ ]" };
                    let due_str = task["due"]
                        .as_str()
                        .map(|d| format!("\n  Due: {}", &d[..10.min(d.len())]))
                        .unwrap_or_default();

                    lines.push(format!(
                        "- {} {}\n  ID: {}\n  Status: {}{}",
                        status_icon,
                        title,
                        task["id"].as_str().unwrap_or("?"),
                        status,
                        due_str,
                    ));
                }
                let buttons = build_google_task_buttons(&all_tasks, list_id);
                Ok(ToolResult::new(lines.join("\n")).with_buttons(buttons))
            }
            "get_task" => {
                let task_id = require_param!(params, "task_id");
                if let Err(e) = validate_url_segment(task_id, "task_id") {
                    return Ok(ToolResult::error(e));
                }

                let endpoint = format!(
                    "lists/{}/tasks/{}",
                    urlencoding::encode(list_id),
                    urlencoding::encode(task_id),
                );
                let task = self.api.call(&endpoint, "GET", None).await?;
                let buttons = build_google_task_buttons(std::slice::from_ref(&task), list_id);
                Ok(ToolResult::new(format_task_detail(&task)).with_buttons(buttons))
            }
            "create_task" => {
                let title = require_param!(params, "title");

                let mut body = serde_json::json!({"title": title});

                if let Some(notes) = params["notes"].as_str() {
                    body["notes"] = Value::String(notes.to_string());
                }
                if let Some(due) = params["due"].as_str() {
                    body["due"] = Value::String(due.to_string());
                }

                let endpoint = format!("lists/{}/tasks", urlencoding::encode(list_id));
                let task = self.api.call(&endpoint, "POST", Some(body)).await?;
                Ok(ToolResult::new(format!(
                    "Task created: {} (ID: {})",
                    task["title"].as_str().unwrap_or("?"),
                    task["id"].as_str().unwrap_or("?"),
                )))
            }
            "update_task" => {
                let task_id = require_param!(params, "task_id");
                if let Err(e) = validate_url_segment(task_id, "task_id") {
                    return Ok(ToolResult::error(e));
                }

                let mut body = serde_json::json!({});

                if let Some(title) = params["title"].as_str() {
                    body["title"] = Value::String(title.to_string());
                }
                if let Some(notes) = params["notes"].as_str() {
                    body["notes"] = Value::String(notes.to_string());
                }
                if let Some(due) = params["due"].as_str() {
                    body["due"] = Value::String(due.to_string());
                }
                if let Some(status) = params["status"].as_str() {
                    body["status"] = Value::String(status.to_string());
                }

                if body.as_object().is_none_or(serde_json::Map::is_empty) {
                    return Ok(ToolResult::error(
                        "update_task requires at least one field: title, notes, due, or status"
                            .to_string(),
                    ));
                }

                let endpoint = format!(
                    "lists/{}/tasks/{}",
                    urlencoding::encode(list_id),
                    urlencoding::encode(task_id),
                );
                let task = self.api.call(&endpoint, "PATCH", Some(body)).await?;
                let result = ToolResult::new(format!(
                    "Task updated: {} (ID: {})",
                    task["title"].as_str().unwrap_or("?"),
                    task["id"].as_str().unwrap_or("?"),
                ));
                // After completing a task, offer to view remaining tasks
                if params["status"].as_str() == Some("completed") {
                    let view_remaining = serde_json::json!({
                        "id": "view-remaining-tasks",
                        "label": "View remaining tasks",
                        "style": "primary",
                        "context": serde_json::json!({
                            "tool": "google_tasks",
                            "params": {"action": "list_tasks", "task_list_id": list_id, "show_completed": false}
                        }).to_string()
                    });
                    Ok(result.with_buttons(vec![view_remaining]))
                } else {
                    Ok(result)
                }
            }
            "delete_task" => {
                let task_id = require_param!(params, "task_id");
                if let Err(e) = validate_url_segment(task_id, "task_id") {
                    return Ok(ToolResult::error(e));
                }

                let endpoint = format!(
                    "lists/{}/tasks/{}",
                    urlencoding::encode(list_id),
                    urlencoding::encode(task_id),
                );
                self.api.call(&endpoint, "DELETE", None).await?;
                Ok(ToolResult::new(format!("Task {task_id} deleted.")))
            }
            _ => Ok(ToolResult::error(format!("unknown action: {action}"))),
        }
    }
}

fn format_task_detail(task: &Value) -> String {
    let mut parts = vec![
        format!("Title: {}", task["title"].as_str().unwrap_or("(untitled)")),
        format!("ID: {}", task["id"].as_str().unwrap_or("?")),
        format!("Status: {}", task["status"].as_str().unwrap_or("?")),
    ];
    if let Some(notes) = task["notes"].as_str() {
        parts.push(format!("Notes: {notes}"));
    }
    if let Some(due) = task["due"].as_str() {
        parts.push(format!("Due: {}", &due[..10.min(due.len())]));
    }
    if let Some(completed) = task["completed"].as_str() {
        parts.push(format!(
            "Completed: {}",
            &completed[..10.min(completed.len())]
        ));
    }
    if let Some(updated) = task["updated"].as_str() {
        parts.push(format!("Updated: {updated}"));
    }
    if let Some(parent) = task["parent"].as_str() {
        parts.push(format!("Parent: {parent}"));
    }
    parts.join("\n")
}

/// Build suggested "Complete" buttons for incomplete Google Tasks (max 5).
fn build_google_task_buttons(tasks: &[Value], tasklist_id: &str) -> Vec<Value> {
    let mut buttons = Vec::new();
    for task in tasks {
        if buttons.len() >= 5 {
            break;
        }
        let status = task["status"].as_str().unwrap_or("needsAction");
        if status == "completed" {
            continue;
        }
        let task_id = task["id"].as_str().unwrap_or_default();
        let title = task["title"].as_str().unwrap_or("task");
        if task_id.is_empty() {
            continue;
        }
        // UTF-8 safe truncation for button labels
        let label = {
            let truncated: String = title.chars().take(25).collect();
            if truncated.len() < title.len() {
                format!(
                    "Complete: {}...",
                    title.chars().take(22).collect::<String>()
                )
            } else {
                format!("Complete: {title}")
            }
        };
        buttons.push(serde_json::json!({
            "id": format!("complete-{task_id}"),
            "label": label,
            "style": "primary",
            "context": serde_json::json!({
                "tool": "google_tasks",
                "params": {
                    "action": "update_task",
                    "task_id": task_id,
                    "task_list_id": tasklist_id,
                    "status": "completed"
                }
            }).to_string()
        }));
    }
    buttons
}

#[cfg(test)]
mod tests;
