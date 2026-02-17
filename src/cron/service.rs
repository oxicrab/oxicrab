use crate::cron::types::{CronJob, CronSchedule, CronStore, UpdateJobParams};
use crate::utils::atomic_write;
use crate::utils::task_tracker::TaskTracker;
use anyhow::Result;
use chrono::DateTime;
use chrono_tz::Tz;
use cron::Schedule;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

const POLL_WHEN_EMPTY_SEC: u64 = 30;
const MIN_SLEEP_MS: i64 = 1000;
const MAX_SLEEP_MS: u64 = 30000;

/// Normalize a cron expression to 6+ fields (prepend "0 " for seconds if 5-field).
/// Then validate it parses. Returns Ok(normalized) or Err with a message.
pub fn validate_cron_expr(expr: &str) -> Result<String> {
    let normalized = if expr.split_whitespace().count() == 5 {
        format!("0 {}", expr)
    } else {
        expr.to_string()
    };
    normalized
        .parse::<Schedule>()
        .map_err(|e| anyhow::anyhow!("Invalid cron expression '{}': {}", expr, e))?;
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
    match schedule {
        CronSchedule::At { at_ms } => {
            at_ms.and_then(|at| if at > now_ms { Some(at) } else { None })
        }
        CronSchedule::Every { every_ms } => every_ms.and_then(|every| {
            if every > 0 {
                Some(now_ms + every)
            } else {
                None
            }
        }),
        // Event jobs don't poll — they fire in response to messages
        CronSchedule::Event { .. } => None,
        CronSchedule::Cron { expr, tz } => {
            if let Some(expr_str) = expr {
                let normalized = validate_cron_expr(expr_str).ok()?;
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
    store_path: PathBuf,
    store: Arc<Mutex<Option<CronStore>>>,
    on_job: Arc<tokio::sync::Mutex<Option<CronJobCallback>>>,
    running: Arc<tokio::sync::Mutex<bool>>,
    task_tracker: Arc<TaskTracker>,
}

impl CronService {
    pub fn new(store_path: PathBuf) -> Self {
        Self {
            store_path,
            store: Arc::new(Mutex::new(None)),
            on_job: Arc::new(tokio::sync::Mutex::new(None)),
            running: Arc::new(tokio::sync::Mutex::new(false)),
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

    pub async fn load_store(&self, force_reload: bool) -> Result<CronStore> {
        let mut store_guard = self.store.lock().await;
        if !force_reload && let Some(ref store) = *store_guard {
            return Ok(store.clone());
        }

        if self.store_path.exists() {
            let content = std::fs::read_to_string(&self.store_path)?;
            let store: CronStore = serde_json::from_str(&content)?;
            *store_guard = Some(store.clone());
            return Ok(store);
        }

        let store = CronStore {
            version: 1,
            jobs: vec![],
        };
        *store_guard = Some(store.clone());
        Ok(store)
    }

    async fn save_store(&self) -> Result<()> {
        let store_guard = self.store.lock().await;
        if let Some(store) = store_guard.as_ref() {
            let content = serde_json::to_string_pretty(store)?;
            atomic_write(&self.store_path, &content)?;
        }
        Ok(())
    }

    /// Update a job's runtime state (status, error) by ID.
    /// Called from the job completion callback.
    async fn update_job_state(
        &self,
        job_id: &str,
        status: String,
        error: Option<String>,
    ) -> Result<()> {
        let mut store_guard = self.store.lock().await;
        if let Some(store) = store_guard.as_mut()
            && let Some(job) = store.jobs.iter_mut().find(|j| j.id == job_id)
        {
            job.state.last_status = Some(status);
            job.state.last_error = error;
        }
        drop(store_guard);
        self.save_store().await
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

                // Force-reload from disk each tick to pick up external CLI changes,
                // then work through the shared store mutex.
                if let Err(e) = service.load_store(true).await {
                    warn!("Failed to reload cron store: {}", e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(POLL_WHEN_EMPTY_SEC)).await;
                    continue;
                }

                let now = now_ms();
                let mut next_run: Option<i64> = None;
                let on_job_guard = service.on_job.lock().await;
                let callback_opt = on_job_guard.as_ref().map(std::clone::Clone::clone);
                drop(on_job_guard);

                let mut store_dirty = false;
                let mut jobs_to_fire: Vec<(CronJob, CronJobCallback)> = vec![];

                let mut store_guard = service.store.lock().await;
                let Some(store) = store_guard.as_mut() else {
                    drop(store_guard);
                    tokio::time::sleep(tokio::time::Duration::from_secs(POLL_WHEN_EMPTY_SEC)).await;
                    continue;
                };

                for job in &mut store.jobs {
                    if !job.enabled {
                        continue;
                    }

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
                        job.enabled = false;
                        job.state.next_run_at_ms = None;
                        job.updated_at_ms = now;
                        store_dirty = true;
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
                                job.state.next_run_at_ms = compute_next_run(&job.schedule, now);
                                job.updated_at_ms = now;
                                store_dirty = true;
                                if let Some(next) = job.state.next_run_at_ms {
                                    next_run = Some(next_run.map_or(next, |n| n.min(next)));
                                }
                                continue;
                            }

                            // Advance next_run_at_ms BEFORE executing so the job
                            // won't re-fire on the next tick.
                            job.state.last_run_at_ms = Some(now);
                            job.state.last_status = Some("running".to_string());
                            job.state.last_error = None;
                            job.state.run_count += 1;
                            job.state.next_run_at_ms = compute_next_run(&job.schedule, now);
                            job.updated_at_ms = now;
                            store_dirty = true;

                            if job.delete_after_run {
                                job.enabled = false;
                            }

                            // Collect job for firing outside the lock
                            if let Some(ref callback) = callback_opt {
                                info!("Firing cron job '{}' ({})", job.name, job.id);
                                jobs_to_fire.push((job.clone(), callback.clone()));
                            }
                        } else {
                            next_run = Some(next_run.map_or(job_next, |n| n.min(job_next)));
                        }
                    }
                }
                drop(store_guard);

                first_tick = false;

                // Persist updated state so fired jobs don't re-trigger
                if store_dirty && let Err(e) = service.save_store().await {
                    warn!("Failed to persist cron store after tick: {}", e);
                }

                // Spawn job tasks outside the lock
                for (job_clone, callback) in jobs_to_fire {
                    let svc = service.clone();
                    let job_id = job_clone.id.clone();
                    let task_tracker = service.task_tracker.clone();
                    task_tracker
                        .spawn_auto_cleanup(format!("cron_job_{}", job_id), async move {
                            let (status, error) = match callback(job_clone).await {
                                Ok(Some(result)) => {
                                    info!(
                                        "Cron job '{}' completed: {} chars",
                                        job_id,
                                        result.len()
                                    );
                                    ("ok".to_string(), None)
                                }
                                Ok(None) => {
                                    info!("Cron job '{}' completed (no output)", job_id);
                                    ("ok".to_string(), None)
                                }
                                Err(e) => {
                                    error!("Cron job '{}' failed: {}", job_id, e);
                                    ("error".to_string(), Some(e.to_string()))
                                }
                            };
                            if let Err(e) = svc.update_job_state(&job_id, status, error).await {
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

    pub async fn add_job(&self, mut job: CronJob) -> Result<()> {
        self.load_store(true).await?;
        let mut store_guard = self.store.lock().await;
        let store = store_guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("CronService store is not initialized"))?;

        // Auto-deduplicate names (case-insensitive) by appending suffix
        let base_lower = job.name.to_lowercase();
        let has_dup = store
            .jobs
            .iter()
            .any(|j| j.name.to_lowercase() == base_lower);
        if has_dup {
            // Find next available suffix
            let mut n = 2u32;
            loop {
                let candidate = format!("{} ({})", job.name, n);
                let cand_lower = candidate.to_lowercase();
                if !store
                    .jobs
                    .iter()
                    .any(|j| j.name.to_lowercase() == cand_lower)
                {
                    job.name = candidate;
                    break;
                }
                n += 1;
            }
        }

        // Compute first run time eagerly so `list` shows it immediately
        if job.state.next_run_at_ms.is_none() {
            job.state.next_run_at_ms = compute_next_run(&job.schedule, now_ms());
        }

        store.jobs.push(job);
        drop(store_guard);
        self.save_store().await?;
        Ok(())
    }

    pub async fn remove_job(&self, job_id: &str) -> Result<Option<CronJob>> {
        self.load_store(true).await?;
        let mut store_guard = self.store.lock().await;
        let store = store_guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("CronService store is not initialized"))?;
        let mut removed = None;
        let mut remaining = Vec::new();
        for job in store.jobs.drain(..) {
            if job.id == job_id {
                removed = Some(job);
            } else {
                remaining.push(job);
            }
        }
        store.jobs = remaining;
        drop(store_guard);
        if removed.is_some() {
            self.save_store().await?;
        }
        Ok(removed)
    }

    pub async fn list_jobs(&self, include_disabled: bool) -> Result<Vec<CronJob>> {
        // Force reload from disk to pick up external CLI changes
        let store = self.load_store(true).await?;
        let mut jobs: Vec<CronJob> = if include_disabled {
            store.jobs
        } else {
            store.jobs.into_iter().filter(|j| j.enabled).collect()
        };
        jobs.sort_by_key(|j| j.state.next_run_at_ms.unwrap_or(i64::MAX));
        Ok(jobs)
    }

    pub async fn enable_job(&self, job_id: &str, enabled: bool) -> Result<Option<CronJob>> {
        self.load_store(true).await?;
        let mut store_guard = self.store.lock().await;
        let store = store_guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("CronService store is not initialized"))?;

        for job in &mut store.jobs {
            if job.id == job_id {
                let now = now_ms();
                job.enabled = enabled;
                job.updated_at_ms = now;
                if enabled {
                    job.state.next_run_at_ms = compute_next_run(&job.schedule, now);
                } else {
                    job.state.next_run_at_ms = None;
                }
                let result = Some(job.clone());
                drop(store_guard);
                self.save_store().await?;
                return Ok(result);
            }
        }
        Ok(None)
    }

    pub async fn update_job(
        &self,
        job_id: &str,
        params: UpdateJobParams,
    ) -> Result<Option<CronJob>> {
        self.load_store(true).await?;
        let mut store_guard = self.store.lock().await;
        let store = store_guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("CronService store is not initialized"))?;

        for job in &mut store.jobs {
            if job.id == job_id {
                if let Some(n) = params.name {
                    job.name = n;
                }
                if let Some(m) = params.message {
                    job.payload.message = m;
                }
                if let Some(s) = params.schedule {
                    job.schedule = s.clone();
                    if job.enabled {
                        job.state.next_run_at_ms = compute_next_run(&s, now_ms());
                    }
                }
                if let Some(d) = params.agent_echo {
                    job.payload.agent_echo = d;
                }
                if let Some(targets) = params.targets {
                    job.payload.targets = targets;
                }
                job.updated_at_ms = now_ms();
                let result = Some(job.clone());
                drop(store_guard);
                self.save_store().await?;
                return Ok(result);
            }
        }
        Ok(None)
    }

    /// Run a job by ID. Returns `None` if job not found or no callback configured.
    /// Returns `Some(result)` with the callback's output on success.
    pub async fn run_job(&self, job_id: &str, force: bool) -> Result<Option<Option<String>>> {
        let store = self.load_store(true).await?;
        let job = store.jobs.iter().find(|j| j.id == job_id);

        if let Some(job) = job {
            if !force && !job.enabled {
                return Ok(None);
            }

            let on_job_guard = self.on_job.lock().await;
            if let Some(ref callback) = *on_job_guard {
                let job_clone = job.clone();
                let callback = callback.clone();
                drop(on_job_guard);
                let result = callback(job_clone).await?;

                // Update last run time and compute next run
                let now = now_ms();
                let mut store_guard = self.store.lock().await;
                if let Some(ref mut store) = *store_guard {
                    for j in &mut store.jobs {
                        if j.id == job_id {
                            j.state.last_run_at_ms = Some(now);
                            j.state.last_status = Some("success".to_string());
                            j.state.run_count += 1;
                            j.state.next_run_at_ms = compute_next_run(&j.schedule, now);
                            j.updated_at_ms = now;
                            break;
                        }
                    }
                }
                drop(store_guard);
                self.save_store().await?;
                Ok(Some(result))
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
#[path = "service_tests.rs"]
mod tests;
