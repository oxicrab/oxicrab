use super::cli_types::CronCommands;
use crate::config::load_config;
use crate::cron::service::CronService;
use crate::cron::types::{CronJob, CronJobState, CronPayload, CronSchedule};
use anyhow::{Context, Result};
use std::time::{SystemTime, UNIX_EPOCH};

#[allow(clippy::too_many_lines)]
pub(super) async fn cron_command(cmd: CronCommands) -> Result<()> {
    let _config = load_config(None)?;
    let db_path = crate::utils::get_memory_db_path()?;
    let db = std::sync::Arc::new(crate::agent::memory::memory_db::MemoryDB::new(&db_path)?);
    let cron = CronService::new(db);

    match cmd {
        CronCommands::List { all } => {
            let jobs = cron.list_jobs(all)?;
            if jobs.is_empty() {
                println!("No cron jobs found.");
            } else {
                println!("Cron jobs:");
                for job in jobs {
                    let status = if job.enabled { "enabled" } else { "disabled" };
                    let next_run = job.state.next_run_at_ms.map_or_else(
                        || "never".to_string(),
                        |ms| {
                            chrono::DateTime::from_timestamp(ms / 1000, 0).map_or_else(
                                || "invalid timestamp".to_string(),
                                |dt| format!("{}", dt.format("%Y-%m-%d %H:%M:%S")),
                            )
                        },
                    );
                    println!(
                        "  [{}] {} - {} (next: {})",
                        job.id, job.name, status, next_run
                    );
                }
            }
        }
        CronCommands::Add {
            name,
            message,
            every,
            cron: cron_expr,
            tz,
            at,
            agent_echo,
            to,
            channel,
            all_channels,
        } => {
            use crate::agent::tools::cron::resolve_all_channel_targets_from_config;
            use crate::cron::types::CronTarget;

            let targets = if all_channels {
                let config = load_config(None)?;
                let targets = resolve_all_channel_targets_from_config(Some(&config.channels));
                if targets.is_empty() {
                    anyhow::bail!("No enabled channels with allowFrom configured");
                }
                targets
            } else if let (Some(ch), Some(to_val)) = (channel, to) {
                vec![CronTarget {
                    channel: ch,
                    to: to_val,
                }]
            } else {
                anyhow::bail!("Either --channel + --to or --all-channels is required");
            };

            let schedule = if let Some(every_sec) = every {
                if !(60..=31_536_000).contains(&every_sec) {
                    anyhow::bail!("--every must be between 60 and 31536000 seconds");
                }
                CronSchedule::Every {
                    every_ms: Some(every_sec.saturating_mul(1000).min(i64::MAX as u64) as i64),
                }
            } else if let Some(expr) = cron_expr {
                // Validate the expression parses
                crate::cron::service::validate_cron_expr(&expr)?;
                let tz = tz.or_else(crate::cron::service::detect_system_timezone);
                CronSchedule::Cron {
                    expr: Some(expr),
                    tz,
                }
            } else if let Some(at_str) = at {
                let dt = chrono::DateTime::parse_from_rfc3339(&at_str)
                    .map(|d| d.to_utc())
                    .or_else(|_| {
                        chrono::NaiveDateTime::parse_from_str(&at_str, "%Y-%m-%d %H:%M:%S")
                            .map(|ndt| ndt.and_utc())
                    })
                    .context(
                        "Invalid date format. Use ISO 8601 or YYYY-MM-DD HH:MM:SS (assumed UTC)",
                    )?;
                CronSchedule::At {
                    at_ms: Some(dt.timestamp_millis()),
                }
            } else {
                anyhow::bail!("Must specify --every, --cron, or --at");
            };

            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("System time is before UNIX epoch")
                .map(|d| d.as_millis() as i64)?;

            let job = CronJob {
                id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
                name,
                enabled: true,
                schedule,
                payload: CronPayload {
                    kind: "agent_turn".to_string(),
                    message,
                    agent_echo,
                    targets,
                },
                state: CronJobState::default(),
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
                delete_after_run: false,
                expires_at_ms: None,
                max_runs: None,
                cooldown_secs: None,
                max_concurrent: None,
            };

            cron.add_job(job)?;
            println!("Cron job added successfully.");
        }
        CronCommands::Remove { id } => match cron.remove_job(&id)? {
            Some(job) => {
                println!("Removed cron job: {} ({})", job.name, job.id);
            }
            None => {
                println!("Cron job {id} not found.");
            }
        },
        CronCommands::Enable { id, disable } => match cron.enable_job(&id, !disable)? {
            Some(job) => {
                let status = if job.enabled { "enabled" } else { "disabled" };
                println!("Job {} ({}) {}", job.name, job.id, status);
            }
            None => {
                println!("Cron job {id} not found.");
            }
        },
        CronCommands::Edit {
            id,
            name,
            message,
            every,
            cron: cron_expr,
            tz,
            at,
            agent_echo,
            to,
            channel,
            all_channels,
        } => {
            use crate::agent::tools::cron::resolve_all_channel_targets_from_config;
            use crate::cron::types::CronTarget;

            let schedule = if let Some(every_sec) = every {
                if !(60..=31_536_000).contains(&every_sec) {
                    anyhow::bail!("--every must be between 60 and 31536000 seconds");
                }
                Some(CronSchedule::Every {
                    every_ms: Some(every_sec.saturating_mul(1000).min(i64::MAX as u64) as i64),
                })
            } else if let Some(expr) = cron_expr {
                crate::cron::service::validate_cron_expr(&expr)?;
                Some(CronSchedule::Cron {
                    expr: Some(expr),
                    tz,
                })
            } else if let Some(at_str) = at {
                let dt = chrono::DateTime::parse_from_rfc3339(&at_str)
                    .map(|d| d.to_utc())
                    .or_else(|_| {
                        chrono::NaiveDateTime::parse_from_str(&at_str, "%Y-%m-%d %H:%M:%S")
                            .map(|ndt| ndt.and_utc())
                    })
                    .context(
                        "Invalid date format. Use ISO 8601 or YYYY-MM-DD HH:MM:SS (assumed UTC)",
                    )?;
                Some(CronSchedule::At {
                    at_ms: Some(dt.timestamp_millis()),
                })
            } else if tz.is_some() {
                // Just updating timezone - need to get current job
                let jobs = cron.list_jobs(true)?;
                let current_job = jobs.iter().find(|j| j.id == id);
                if let Some(job) = current_job {
                    if let CronSchedule::Cron { expr, .. } = &job.schedule {
                        Some(CronSchedule::Cron {
                            expr: expr.clone(),
                            tz,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            let targets = if all_channels {
                let config = load_config(None)?;
                let targets = resolve_all_channel_targets_from_config(Some(&config.channels));
                if targets.is_empty() {
                    anyhow::bail!("No enabled channels with allowFrom configured");
                }
                Some(targets)
            } else if let (Some(ch), Some(to_val)) = (channel, to) {
                Some(vec![CronTarget {
                    channel: ch,
                    to: to_val,
                }])
            } else {
                None
            };

            match cron.update_job(
                &id,
                &crate::cron::types::UpdateJobParams {
                    name,
                    message,
                    schedule,
                    agent_echo,
                    targets,
                },
            )? {
                Some(job) => {
                    println!("Updated job: {} ({})", job.name, job.id);
                }
                None => {
                    println!("Cron job {id} not found.");
                }
            }
        }
        CronCommands::Run { id, force } => match cron.run_job(&id, force).await? {
            Some(result) => {
                println!("Job executed successfully.");
                if let Some(output) = result {
                    println!("{output}");
                }
            }
            None => {
                println!("Failed to run job {id} (not found or disabled)");
            }
        },
    }

    Ok(())
}
