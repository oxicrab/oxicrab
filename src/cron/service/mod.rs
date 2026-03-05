use crate::cron::types::{CronJob, CronSchedule, UpdateJobParams};
use crate::utils::task_tracker::TaskTracker;
use anyhow::Result;
use chrono::DateTime;
use chrono_tz::Tz;
use cron::Schedule;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

const POLL_WHEN_EMPTY_SEC: u64 = 30;
const MIN_SLEEP_MS: i64 = 1000;
const MAX_SLEEP_MS: u64 = 30000;
/// Disabled jobs are pruned after this many days to prevent unbounded store growth.
const PRUNE_DISABLED_AFTER_DAYS: i64 = 30;

/// Normalize a cron expression to 6+ fields (prepend "0 " for seconds if 5-field).
/// Then validate it parses. Returns Ok(normalized) or Err with a message.
pub fn validate_cron_expr(expr: &str) -> Result<String> {
    let normalized = if expr.split_whitespace().count() == 5 {
        format!("0 {expr}")
    } else {
        expr.to_string()
    };
    normalized
        .parse::<Schedule>()
        .map_err(|e| anyhow::anyhow!("Invalid cron expression '{expr}': {e}"))?;
    Ok(normalized)
}

/// Detect the system's IANA timezone (e.g. "`America/New_York`").
/// Returns None if detection fails.
pub fn detect_system_timezone() -> Option<String> {
    iana_time_zone::get_timezone().ok()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn compute_next_run(schedule: &CronSchedule, now_ms: i64) -> Option<i64> {
    compute_next_run_with_last(schedule, now_ms, None)
}

fn compute_next_run_with_last(
    schedule: &CronSchedule,
    now_ms: i64,
    last_run_ms: Option<i64>,
) -> Option<i64> {
    match schedule {
        CronSchedule::At { at_ms } => {
            at_ms.and_then(|at| if at > now_ms { Some(at) } else { None })
        }
        CronSchedule::Every { every_ms } => every_ms.and_then(|every| {
            if every > 0 {
                // Anchor from last run time to prevent drift accumulation.
                // Use O(1) arithmetic instead of a loop to handle large gaps
                // (e.g. 24h gap with 1s interval would be 86,400 loop iterations).
                let anchor = last_run_ms.unwrap_or(now_ms);
                let gap = now_ms.saturating_sub(anchor);
                let intervals = gap / every + 1;
                intervals
                    .checked_mul(every)
                    .and_then(|offset| anchor.checked_add(offset))
            } else {
                None
            }
        }),
        // Event jobs don't poll — they fire in response to messages
        CronSchedule::Event { .. } => None,
        CronSchedule::Cron { expr, tz } => {
            if let Some(expr_str) = expr {
                // validate_cron_expr normalizes and validates the expression;
                // parse it directly here to avoid a redundant second parse.
                let normalized = if expr_str.split_whitespace().count() == 5 {
                    format!("0 {expr_str}")
                } else {
                    expr_str.clone()
                };
                let sched = normalized.parse::<Schedule>().ok()?;
                let now_sec = now_ms / 1000;
                let now_dt: Option<DateTime<Tz>> = if let Some(tz_str) = tz {
                    if let Ok(tz_val) = tz_str.parse::<Tz>() {
                        DateTime::from_timestamp(now_sec, 0).map(|dt| dt.with_timezone(&tz_val))
                    } else {
                        warn!("Invalid timezone '{}', falling back to UTC", tz_str);
                        DateTime::from_timestamp(now_sec, 0).map(|dt| dt.with_timezone(&Tz::UTC))
                    }
                } else {
                    DateTime::from_timestamp(now_sec, 0).map(|dt| dt.with_timezone(&Tz::UTC))
                };
                if let Some(now_dt) = now_dt
                    && let Some(next) = sched.after(&now_dt).next()
                {
                    return Some(next.timestamp_millis());
                }
            }
            None
        }
    }
}

/// Async callback that takes a [`CronJob`] and returns an optional result string.
type CronJobCallback = Arc<
    dyn Fn(
            CronJob,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<String>>> + Send>>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct CronService {
    db: Arc<crate::agent::memory::memory_db::MemoryDB>,
    on_job: Arc<Mutex<Option<CronJobCallback>>>,
    running: Arc<Mutex<bool>>,
    task_tracker: Arc<TaskTracker>,
}

impl CronService {
    pub fn new(db: Arc<crate::agent::memory::memory_db::MemoryDB>) -> Self {
        Self {
            db,
            on_job: Arc::new(Mutex::new(None)),
            running: Arc::new(Mutex::new(false)),
            task_tracker: Arc::new(TaskTracker::new()),
        }
    }

    pub async fn set_on_job<F>(&self, callback: F)
    where
        F: Fn(
                CronJob,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<Option<String>>> + Send>,
            > + Send
            + Sync
            + 'static,
    {
        *self.on_job.lock().await = Some(Arc::new(callback));
    }

    /// Update only a job's completion status and error by ID.
    /// Called from the job completion callback. Uses a targeted SQL UPDATE
    /// to avoid a read-modify-write race with the polling loop.
    fn update_job_status(&self, job_id: &str, status: &str, error: Option<&str>) -> Result<()> {
        self.db.update_cron_job_status(job_id, status, error)
    }

    pub async fn start(&self) -> Result<()> {
        *self.running.lock().await = true;
        let service = self.clone();
        let task_tracker_for_service = self.task_tracker.clone();

        let handle = tokio::spawn(async move {
            let mut first_tick = true;

            loop {
                if !*service.running.lock().await {
                    break;
                }

                // On first tick, recover jobs stuck in "running" from a prior crash
                if first_tick {
                    match service.db.recover_running_cron_jobs() {
                        Ok(recovered) if recovered > 0 => {
                            warn!(
                                "recovered {} cron job(s) stuck in 'running' from prior crash",
                                recovered
                            );
                        }
                        Err(e) => {
                            warn!("failed to recover running cron jobs: {}", e);
                        }
                        _ => {}
                    }
                }

                let now = now_ms();
                let mut next_run: Option<i64> = None;
                let on_job_guard = service.on_job.lock().await;
                let callback_opt = on_job_guard.as_ref().map(std::clone::Clone::clone);
                drop(on_job_guard);

                let mut jobs_to_fire: Vec<(CronJob, CronJobCallback)> = vec![];

                let jobs = match service.db.list_cron_jobs(false) {
                    Ok(jobs) => jobs,
                    Err(e) => {
                        warn!("failed to list cron jobs: {}", e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(POLL_WHEN_EMPTY_SEC))
                            .await;
                        continue;
                    }
                };

                for job in &jobs {
                    // Check expiry: disable job if past its expires_at or max_runs
                    let expired = job.expires_at_ms.is_some_and(|exp| exp <= now);
                    let exhausted = job.max_runs.is_some_and(|max| job.state.run_count >= max);
                    if expired || exhausted {
                        let reason = if expired {
                            "expired"
                        } else {
                            "max runs reached"
                        };
                        info!("Disabling cron job '{}' ({}): {}", job.name, job.id, reason);
                        if let Err(e) = service
                            .db
                            .update_cron_job_enabled(&job.id, false, None, now)
                        {
                            warn!("failed to disable cron job '{}': {}", job.id, e);
                        }
                        continue;
                    }

                    let job_next = job
                        .state
                        .next_run_at_ms
                        .or_else(|| compute_next_run(&job.schedule, now));

                    if let Some(job_next) = job_next {
                        if job_next <= now {
                            if first_tick {
                                // On startup, skip missed jobs — just advance to next run
                                info!(
                                    "Skipping missed cron job '{}' (was due at {}ms, now {}ms)",
                                    job.id, job_next, now
                                );
                                let new_next = compute_next_run(&job.schedule, now);
                                if let Err(e) = service.db.update_cron_job_state(
                                    &job.id,
                                    job.state.last_status.as_deref(),
                                    job.state.last_error.as_deref(),
                                    job.state.run_count,
                                    new_next,
                                    job.state.last_run_at_ms,
                                    job.state.last_fired_at_ms,
                                    now,
                                ) {
                                    warn!("failed to advance missed cron job '{}': {}", job.id, e);
                                }
                                if let Some(next) = new_next {
                                    next_run = Some(next_run.map_or(next, |n| n.min(next)));
                                }
                                continue;
                            }

                            // Advance next_run_at_ms BEFORE executing so the job
                            // won't re-fire on the next tick.
                            let new_next =
                                compute_next_run_with_last(&job.schedule, now, Some(now));
                            let new_run_count = job.state.run_count.saturating_add(1);
                            let enabled_after = if job.delete_after_run {
                                // Disable via update_cron_job_enabled
                                if let Err(e) = service
                                    .db
                                    .update_cron_job_enabled(&job.id, false, None, now)
                                {
                                    warn!(
                                        "failed to disable delete-after-run job '{}': {}",
                                        job.id, e
                                    );
                                }
                                false
                            } else {
                                true
                            };

                            if let Err(e) = service.db.update_cron_job_state(
                                &job.id,
                                Some("running"),
                                None,
                                new_run_count,
                                if enabled_after { new_next } else { None },
                                Some(now),
                                job.state.last_fired_at_ms,
                                now,
                            ) {
                                warn!(
                                    "failed to update cron job '{}' state before run: {}",
                                    job.id, e
                                );
                            }

                            // Collect job for firing outside the loop
                            if let Some(ref callback) = callback_opt {
                                info!("Firing cron job '{}' ({})", job.name, job.id);
                                // Use a clone with updated state for the callback
                                let mut job_for_callback = job.clone();
                                job_for_callback.state.last_run_at_ms = Some(now);
                                job_for_callback.state.last_status = Some("running".to_string());
                                job_for_callback.state.last_error = None;
                                job_for_callback.state.run_count = new_run_count;
                                job_for_callback.state.next_run_at_ms =
                                    if enabled_after { new_next } else { None };
                                jobs_to_fire.push((job_for_callback, callback.clone()));
                            }
                        } else {
                            next_run = Some(next_run.map_or(job_next, |n| n.min(job_next)));
                        }
                    }
                }

                // Prune disabled jobs that haven't been updated in PRUNE_DISABLED_AFTER_DAYS
                let prune_cutoff_ms = now - PRUNE_DISABLED_AFTER_DAYS * 24 * 60 * 60 * 1000;
                match service.db.prune_disabled_cron_jobs(prune_cutoff_ms) {
                    Ok(pruned) if pruned > 0 => {
                        info!(
                            "Pruned {} disabled cron jobs older than {} days",
                            pruned, PRUNE_DISABLED_AFTER_DAYS
                        );
                    }
                    Err(e) => {
                        warn!("failed to prune disabled cron jobs: {}", e);
                    }
                    _ => {}
                }

                first_tick = false;

                // Spawn job tasks
                for (job_clone, callback) in jobs_to_fire {
                    let svc = service.clone();
                    let job_id = job_clone.id.clone();
                    let task_tracker = service.task_tracker.clone();
                    task_tracker
                        .spawn_auto_cleanup(format!("cron_job_{job_id}"), async move {
                            let (status, error) = match callback(job_clone).await {
                                Ok(Some(result)) => {
                                    info!(
                                        "Cron job '{}' completed: {} chars",
                                        job_id,
                                        result.len()
                                    );
                                    ("success".to_string(), None)
                                }
                                Ok(None) => {
                                    info!("Cron job '{}' completed (no output)", job_id);
                                    ("success".to_string(), None)
                                }
                                Err(e) => {
                                    error!("Cron job '{}' failed: {}", job_id, e);
                                    ("error".to_string(), Some(e.to_string()))
                                }
                            };
                            if let Err(e) =
                                svc.update_job_status(&job_id, &status, error.as_deref())
                            {
                                warn!("Failed to update cron job '{}' state: {}", job_id, e);
                            }
                        })
                        .await;
                }

                let delay = if let Some(next) = next_run {
                    (next - now).max(MIN_SLEEP_MS) as u64
                } else {
                    POLL_WHEN_EMPTY_SEC * 1000
                };

                tokio::time::sleep(tokio::time::Duration::from_millis(delay.min(MAX_SLEEP_MS)))
                    .await;
            }
        });

        // Track the cron service task
        task_tracker_for_service
            .spawn("cron_service".to_string(), handle)
            .await;

        info!("Cron service started");
        Ok(())
    }

    pub async fn stop(&self) {
        *self.running.lock().await = false;
        // Cancel tracked tasks
        self.task_tracker.cancel_all().await;
    }

    pub fn add_job(&self, mut job: CronJob) -> Result<()> {
        // Auto-deduplicate names (case-insensitive) by appending suffix
        if self.db.count_cron_jobs_by_name(&job.name)? > 0 {
            // Find next available suffix (bounded to prevent pathological loops)
            let base_name = job.name.clone();
            let mut n = 2u32;
            let mut found = false;
            for _ in 0..10_000 {
                let candidate = format!("{base_name} ({n})");
                if self.db.count_cron_jobs_by_name(&candidate)? == 0 {
                    job.name = candidate;
                    found = true;
                    break;
                }
                n += 1;
            }
            if !found {
                anyhow::bail!("unable to find unique name for job after 10000 attempts");
            }
        }

        // Compute first run time eagerly so `list` shows it immediately
        if job.state.next_run_at_ms.is_none() {
            job.state.next_run_at_ms = compute_next_run(&job.schedule, now_ms());
        }

        self.db.insert_cron_job(&job)?;
        Ok(())
    }

    pub fn remove_job(&self, job_id: &str) -> Result<Option<CronJob>> {
        let job = self.db.get_cron_job(job_id)?;
        if job.is_some() {
            self.db.delete_cron_job(job_id)?;
        }
        Ok(job)
    }

    pub fn list_jobs(&self, include_disabled: bool) -> Result<Vec<CronJob>> {
        let mut jobs = self.db.list_cron_jobs(include_disabled)?;
        jobs.sort_by_key(|j| j.state.next_run_at_ms.unwrap_or(i64::MAX));
        Ok(jobs)
    }

    pub fn enable_job(&self, job_id: &str, enabled: bool) -> Result<Option<CronJob>> {
        let job = self.db.get_cron_job(job_id)?;
        if job.is_none() {
            return Ok(None);
        }
        let job = job.unwrap();
        let now = now_ms();
        let next_run = if enabled {
            compute_next_run(&job.schedule, now)
        } else {
            None
        };
        self.db
            .update_cron_job_enabled(job_id, enabled, next_run, now)?;
        self.db.get_cron_job(job_id)
    }

    pub fn update_job(&self, job_id: &str, params: &UpdateJobParams) -> Result<Option<CronJob>> {
        let job = self.db.get_cron_job(job_id)?;
        let Some(job) = job else {
            return Ok(None);
        };
        let now = now_ms();

        // If schedule changed and job is enabled, recompute next_run
        if let Some(ref new_schedule) = params.schedule {
            if !job.enabled {
                self.db.update_cron_job(job_id, params, now)?;
                return self.db.get_cron_job(job_id);
            }
            let next_run = compute_next_run(new_schedule, now);
            // First update the job fields
            self.db.update_cron_job(job_id, params, now)?;
            // Then update the next_run via state update
            self.db.update_cron_job_state(
                job_id,
                job.state.last_status.as_deref(),
                job.state.last_error.as_deref(),
                job.state.run_count,
                next_run,
                job.state.last_run_at_ms,
                job.state.last_fired_at_ms,
                now,
            )?;
        } else {
            self.db.update_cron_job(job_id, params, now)?;
        }

        self.db.get_cron_job(job_id)
    }

    /// Run a job by ID. Returns `None` if job not found or no callback configured.
    /// Returns `Some(result)` with the callback's output on success.
    pub async fn run_job(&self, job_id: &str, force: bool) -> Result<Option<Option<String>>> {
        let job = self.db.get_cron_job(job_id)?;

        if let Some(job) = job {
            if !force && !job.enabled {
                return Ok(None);
            }

            let on_job_guard = self.on_job.lock().await;
            if let Some(ref callback) = *on_job_guard {
                let job_clone = job.clone();
                let callback = callback.clone();
                drop(on_job_guard);

                let (status, error_msg, callback_result) = match callback(job_clone).await {
                    Ok(result) => ("success".to_string(), None, Ok(result)),
                    Err(e) => {
                        let err_str = e.to_string();
                        ("error".to_string(), Some(err_str), Err(e))
                    }
                };

                // Update state regardless of success or failure
                let now = now_ms();
                let new_next = compute_next_run_with_last(&job.schedule, now, Some(now));
                self.db.update_cron_job_state(
                    job_id,
                    Some(status.as_str()),
                    error_msg.as_deref(),
                    job.state.run_count.saturating_add(1),
                    new_next,
                    Some(now),
                    Some(now),
                    now,
                )?;

                // Propagate the callback error after persisting state
                Ok(Some(callback_result?))
            } else {
                warn!("Cron job callback not set, cannot run job");
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests;
