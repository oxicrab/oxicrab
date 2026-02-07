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
                    name: if message.len() > 30 {
                        format!("{}...", &message[..30])
                    } else {
                        message.clone()
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
                        format!(
                            "- {} (id: {}, {})",
                            j.name,
                            j.id,
                            match &j.schedule {
                                CronSchedule::At { .. } => "at",
                                CronSchedule::Every { .. } => "every",
                                CronSchedule::Cron { .. } => "cron",
                            }
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
