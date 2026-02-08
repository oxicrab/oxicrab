use crate::agent::tools::{Tool, ToolResult};
use crate::cron::service::CronService;
use crate::cron::types::{CronJob, CronJobState, CronPayload, CronSchedule};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct CronTool {
    cron_service: Arc<CronService>,
    channel: Arc<tokio::sync::Mutex<String>>,
    chat_id: Arc<tokio::sync::Mutex<String>>,
}

impl CronTool {
    pub fn new(cron_service: Arc<CronService>) -> Self {
        Self {
            cron_service,
            channel: Arc::new(tokio::sync::Mutex::new(String::new())),
            chat_id: Arc::new(tokio::sync::Mutex::new(String::new())),
        }
    }
}

#[async_trait]
impl Tool for CronTool {
    fn name(&self) -> &str {
        "cron"
    }

    fn description(&self) -> &str {
        "Schedule reminders and recurring tasks. Actions: add, list, remove. Jobs are automatically delivered to the current conversation."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "remove"],
                    "description": "Action to perform"
                },
                "message": {
                    "type": "string",
                    "description": "Reminder message (for add)"
                },
                "every_seconds": {
                    "type": "integer",
                    "description": "Interval in seconds (for recurring tasks)"
                },
                "cron_expr": {
                    "type": "string",
                    "description": "Cron expression like '0 9 * * *' (for scheduled tasks)"
                },
                "job_id": {
                    "type": "string",
                    "description": "Job ID (for remove)"
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
            "add" => {
                let message = params["message"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'message' parameter for add"))?
                    .to_string();

                let channel = self.channel.lock().await.clone();
                let chat_id = self.chat_id.lock().await.clone();

                if channel.is_empty() || chat_id.is_empty() {
                    return Ok(ToolResult::error(
                        "Error: no session context (channel/chat_id)".to_string(),
                    ));
                }

                let schedule = if let Some(every_secs) = params["every_seconds"].as_u64() {
                    CronSchedule::Every {
                        every_ms: Some((every_secs * 1000) as i64),
                    }
                } else if let Some(cron_expr) = params["cron_expr"].as_str() {
                    CronSchedule::Cron {
                        expr: Some(cron_expr.to_string()),
                        tz: None,
                    }
                } else {
                    return Ok(ToolResult::error(
                        "Error: either every_seconds or cron_expr is required".to_string(),
                    ));
                };

                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .context("System time is before UNIX epoch")
                    .map(|d| d.as_millis() as i64)?;

                let job = CronJob {
                    id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
                    name: {
                        let truncated: String = message.chars().take(30).collect();
                        if truncated.len() < message.len() {
                            format!("{}...", truncated)
                        } else {
                            message.clone()
                        }
                    },
                    enabled: true,
                    schedule,
                    payload: CronPayload {
                        kind: "agent_turn".to_string(),
                        message,
                        deliver: true,
                        channel: Some(channel),
                        to: Some(chat_id),
                    },
                    state: CronJobState {
                        next_run_at_ms: None,
                        last_run_at_ms: None,
                        last_status: None,
                        last_error: None,
                    },
                    created_at_ms: now_ms,
                    updated_at_ms: now_ms,
                    delete_after_run: false,
                };

                self.cron_service.add_job(job.clone()).await?;
                Ok(ToolResult::new(format!(
                    "Created job '{}' (id: {})",
                    job.name, job.id
                )))
            }
            "list" => {
                let jobs = self.cron_service.list_jobs(false).await?;
                if jobs.is_empty() {
                    return Ok(ToolResult::new("No scheduled jobs.".to_string()));
                }
                let lines: Vec<String> = jobs
                    .iter()
                    .map(|j| {
                        let schedule_desc = match &j.schedule {
                            CronSchedule::At { at_ms } => {
                                at_ms.and_then(|ms| {
                                    chrono::DateTime::from_timestamp(ms / 1000, 0)
                                        .map(|dt| format!("once at {}", dt.format("%Y-%m-%d %H:%M UTC")))
                                }).unwrap_or_else(|| "once (no time set)".to_string())
                            }
                            CronSchedule::Every { every_ms } => {
                                every_ms.map(|ms| {
                                    let secs = ms / 1000;
                                    if secs >= 86400 { format!("every {}d", secs / 86400) }
                                    else if secs >= 3600 { format!("every {}h", secs / 3600) }
                                    else if secs >= 60 { format!("every {}m", secs / 60) }
                                    else { format!("every {}s", secs) }
                                }).unwrap_or_else(|| "recurring (no interval set)".to_string())
                            }
                            CronSchedule::Cron { expr, tz } => {
                                let tz_str = tz.as_deref().unwrap_or("UTC");
                                expr.as_deref()
                                    .map(|e| format!("cron '{}' ({})", e, tz_str))
                                    .unwrap_or_else(|| "cron (no expression)".to_string())
                            }
                        };
                        let next_run = j.state.next_run_at_ms.and_then(|ms| {
                            chrono::DateTime::from_timestamp(ms / 1000, 0)
                                .map(|dt| format!("next: {}", dt.format("%Y-%m-%d %H:%M UTC")))
                        }).unwrap_or_else(|| "next: pending".to_string());
                        format!(
                            "- [{}] {} | schedule: {} | {} | message: \"{}\"",
                            j.id, j.name, schedule_desc, next_run, j.payload.message
                        )
                    })
                    .collect();
                Ok(ToolResult::new(format!(
                    "Scheduled jobs:\n{}",
                    lines.join("\n")
                )))
            }
            "remove" => {
                let job_id = params["job_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'job_id' parameter for remove"))?;

                match self.cron_service.remove_job(job_id).await? {
                    Some(_) => Ok(ToolResult::new(format!("Removed job {}", job_id))),
                    None => Ok(ToolResult::error(format!("Job {} not found", job_id))),
                }
            }
            _ => Ok(ToolResult::error(format!("Unknown action: {}", action))),
        }
    }

    async fn set_context(&self, channel: &str, chat_id: &str) {
        *self.channel.lock().await = channel.to_string();
        *self.chat_id.lock().await = chat_id.to_string();
    }
}
