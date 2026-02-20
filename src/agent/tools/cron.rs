use crate::agent::tools::base::ExecutionContext;
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
}

impl CronTool {
    pub fn new(cron_service: Arc<CronService>, channels_config: Option<ChannelsConfig>) -> Self {
        Self {
            cron_service,
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
        "Schedule recurring or one-shot tasks. Two job types: 'agent' (default) processes the message as a full agent turn with all tools; 'echo' delivers the message directly to channels without invoking the LLM (ideal for simple reminders like 'standup in 5 min'). Schedule with cron_expr, every_seconds, or at_time (one-shot ISO 8601). Optional limits: expires_at (auto-disable after datetime) and max_runs (auto-disable after N executions). Actions: add, list, remove, run."
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
                        "Error: invalid type '{}'. Must be 'agent' or 'echo'.",
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
                    if every_secs == 0 || every_secs > 31_536_000 {
                        return Ok(ToolResult::error(
                            "Error: every_seconds must be between 1 and 31536000 (1 year)"
                                .to_string(),
                        ));
                    }
                    CronSchedule::Every {
                        every_ms: Some((every_secs * 1000) as i64),
                    }
                } else if let Some(cron_expr) = params["cron_expr"].as_str() {
                    // Validate the expression parses before storing
                    if let Err(e) = crate::cron::service::validate_cron_expr(cron_expr) {
                        return Ok(ToolResult::error(format!("Error: {}", e)));
                    }
                    // Use explicit tz param, or detect system timezone
                    let tz = params["tz"]
                        .as_str()
                        .map(std::string::ToString::to_string)
                        .or_else(crate::cron::service::detect_system_timezone);
                    CronSchedule::Cron {
                        expr: Some(cron_expr.to_string()),
                        tz,
                    }
                } else if let Some(at_time_str) = params["at_time"].as_str() {
                    let dt = chrono::DateTime::parse_from_rfc3339(at_time_str)
                        .or_else(|_| chrono::DateTime::parse_from_str(at_time_str, "%Y-%m-%dT%H:%M:%S%z"))
                        .map_err(|_| anyhow::anyhow!("Invalid at_time format. Use ISO 8601 (e.g. '2025-01-15T09:00:00-05:00')"))?;
                    let at_ms = dt.timestamp_millis();
                    let now_ms_check = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis() as i64);
                    if at_ms <= now_ms_check {
                        return Ok(ToolResult::error(
                            "Error: at_time must be in the future".to_string(),
                        ));
                    }
                    CronSchedule::At { at_ms: Some(at_ms) }
                } else if let Some(event_pattern) = params["event_pattern"].as_str() {
                    // Validate the regex compiles
                    if let Err(e) = regex::Regex::new(event_pattern) {
                        return Ok(ToolResult::error(format!(
                            "Error: invalid event_pattern regex: {}",
                            e
                        )));
                    }
                    CronSchedule::Event {
                        pattern: Some(event_pattern.to_string()),
                        channel: params["event_channel"]
                            .as_str()
                            .map(std::string::ToString::to_string),
                    }
                } else {
                    return Ok(ToolResult::error(
                        "Error: either every_seconds, cron_expr, at_time, or event_pattern is required"
                            .to_string(),
                    ));
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
                            "Error: expires_at must be in the future".to_string(),
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
                    id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
                    name: {
                        let truncated: String = message.chars().take(30).collect();
                        if message.chars().count() > 30 {
                            format!("{}...", truncated)
                        } else {
                            message.clone()
                        }
                    },
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
                        let schedule_desc = match &j.schedule {
                            CronSchedule::At { at_ms } => at_ms
                                .and_then(|ms| {
                                    chrono::DateTime::from_timestamp(ms / 1000, 0).map(|dt| {
                                        format!("once at {}", dt.format("%Y-%m-%d %H:%M UTC"))
                                    })
                                })
                                .unwrap_or_else(|| "once (no time set)".to_string()),
                            CronSchedule::Every { every_ms } => every_ms.map_or_else(|| "recurring (no interval set)".to_string(), |ms| {
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
                                }),
                            CronSchedule::Cron { expr, tz } => {
                                let tz_str = tz.as_deref().unwrap_or("UTC");
                                expr.as_deref().map_or_else(|| "cron (no expression)".to_string(), |e| format!("cron '{}' ({})", e, tz_str))
                            }
                            CronSchedule::Event { pattern, channel } => {
                                let pat = pattern.as_deref().unwrap_or("*");
                                if let Some(ch) = channel {
                                    format!("event /{pat}/ on {ch}")
                                } else {
                                    format!("event /{pat}/")
                                }
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
                    None => Ok(ToolResult::error(format!("Job {} not found", job_id))),
                }
            }
            "run" => {
                let job_id = params["job_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'job_id' parameter for run"))?;

                match self.cron_service.run_job(job_id, true).await? {
                    Some(Some(result)) => Ok(ToolResult::new(format!(
                        "Job {} completed. Result:\n{}",
                        job_id, result
                    ))),
                    Some(None) => Ok(ToolResult::new(format!(
                        "Job {} completed (no output)",
                        job_id
                    ))),
                    None => Ok(ToolResult::error(format!(
                        "Job {} not found or no callback configured",
                        job_id
                    ))),
                }
            }
            _ => Ok(ToolResult::error(format!("Unknown action: {}", action))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        ChannelsConfig, DiscordConfig, SlackConfig, TelegramConfig, TwilioConfig, WhatsAppConfig,
    };

    fn make_test_channels_config() -> ChannelsConfig {
        ChannelsConfig {
            slack: SlackConfig {
                enabled: true,
                bot_token: String::new(),
                app_token: String::new(),
                allow_from: vec!["U08G6HBC89X".to_string()],
                dm_policy: crate::config::DmPolicy::Allowlist,
            },
            discord: DiscordConfig {
                enabled: true,
                token: String::new(),
                allow_from: vec!["123456789".to_string()],
                commands: Vec::new(),
                dm_policy: crate::config::DmPolicy::Allowlist,
            },
            telegram: TelegramConfig {
                enabled: true,
                token: String::new(),
                allow_from: vec!["987654321".to_string()],
                dm_policy: crate::config::DmPolicy::Allowlist,
            },
            whatsapp: WhatsAppConfig {
                enabled: true,
                allow_from: vec!["+15551234567".to_string()],
                dm_policy: crate::config::DmPolicy::Allowlist,
            },
            twilio: TwilioConfig::default(),
        }
    }

    #[test]
    fn test_resolve_all_channels() {
        let cfg = make_test_channels_config();
        let targets = resolve_all_channel_targets_from_config(Some(&cfg));
        assert_eq!(targets.len(), 4);
        assert!(
            targets
                .iter()
                .any(|t| t.channel == "slack" && t.to == "U08G6HBC89X")
        );
        assert!(
            targets
                .iter()
                .any(|t| t.channel == "discord" && t.to == "123456789")
        );
        assert!(
            targets
                .iter()
                .any(|t| t.channel == "telegram" && t.to == "987654321")
        );
        assert!(
            targets
                .iter()
                .any(|t| t.channel == "whatsapp" && t.to == "15551234567@s.whatsapp.net")
        );
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

    #[test]
    fn test_first_concrete_target_skips_wildcard() {
        let list = vec!["*".to_string(), "user123".to_string()];
        assert_eq!(first_concrete_target(&list), "user123");
    }

    #[test]
    fn test_first_concrete_target_empty_list() {
        let list: Vec<String> = vec![];
        assert_eq!(first_concrete_target(&list), "");
    }

    #[test]
    fn test_first_concrete_target_only_wildcard() {
        let list = vec!["*".to_string()];
        assert_eq!(first_concrete_target(&list), "");
    }

    #[test]
    fn test_first_concrete_target_no_wildcard() {
        let list = vec!["alice".to_string(), "bob".to_string()];
        assert_eq!(first_concrete_target(&list), "alice");
    }

    #[test]
    fn test_format_whatsapp_target_with_plus() {
        assert_eq!(
            format_whatsapp_target("+441234567890"),
            "441234567890@s.whatsapp.net"
        );
    }

    #[test]
    fn test_resolve_empty_allow_from_excluded() {
        let mut cfg = make_test_channels_config();
        cfg.slack.allow_from = vec![];
        let targets = resolve_all_channel_targets_from_config(Some(&cfg));
        // Slack should be excluded (empty allow_from → first_concrete_target returns "")
        assert!(!targets.iter().any(|t| t.channel == "slack"));
    }

    #[test]
    fn test_resolve_wildcard_only_excluded() {
        let mut cfg = make_test_channels_config();
        cfg.telegram.allow_from = vec!["*".to_string()];
        let targets = resolve_all_channel_targets_from_config(Some(&cfg));
        // Telegram wildcard has no concrete target → excluded
        assert!(!targets.iter().any(|t| t.channel == "telegram"));
    }
}
