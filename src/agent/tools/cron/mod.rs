use crate::actions;
use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult};
use crate::config::ChannelsConfig;
use crate::cron::service::CronService;
use crate::cron::types::{CronJob, CronJobState, CronPayload, CronSchedule, CronTarget};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct CronTool {
    cron_service: Arc<CronService>,
    channels_config: Option<ChannelsConfig>,
    memory_db: Option<Arc<MemoryDB>>,
}

impl CronTool {
    pub fn new(
        cron_service: Arc<CronService>,
        channels_config: Option<ChannelsConfig>,
        memory_db: Option<Arc<MemoryDB>>,
    ) -> Self {
        Self {
            cron_service,
            channels_config,
            memory_db,
        }
    }

    fn resolve_targets(
        &self,
        channels_param: Option<&Vec<Value>>,
        current_channel: &str,
        current_chat_id: &str,
    ) -> Vec<CronTarget> {
        match channels_param {
            None => {
                // No channels param → current channel only
                vec![CronTarget {
                    channel: current_channel.to_string(),
                    to: current_chat_id.to_string(),
                }]
            }
            Some(channels) => {
                let channel_names: Vec<String> = channels
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_lowercase))
                    .collect();

                if channel_names.iter().any(|c| c == "all") {
                    self.resolve_all_channel_targets()
                } else {
                    self.resolve_specific_channel_targets(&channel_names)
                }
            }
        }
    }

    fn resolve_all_channel_targets(&self) -> Vec<CronTarget> {
        resolve_all_channel_targets_from_config(self.channels_config.as_ref())
    }

    /// Parse schedule parameters into a `CronSchedule`.
    ///
    /// Validates `every_seconds`, `cron_expr`, `at_time`, or `event_pattern` from
    /// the tool params and returns the appropriate schedule variant. Returns
    /// `Err(ToolResult)` for user-facing validation errors.
    fn parse_schedule(params: &Value) -> std::result::Result<CronSchedule, ToolResult> {
        if let Some(every_secs) = params["every_seconds"].as_u64() {
            if every_secs == 0 || every_secs > 31_536_000 {
                return Err(ToolResult::error(
                    "every_seconds must be between 1 and 31536000 (1 year)".to_string(),
                ));
            }
            Ok(CronSchedule::Every {
                every_ms: Some((every_secs * 1000) as i64),
            })
        } else if let Some(cron_expr) = params["cron_expr"].as_str() {
            if let Err(e) = crate::cron::service::validate_cron_expr(cron_expr) {
                return Err(ToolResult::error(format!("invalid cron expression: {}", e)));
            }
            let tz = params["tz"]
                .as_str()
                .map(std::string::ToString::to_string)
                .or_else(crate::cron::service::detect_system_timezone);
            Ok(CronSchedule::Cron {
                expr: Some(cron_expr.to_string()),
                tz,
            })
        } else if let Some(at_time_str) = params["at_time"].as_str() {
            let dt = chrono::DateTime::parse_from_rfc3339(at_time_str)
                .or_else(|_| chrono::DateTime::parse_from_str(at_time_str, "%Y-%m-%dT%H:%M:%S%z"))
                .map_err(|_| {
                    ToolResult::error(
                        "Invalid at_time format. Use ISO 8601 (e.g. '2025-01-15T09:00:00-05:00')"
                            .to_string(),
                    )
                })?;
            let at_ms = dt.timestamp_millis();
            let now_ms_check = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64);
            if at_ms <= now_ms_check {
                return Err(ToolResult::error(
                    "at_time must be in the future".to_string(),
                ));
            }
            Ok(CronSchedule::At { at_ms: Some(at_ms) })
        } else if let Some(event_pattern) = params["event_pattern"].as_str() {
            if let Err(e) = regex::Regex::new(event_pattern) {
                return Err(ToolResult::error(format!(
                    "invalid event_pattern regex: {}",
                    e
                )));
            }
            Ok(CronSchedule::Event {
                pattern: Some(event_pattern.to_string()),
                channel: params["event_channel"]
                    .as_str()
                    .map(std::string::ToString::to_string),
            })
        } else {
            Err(ToolResult::error(
                "either every_seconds, cron_expr, at_time, or event_pattern is required"
                    .to_string(),
            ))
        }
    }

    fn resolve_specific_channel_targets(&self, channel_names: &[String]) -> Vec<CronTarget> {
        let Some(ref cfg) = self.channels_config else {
            return vec![];
        };

        let mut targets = Vec::new();
        for name in channel_names {
            match name.as_str() {
                "slack" if cfg.slack.enabled => {
                    let to = first_concrete_target(&cfg.slack.allow_from);
                    if !to.is_empty() {
                        targets.push(CronTarget {
                            channel: "slack".to_string(),
                            to,
                        });
                    }
                }
                "discord" if cfg.discord.enabled => {
                    let to = first_concrete_target(&cfg.discord.allow_from);
                    if !to.is_empty() {
                        targets.push(CronTarget {
                            channel: "discord".to_string(),
                            to,
                        });
                    }
                }
                "telegram" if cfg.telegram.enabled => {
                    let to = first_concrete_target(&cfg.telegram.allow_from);
                    if !to.is_empty() {
                        targets.push(CronTarget {
                            channel: "telegram".to_string(),
                            to,
                        });
                    }
                }
                "whatsapp" if cfg.whatsapp.enabled => {
                    let to = first_concrete_target(&cfg.whatsapp.allow_from);
                    if !to.is_empty() {
                        let to = format_whatsapp_target(&to);
                        targets.push(CronTarget {
                            channel: "whatsapp".to_string(),
                            to,
                        });
                    }
                }
                _ => {}
            }
        }
        targets
    }
}

/// Return the first concrete (non-wildcard) target from an allowlist.
fn first_concrete_target(allow_from: &[String]) -> String {
    allow_from
        .iter()
        .find(|s| *s != "*")
        .cloned()
        .unwrap_or_default()
}

/// Format a `WhatsApp` target: append @s.whatsapp.net if not already present.
fn format_whatsapp_target(phone: &str) -> String {
    if phone.contains("@s.whatsapp.net") {
        phone.to_string()
    } else {
        let cleaned = phone.trim_start_matches('+');
        format!("{}@s.whatsapp.net", cleaned)
    }
}

/// Resolve all enabled channel targets from a `ChannelsConfig`.
/// Used by both `CronTool` and CLI commands.
pub fn resolve_all_channel_targets_from_config(cfg: Option<&ChannelsConfig>) -> Vec<CronTarget> {
    let Some(cfg) = cfg else {
        return vec![];
    };

    let mut targets = Vec::new();

    if cfg.slack.enabled {
        let to = first_concrete_target(&cfg.slack.allow_from);
        if !to.is_empty() {
            targets.push(CronTarget {
                channel: "slack".to_string(),
                to,
            });
        }
    }
    if cfg.discord.enabled {
        let to = first_concrete_target(&cfg.discord.allow_from);
        if !to.is_empty() {
            targets.push(CronTarget {
                channel: "discord".to_string(),
                to,
            });
        }
    }
    if cfg.telegram.enabled {
        let to = first_concrete_target(&cfg.telegram.allow_from);
        if !to.is_empty() {
            targets.push(CronTarget {
                channel: "telegram".to_string(),
                to,
            });
        }
    }
    if cfg.whatsapp.enabled {
        let to = first_concrete_target(&cfg.whatsapp.allow_from);
        if !to.is_empty() {
            let to = format_whatsapp_target(&to);
            targets.push(CronTarget {
                channel: "whatsapp".to_string(),
                to,
            });
        }
    }

    targets
}

#[async_trait]
impl Tool for CronTool {
    fn name(&self) -> &'static str {
        "cron"
    }

    fn description(&self) -> &'static str {
        "Schedule recurring or one-shot tasks. Two job types: 'agent' (default) processes the message as a full agent turn with all tools; 'echo' delivers the message directly to channels without invoking the LLM (ideal for simple reminders like 'standup in 5 min'). Schedule with cron_expr, every_seconds, or at_time (one-shot ISO 8601). Optional limits: expires_at (auto-disable after datetime) and max_runs (auto-disable after N executions). Actions: add, list, remove, run, dlq_list, dlq_replay, dlq_clear."
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: actions![
                add,
                list: ro,
                remove,
                run,
                dlq_list: ro,
                dlq_replay,
                dlq_clear,
            ],
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "remove", "run", "dlq_list", "dlq_replay", "dlq_clear"],
                    "description": "Action to perform. dlq_list/dlq_replay/dlq_clear manage the dead letter queue for failed executions."
                },
                "type": {
                    "type": "string",
                    "enum": ["agent", "echo"],
                    "description": "Job type: 'agent' (default) runs a full agent turn with tools; 'echo' delivers the message directly without LLM (saves tokens, good for simple reminders)"
                },
                "message": {
                    "type": "string",
                    "description": "For 'agent' type: instruction/prompt for the agent (e.g. 'fetch my todoist tasks'). For 'echo' type: the exact text to deliver (e.g. 'Standup in 5 minutes!')."
                },
                "every_seconds": {
                    "type": "integer",
                    "description": "Interval in seconds (for recurring tasks)"
                },
                "cron_expr": {
                    "type": "string",
                    "description": "Cron expression like '0 9 * * *' (for scheduled tasks). Standard 5-field format."
                },
                "at_time": {
                    "type": "string",
                    "description": "ISO 8601 datetime for a one-shot job (e.g. '2025-01-15T09:00:00-05:00'). The job runs once at this time and is automatically deleted afterward."
                },
                "tz": {
                    "type": "string",
                    "description": "IANA timezone for cron_expr (e.g. 'America/New_York'). Defaults to system timezone."
                },
                "job_id": {
                    "type": "string",
                    "description": "Job ID (for remove or run)"
                },
                "channels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Target channels: [\"all\"] for all enabled channels, [\"slack\", \"discord\"] for specific ones, or omit for current channel only"
                },
                "expires_at": {
                    "type": "string",
                    "description": "ISO 8601 datetime after which the job auto-disables (e.g. '2025-01-15T17:00:00-05:00'). For recurring jobs that should stop at a certain date/time."
                },
                "max_runs": {
                    "type": "integer",
                    "description": "Maximum number of times the job should run before auto-disabling. E.g. 7 for '7 pings then stop'."
                },
                "event_pattern": {
                    "type": "string",
                    "description": "Regex pattern to trigger the job when an inbound message matches. Mutually exclusive with every_seconds/cron_expr/at_time."
                },
                "event_channel": {
                    "type": "string",
                    "description": "Optional channel filter for event-triggered jobs (only fire for messages from this channel)."
                },
                "cooldown_secs": {
                    "type": "integer",
                    "description": "Minimum seconds between event-triggered firings. Prevents flooding."
                },
                "dlq_id": {
                    "type": "integer",
                    "description": "DLQ entry ID (for dlq_replay)"
                },
                "dlq_status": {
                    "type": "string",
                    "description": "Filter DLQ entries by status (for dlq_list and dlq_clear). E.g. 'pending_retry', 'replayed', 'discarded'."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        match action {
            "add" => {
                let job_type = params["type"].as_str().unwrap_or("agent");
                if job_type != "agent" && job_type != "echo" {
                    return Ok(ToolResult::error(format!(
                        "invalid type '{}'. Must be 'agent' or 'echo'",
                        job_type
                    )));
                }

                let message = params["message"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'message' parameter for add"))?
                    .to_string();

                let channel = ctx.channel.clone();
                let chat_id = ctx.chat_id.clone();

                if channel.is_empty() || chat_id.is_empty() {
                    return Ok(ToolResult::error(
                        "no session context (channel/chat_id)".to_string(),
                    ));
                }

                let channels_param = params["channels"].as_array();
                let targets = self.resolve_targets(channels_param, &channel, &chat_id);

                if targets.is_empty() {
                    return Ok(ToolResult::error(
                        "no valid targets resolved. Check that the specified channels are enabled and have allowFrom configured".to_string(),
                    ));
                }

                let schedule = match Self::parse_schedule(&params) {
                    Ok(s) => s,
                    Err(tool_err) => return Ok(tool_err),
                };

                let delete_after_run = matches!(&schedule, CronSchedule::At { .. });

                // Parse optional expiry
                let expires_at_ms = if let Some(exp_str) = params["expires_at"].as_str() {
                    let dt = chrono::DateTime::parse_from_rfc3339(exp_str)
                        .or_else(|_| {
                            chrono::DateTime::parse_from_str(exp_str, "%Y-%m-%dT%H:%M:%S%z")
                        })
                        .map_err(|_| anyhow::anyhow!("Invalid expires_at format. Use ISO 8601."))?;
                    let ms = dt.timestamp_millis();
                    let now_check = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis() as i64);
                    if ms <= now_check {
                        return Ok(ToolResult::error(
                            "expires_at must be in the future".to_string(),
                        ));
                    }
                    Some(ms)
                } else {
                    None
                };

                let max_runs = params["max_runs"].as_u64().map(|n| n as u32);
                let cooldown_secs = params["cooldown_secs"].as_u64();
                let max_concurrent = params["max_concurrent"].as_u64().map(|n| n as u32);

                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .context("System time is before UNIX epoch")
                    .map(|d| d.as_millis() as i64)?;

                let targets_desc: Vec<String> = targets.iter().map(|t| t.channel.clone()).collect();

                let job = CronJob {
                    id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
                    name: crate::utils::truncate_chars(&message, 30, "..."),
                    enabled: true,
                    schedule,
                    payload: CronPayload {
                        kind: if job_type == "echo" {
                            "echo".to_string()
                        } else {
                            "agent_turn".to_string()
                        },
                        message,
                        agent_echo: job_type == "agent",
                        targets,
                        origin_metadata: ctx.metadata.clone(),
                    },
                    state: CronJobState {
                        next_run_at_ms: None,
                        last_run_at_ms: None,
                        last_status: None,
                        last_error: None,
                        run_count: 0,
                        last_fired_at_ms: None,
                    },
                    created_at_ms: now_ms,
                    updated_at_ms: now_ms,
                    delete_after_run,
                    expires_at_ms,
                    max_runs,
                    cooldown_secs,
                    max_concurrent,
                };

                self.cron_service.add_job(job.clone()).await?;
                Ok(ToolResult::new(format!(
                    "Created job '{}' (id: {}, targets: {})",
                    job.name,
                    job.id,
                    targets_desc.join(", ")
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
                        let schedule_desc = j.schedule.describe();
                        let next_run = j
                            .state
                            .next_run_at_ms
                            .and_then(|ms| {
                                chrono::DateTime::from_timestamp(ms / 1000, 0)
                                    .map(|dt| format!("next: {}", dt.format("%Y-%m-%d %H:%M UTC")))
                            })
                            .unwrap_or_else(|| "next: pending".to_string());
                        let targets_desc: String = if j.payload.targets.is_empty() {
                            "no targets".to_string()
                        } else {
                            j.payload
                                .targets
                                .iter()
                                .map(|t| format!("{}:{}", t.channel, t.to))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        let type_label = if j.payload.kind == "echo" {
                            "echo"
                        } else {
                            "agent"
                        };
                        let mut limits = Vec::new();
                        if let Some(exp) = j.expires_at_ms.and_then(|ms| {
                            chrono::DateTime::from_timestamp(ms / 1000, 0)
                        }) {
                            limits.push(format!(
                                "expires: {}",
                                exp.format("%Y-%m-%d %H:%M UTC")
                            ));
                        }
                        if let Some(max) = j.max_runs {
                            limits.push(format!(
                                "runs: {}/{}",
                                j.state.run_count, max
                            ));
                        }
                        let limits_str = if limits.is_empty() {
                            String::new()
                        } else {
                            format!(" | {}", limits.join(", "))
                        };
                        format!(
                            "- [{}] {} | type: {} | schedule: {} | {} | targets: [{}]{} | message: \"{}\"",
                            j.id, j.name, type_label, schedule_desc, next_run, targets_desc, limits_str, j.payload.message
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
                    None => Ok(ToolResult::error(format!("job {} not found", job_id))),
                }
            }
            "run" => {
                let job_id = params["job_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'job_id' parameter for run"))?
                    .to_string();

                // Spawn job execution on a separate task to avoid deadlock.
                // The agent loop holds `processing_lock` during tool execution,
                // and the cron callback calls `process_direct()` which re-acquires
                // the same lock — awaiting inline would deadlock.
                let cron = self.cron_service.clone();
                let job_id_clone = job_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = cron.run_job(&job_id_clone, true).await {
                        tracing::error!("cron run job {} failed: {}", job_id_clone, e);
                    }
                });

                Ok(ToolResult::new(format!(
                    "Job {} triggered (running in background)",
                    job_id
                )))
            }
            "dlq_list" => {
                let Some(ref db) = self.memory_db else {
                    return Ok(ToolResult::error(
                        "DLQ not available (no memory database)".to_string(),
                    ));
                };
                let status_filter = params["dlq_status"].as_str();
                let entries = db.list_dlq_entries(status_filter)?;
                if entries.is_empty() {
                    return Ok(ToolResult::new("No DLQ entries.".to_string()));
                }
                let lines: Vec<String> = entries
                    .iter()
                    .map(|e| {
                        format!(
                            "- [{}] job={} ({}) | status={} | retries={} | failed={} | error: {}",
                            e.id,
                            e.job_id,
                            e.job_name,
                            e.status,
                            e.retry_count,
                            e.failed_at,
                            e.error_message
                        )
                    })
                    .collect();
                Ok(ToolResult::new(format!(
                    "DLQ entries:\n{}",
                    lines.join("\n")
                )))
            }
            "dlq_replay" => {
                let Some(ref db) = self.memory_db else {
                    return Ok(ToolResult::error(
                        "DLQ not available (no memory database)".to_string(),
                    ));
                };
                let dlq_id = params["dlq_id"]
                    .as_i64()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'dlq_id' parameter for dlq_replay"))?;

                // Find the entry
                let entries = db.list_dlq_entries(None)?;
                let entry = entries.iter().find(|e| e.id == dlq_id);
                let Some(entry) = entry else {
                    return Ok(ToolResult::error(format!("DLQ entry {} not found", dlq_id)));
                };

                // Spawn replay on a separate task to avoid deadlock (same
                // reason as the "run" action — processing_lock re-entrancy).
                let job_id = entry.job_id.clone();
                db.increment_dlq_retry(dlq_id)?;
                let cron = self.cron_service.clone();
                let job_id_clone = job_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = cron.run_job(&job_id_clone, true).await {
                        tracing::error!("cron dlq_replay job {} failed: {}", job_id_clone, e);
                    }
                });

                Ok(ToolResult::new(format!(
                    "DLQ entry {} replay triggered (job {}, running in background)",
                    dlq_id, job_id
                )))
            }
            "dlq_clear" => {
                let Some(ref db) = self.memory_db else {
                    return Ok(ToolResult::error(
                        "DLQ not available (no memory database)".to_string(),
                    ));
                };
                let status_filter = params["dlq_status"].as_str();
                let deleted = db.clear_dlq(status_filter)?;
                Ok(ToolResult::new(format!(
                    "Cleared {} DLQ entries{}",
                    deleted,
                    status_filter
                        .map(|s| format!(" (status={})", s))
                        .unwrap_or_default()
                )))
            }
            _ => Ok(ToolResult::error(format!("unknown action: {}", action))),
        }
    }
}

#[cfg(test)]
mod tests;
