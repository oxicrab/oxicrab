use crate::actions;
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory};
use crate::agent::tools::google_common::GoogleApiClient;
use crate::agent::tools::{Tool, ToolResult};
use crate::auth::google::GoogleCredentials;
use crate::require_param;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

pub struct GoogleTasksTool {
    api: GoogleApiClient,
}

impl GoogleTasksTool {
    pub fn new(credentials: GoogleCredentials) -> Self {
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

        match action {
            "list_task_lists" => {
                let result = self.api.call("users/@me/lists", "GET", None).await?;
                let empty_vec: Vec<Value> = vec![];
                let lists = result["items"].as_array().unwrap_or(&empty_vec);

                if lists.is_empty() {
                    return Ok(ToolResult::new("No task lists found.".to_string()));
                }

                let mut lines = vec![format!("Found {} task list(s):\n", lists.len())];
                for list in lists {
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

                let mut endpoint = format!(
                    "lists/{}/tasks?maxResults={}",
                    urlencoding::encode(list_id),
                    max_results
                );
                if !show_completed {
                    endpoint.push_str("&showCompleted=false&showHidden=false");
                }

                let result = self.api.call(&endpoint, "GET", None).await?;
                let empty_vec: Vec<Value> = vec![];
                let tasks = result["items"].as_array().unwrap_or(&empty_vec);

                if tasks.is_empty() {
                    return Ok(ToolResult::new("No tasks found.".to_string()));
                }

                let mut lines = vec![format!("Found {} task(s):\n", tasks.len())];
                for task in tasks {
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
                let buttons = build_google_task_buttons(tasks, list_id);
                Ok(with_buttons(ToolResult::new(lines.join("\n")), buttons))
            }
            "get_task" => {
                let task_id = require_param!(params, "task_id");

                let endpoint = format!(
                    "lists/{}/tasks/{}",
                    urlencoding::encode(list_id),
                    urlencoding::encode(task_id),
                );
                let task = self.api.call(&endpoint, "GET", None).await?;
                let buttons = build_google_task_buttons(std::slice::from_ref(&task), list_id);
                Ok(with_buttons(
                    ToolResult::new(format_task_detail(&task)),
                    buttons,
                ))
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
                Ok(ToolResult::new(format!(
                    "Task updated: {} (ID: {})",
                    task["title"].as_str().unwrap_or("?"),
                    task["id"].as_str().unwrap_or("?"),
                )))
            }
            "delete_task" => {
                let task_id = require_param!(params, "task_id");

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
                "task_id": task_id,
                "tasklist_id": tasklist_id,
                "action": "complete"
            }).to_string()
        }));
    }
    buttons
}

/// Attach suggested buttons metadata to a `ToolResult` if there are any buttons.
fn with_buttons(result: ToolResult, buttons: Vec<Value>) -> ToolResult {
    if buttons.is_empty() {
        result
    } else {
        result.with_metadata(HashMap::from([(
            "suggested_buttons".to_string(),
            Value::Array(buttons),
        )]))
    }
}

#[cfg(test)]
mod tests;
