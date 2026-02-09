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
    channel: Arc<tokio::sync::Mutex<String>>,
    chat_id: Arc<tokio::sync::Mutex<String>>,
    channels_config: Option<ChannelsConfig>,
}

impl CronTool {
    pub fn new(cron_service: Arc<CronService>, channels_config: Option<ChannelsConfig>) -> Self {
        Self {
            cron_service,
            channel: Arc::new(tokio::sync::Mutex::new(String::new())),
            chat_id: Arc::new(tokio::sync::Mutex::new(String::new())),
            channels_config,
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
                    .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
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

    fn resolve_specific_channel_targets(&self, channel_names: &[String]) -> Vec<CronTarget> {
        let Some(ref cfg) = self.channels_config else {
            return vec![];
        };

        let mut targets = Vec::new();
        for name in channel_names {
            match name.as_str() {
                "slack" if cfg.slack.enabled => {
                    let to = cfg.slack.allow_from.first().cloned().unwrap_or_default();
                    if !to.is_empty() {
                        targets.push(CronTarget {
                            channel: "slack".to_string(),
                            to,
                        });
                    }
                }
                "discord" if cfg.discord.enabled => {
                    let to = cfg.discord.allow_from.first().cloned().unwrap_or_default();
                    if !to.is_empty() {
                        targets.push(CronTarget {
                            channel: "discord".to_string(),
                            to,
                        });
                    }
                }
                "telegram" if cfg.telegram.enabled => {
                    let to = cfg.telegram.allow_from.first().cloned().unwrap_or_default();
                    if !to.is_empty() {
                        targets.push(CronTarget {
                            channel: "telegram".to_string(),
                            to,
                        });
                    }
                }
                "whatsapp" if cfg.whatsapp.enabled => {
                    let to = cfg.whatsapp.allow_from.first().cloned().unwrap_or_default();
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

/// Format a WhatsApp target: append @s.whatsapp.net if not already present.
fn format_whatsapp_target(phone: &str) -> String {
    if phone.contains("@s.whatsapp.net") {
        phone.to_string()
    } else {
        let cleaned = phone.trim_start_matches('+');
        format!("{}@s.whatsapp.net", cleaned)
    }
}

/// Resolve all enabled channel targets from a ChannelsConfig.
/// Used by both CronTool and CLI commands.
pub fn resolve_all_channel_targets_from_config(cfg: Option<&ChannelsConfig>) -> Vec<CronTarget> {
    let Some(cfg) = cfg else {
        return vec![];
    };

    let mut targets = Vec::new();

    if cfg.slack.enabled {
        let to = cfg.slack.allow_from.first().cloned().unwrap_or_default();
        if !to.is_empty() {
            targets.push(CronTarget {
                channel: "slack".to_string(),
                to,
            });
        }
    }
    if cfg.discord.enabled {
        let to = cfg.discord.allow_from.first().cloned().unwrap_or_default();
        if !to.is_empty() {
            targets.push(CronTarget {
                channel: "discord".to_string(),
                to,
            });
        }
    }
    if cfg.telegram.enabled {
        let to = cfg.telegram.allow_from.first().cloned().unwrap_or_default();
        if !to.is_empty() {
            targets.push(CronTarget {
                channel: "telegram".to_string(),
                to,
            });
        }
    }
    if cfg.whatsapp.enabled {
        let to = cfg.whatsapp.allow_from.first().cloned().unwrap_or_default();
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
    fn name(&self) -> &str {
        "cron"
    }

    fn description(&self) -> &str {
        "Schedule recurring agent tasks. When a job fires, the message is processed as a full agent turn with access to all tools (todoist, weather, web search, etc.). The agent delivers results via the message tool. Actions: add, list, remove, run. Jobs can target the current channel, specific channels, or all channels."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "remove", "run"],
                    "description": "Action to perform"
                },
                "message": {
                    "type": "string",
                    "description": "Instruction or prompt for the agent when the job fires (for add). Can be a simple reminder or a complex task like 'fetch my todoist tasks due today and send them to me'."
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
                    "description": "Job ID (for remove or run)"
                },
                "channels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Target channels: [\"all\"] for all enabled channels, [\"slack\", \"discord\"] for specific ones, or omit for current channel only"
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

                let channels_param = params["channels"].as_array();
                let targets = self.resolve_targets(channels_param, &channel, &chat_id);

                if targets.is_empty() {
                    return Ok(ToolResult::error(
                        "Error: no valid targets resolved. Check that the specified channels are enabled and have allowFrom configured.".to_string(),
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

                let targets_desc: Vec<String> = targets.iter().map(|t| t.channel.clone()).collect();

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
                        agent_echo: false,
                        targets,
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
                        let schedule_desc = match &j.schedule {
                            CronSchedule::At { at_ms } => at_ms
                                .and_then(|ms| {
                                    chrono::DateTime::from_timestamp(ms / 1000, 0).map(|dt| {
                                        format!("once at {}", dt.format("%Y-%m-%d %H:%M UTC"))
                                    })
                                })
                                .unwrap_or_else(|| "once (no time set)".to_string()),
                            CronSchedule::Every { every_ms } => every_ms
                                .map(|ms| {
                                    let secs = ms / 1000;
                                    if secs >= 86400 {
                                        format!("every {}d", secs / 86400)
                                    } else if secs >= 3600 {
                                        format!("every {}h", secs / 3600)
                                    } else if secs >= 60 {
                                        format!("every {}m", secs / 60)
                                    } else {
                                        format!("every {}s", secs)
                                    }
                                })
                                .unwrap_or_else(|| "recurring (no interval set)".to_string()),
                            CronSchedule::Cron { expr, tz } => {
                                let tz_str = tz.as_deref().unwrap_or("UTC");
                                expr.as_deref()
                                    .map(|e| format!("cron '{}' ({})", e, tz_str))
                                    .unwrap_or_else(|| "cron (no expression)".to_string())
                            }
                        };
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
                        format!(
                            "- [{}] {} | schedule: {} | {} | targets: [{}] | message: \"{}\"",
                            j.id, j.name, schedule_desc, next_run, targets_desc, j.payload.message
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
            "run" => {
                let job_id = params["job_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'job_id' parameter for run"))?;

                match self.cron_service.run_job(job_id, true).await? {
                    true => Ok(ToolResult::new(format!(
                        "Job {} triggered successfully",
                        job_id
                    ))),
                    false => Ok(ToolResult::error(format!(
                        "Job {} not found or no callback configured",
                        job_id
                    ))),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        ChannelsConfig, DiscordConfig, SlackConfig, TelegramConfig, WhatsAppConfig,
    };

    fn make_test_channels_config() -> ChannelsConfig {
        ChannelsConfig {
            slack: SlackConfig {
                enabled: true,
                bot_token: String::new(),
                app_token: String::new(),
                allow_from: vec!["U08G6HBC89X".to_string()],
            },
            discord: DiscordConfig {
                enabled: true,
                token: String::new(),
                allow_from: vec!["123456789".to_string()],
            },
            telegram: TelegramConfig {
                enabled: true,
                token: String::new(),
                allow_from: vec!["987654321".to_string()],
                proxy: None,
            },
            whatsapp: WhatsAppConfig {
                enabled: true,
                allow_from: vec!["+15551234567".to_string()],
            },
        }
    }

    #[test]
    fn test_resolve_all_channels() {
        let cfg = make_test_channels_config();
        let targets = resolve_all_channel_targets_from_config(Some(&cfg));
        assert_eq!(targets.len(), 4);
        assert!(targets
            .iter()
            .any(|t| t.channel == "slack" && t.to == "U08G6HBC89X"));
        assert!(targets
            .iter()
            .any(|t| t.channel == "discord" && t.to == "123456789"));
        assert!(targets
            .iter()
            .any(|t| t.channel == "telegram" && t.to == "987654321"));
        assert!(targets
            .iter()
            .any(|t| t.channel == "whatsapp" && t.to == "15551234567@s.whatsapp.net"));
    }

    #[test]
    fn test_resolve_disabled_channels_excluded() {
        let mut cfg = make_test_channels_config();
        cfg.discord.enabled = false;
        cfg.whatsapp.enabled = false;
        let targets = resolve_all_channel_targets_from_config(Some(&cfg));
        assert_eq!(targets.len(), 2);
        assert!(targets.iter().any(|t| t.channel == "slack"));
        assert!(targets.iter().any(|t| t.channel == "telegram"));
        assert!(!targets.iter().any(|t| t.channel == "discord"));
        assert!(!targets.iter().any(|t| t.channel == "whatsapp"));
    }

    #[test]
    fn test_resolve_whatsapp_format() {
        assert_eq!(
            format_whatsapp_target("+15551234567"),
            "15551234567@s.whatsapp.net"
        );
        assert_eq!(
            format_whatsapp_target("15551234567"),
            "15551234567@s.whatsapp.net"
        );
        assert_eq!(
            format_whatsapp_target("15551234567@s.whatsapp.net"),
            "15551234567@s.whatsapp.net"
        );
    }

    #[test]
    fn test_resolve_no_config() {
        let targets = resolve_all_channel_targets_from_config(None);
        assert!(targets.is_empty());
    }
}
