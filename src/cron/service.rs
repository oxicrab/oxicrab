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

/// Detect the system's IANA timezone (e.g. "America/New_York").
/// Returns None if detection fails.
pub fn detect_system_timezone() -> Option<String> {
    iana_time_zone::get_timezone().ok()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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
                if let Some(now_dt) = now_dt {
                    if let Some(next) = sched.after(&now_dt).next() {
                        return Some(next.timestamp_millis());
                    }
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
        if !force_reload {
            if let Some(ref store) = *store_guard {
                return Ok(store.clone());
            }
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

    pub async fn start(&self) -> Result<()> {
        *self.running.lock().await = true;
        let running = self.running.clone();
        let store_path = self.store_path.clone();
        let on_job = self.on_job.clone();
        let task_tracker_for_service = self.task_tracker.clone();
        let task_tracker_for_jobs = self.task_tracker.clone();

        let handle = tokio::spawn(async move {
            let mut first_tick = true;

            loop {
                if !*running.lock().await {
                    break;
                }

                // Load jobs directly from disk each tick to pick up changes
                let mut store = match std::fs::read_to_string(&store_path) {
                    Ok(content) => {
                        serde_json::from_str::<CronStore>(&content).unwrap_or(CronStore {
                            version: 1,
                            jobs: vec![],
                        })
                    }
                    Err(_) => CronStore {
                        version: 1,
                        jobs: vec![],
                    },
                };

                let now = now_ms();
                let mut next_run: Option<i64> = None;
                let on_job_guard = on_job.lock().await;
                let callback_opt = on_job_guard.as_ref().map(|c| c.clone());
                drop(on_job_guard);

                let mut store_dirty = false;

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
                                    next_run = Some(next_run.map(|n| n.min(next)).unwrap_or(next));
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

                            // Execute job
                            if let Some(ref callback) = callback_opt {
                                let job_clone = job.clone();
                                let callback = callback.clone();
                                let job_id = job.id.clone();
                                let store_path_for_cb = store_path.clone();
                                let task_tracker_for_job = task_tracker_for_jobs.clone();
                                let job_id_for_tracking = job_id.clone();
                                info!("Firing cron job '{}' ({})", job.name, job.id);
                                task_tracker_for_job
                                    .spawn_auto_cleanup(
                                        format!("cron_job_{}", job_id_for_tracking),
                                        async move {
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
                                                    info!(
                                                        "Cron job '{}' completed (no output)",
                                                        job_id
                                                    );
                                                    ("ok".to_string(), None)
                                                }
                                                Err(e) => {
                                                    error!("Cron job '{}' failed: {}", job_id, e);
                                                    ("error".to_string(), Some(e.to_string()))
                                                }
                                            };
                                            // Write status back to store on disk
                                            if let Ok(content) =
                                                std::fs::read_to_string(&store_path_for_cb)
                                            {
                                                if let Ok(mut store) =
                                                    serde_json::from_str::<CronStore>(&content)
                                                {
                                                    for j in &mut store.jobs {
                                                        if j.id == job_id {
                                                            j.state.last_status = Some(status);
                                                            j.state.last_error = error;
                                                            break;
                                                        }
                                                    }
                                                    if let Ok(json) =
                                                        serde_json::to_string_pretty(&store)
                                                    {
                                                        let _ =
                                                            atomic_write(&store_path_for_cb, &json);
                                                    }
                                                }
                                            }
                                        },
                                    )
                                    .await;
                            }
                        } else {
                            next_run = Some(next_run.map(|n| n.min(job_next)).unwrap_or(job_next));
                        }
                    }
                }

                first_tick = false;

                // Persist updated state so fired jobs don't re-trigger
                if store_dirty {
                    if let Ok(content) = serde_json::to_string_pretty(&store) {
                        if let Err(e) = crate::utils::atomic_write(&store_path, &content) {
                            warn!("Failed to persist cron store after job execution: {}", e);
                        }
                    }
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
        self.load_store(false).await?;
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
        self.load_store(false).await?;
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
        // Force reload from disk — the scheduler loop writes directly to disk
        // bypassing the cached store, so cached data would be stale.
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
        self.load_store(false).await?;
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
        self.load_store(false).await?;
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
        let store = self.load_store(false).await?;
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
mod tests {
    use super::*;
    use crate::cron::types::{CronJob, CronJobState, CronPayload, CronSchedule};

    #[test]
    fn test_five_field_cron_needs_normalization() {
        // The `cron` crate requires 6+ fields — raw 5-field expressions fail
        let expr = "0 9 * * *";
        assert!(expr.parse::<Schedule>().is_err());

        // But normalized to 6-field (prepend seconds) it works
        let normalized = format!("0 {}", expr);
        assert!(normalized.parse::<Schedule>().is_ok());
    }

    #[test]
    fn test_compute_next_run_five_field_cron() {
        let schedule = CronSchedule::Cron {
            expr: Some("0 9 * * *".to_string()),
            tz: None,
        };
        let now = now_ms();
        let result = compute_next_run(&schedule, now);
        assert!(
            result.is_some(),
            "compute_next_run should normalize 5-field cron and return Some"
        );
    }

    #[test]
    fn test_compute_next_run_six_field_cron() {
        let schedule = CronSchedule::Cron {
            expr: Some("0 30 8 * * *".to_string()),
            tz: None,
        };
        let result = compute_next_run(&schedule, now_ms());
        assert!(result.is_some(), "6-field cron should work directly");
    }

    #[test]
    fn test_compute_next_run_with_timezone() {
        let schedule = CronSchedule::Cron {
            expr: Some("0 9 * * *".to_string()),
            tz: Some("America/New_York".to_string()),
        };
        let result = compute_next_run(&schedule, now_ms());
        assert!(result.is_some(), "cron with timezone should return Some");
    }

    #[test]
    fn test_compute_next_run_every() {
        let schedule = CronSchedule::Every {
            every_ms: Some(60_000),
        };
        let now = now_ms();
        let result = compute_next_run(&schedule, now);
        assert_eq!(result, Some(now + 60_000));
    }

    #[test]
    fn test_compute_next_run_at_future() {
        let future = now_ms() + 100_000;
        let schedule = CronSchedule::At {
            at_ms: Some(future),
        };
        assert_eq!(compute_next_run(&schedule, now_ms()), Some(future));
    }

    #[test]
    fn test_compute_next_run_at_past() {
        let past = now_ms() - 100_000;
        let schedule = CronSchedule::At { at_ms: Some(past) };
        assert_eq!(compute_next_run(&schedule, now_ms()), None);
    }

    #[test]
    fn test_validate_cron_expr_five_field() {
        let result = validate_cron_expr("0 9 * * *");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "0 0 9 * * *");
    }

    #[test]
    fn test_validate_cron_expr_six_field() {
        let result = validate_cron_expr("0 30 8 * * *");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "0 30 8 * * *");
    }

    #[test]
    fn test_validate_cron_expr_invalid() {
        let result = validate_cron_expr("not a cron");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid cron expression"));
    }

    #[test]
    fn test_detect_system_timezone() {
        let tz = detect_system_timezone();
        // Should succeed on any standard Linux/macOS system
        assert!(tz.is_some(), "should detect system timezone");
        let tz = tz.unwrap();
        assert!(
            tz.contains('/'),
            "timezone should be IANA format like Area/City, got: {}",
            tz
        );
    }

    #[tokio::test]
    async fn test_expired_job_auto_disables() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store_path = tmp.path().join("cron_jobs.json");
        let svc = CronService::new(store_path.clone());

        let now = now_ms();
        let job = CronJob {
            id: "exp-1".to_string(),
            name: "Expired Job".to_string(),
            enabled: true,
            schedule: CronSchedule::Every {
                every_ms: Some(1000),
            },
            payload: CronPayload {
                kind: "echo".to_string(),
                message: "ping".to_string(),
                agent_echo: false,
                targets: vec![],
            },
            state: CronJobState {
                next_run_at_ms: Some(now + 5000),
                ..Default::default()
            },
            created_at_ms: now,
            updated_at_ms: now,
            delete_after_run: false,
            expires_at_ms: Some(now - 1000), // already expired
            max_runs: None,
        };

        svc.add_job(job).await.unwrap();

        // Start the service — it should disable the expired job on first tick
        svc.start().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        svc.stop().await;

        let jobs = svc.list_jobs(true).await.unwrap();
        let j = jobs.iter().find(|j| j.id == "exp-1").unwrap();
        assert!(!j.enabled, "expired job should be disabled");
    }

    #[tokio::test]
    async fn test_max_runs_auto_disables() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store_path = tmp.path().join("cron_jobs.json");
        let svc = CronService::new(store_path.clone());

        let now = now_ms();
        let job = CronJob {
            id: "max-1".to_string(),
            name: "Max Runs Job".to_string(),
            enabled: true,
            schedule: CronSchedule::Every {
                every_ms: Some(1000),
            },
            payload: CronPayload {
                kind: "echo".to_string(),
                message: "ping".to_string(),
                agent_echo: false,
                targets: vec![],
            },
            state: CronJobState {
                next_run_at_ms: Some(now + 5000),
                run_count: 5,
                ..Default::default()
            },
            created_at_ms: now,
            updated_at_ms: now,
            delete_after_run: false,
            expires_at_ms: None,
            max_runs: Some(5), // already at max
        };

        svc.add_job(job).await.unwrap();

        svc.start().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        svc.stop().await;

        let jobs = svc.list_jobs(true).await.unwrap();
        let j = jobs.iter().find(|j| j.id == "max-1").unwrap();
        assert!(!j.enabled, "job at max runs should be disabled");
    }

    #[tokio::test]
    async fn test_add_job_deduplicates_names() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store_path = tmp.path().join("cron_jobs.json");
        let svc = CronService::new(store_path.clone());

        let now = now_ms();
        let make_job = |id: &str| CronJob {
            id: id.to_string(),
            name: "Daily Reminder".to_string(),
            enabled: true,
            schedule: CronSchedule::Every {
                every_ms: Some(60_000),
            },
            payload: CronPayload {
                kind: "echo".to_string(),
                message: "ping".to_string(),
                agent_echo: false,
                targets: vec![],
            },
            state: CronJobState::default(),
            created_at_ms: now,
            updated_at_ms: now,
            delete_after_run: false,
            expires_at_ms: None,
            max_runs: None,
        };

        // First job keeps its name
        svc.add_job(make_job("a1")).await.unwrap();
        // Second job with same name gets "(2)" suffix
        svc.add_job(make_job("a2")).await.unwrap();
        // Third gets "(3)"
        svc.add_job(make_job("a3")).await.unwrap();

        let jobs = svc.list_jobs(false).await.unwrap();
        let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
        assert!(names.contains(&"Daily Reminder"));
        assert!(names.contains(&"Daily Reminder (2)"));
        assert!(names.contains(&"Daily Reminder (3)"));
    }

    #[tokio::test]
    async fn test_run_job_increments_run_count() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store_path = tmp.path().join("cron_jobs.json");
        let svc = CronService::new(store_path.clone());

        let now = now_ms();
        let job = CronJob {
            id: "cnt-1".to_string(),
            name: "Counter Job".to_string(),
            enabled: true,
            schedule: CronSchedule::Every {
                every_ms: Some(60_000),
            },
            payload: CronPayload {
                kind: "echo".to_string(),
                message: "ping".to_string(),
                agent_echo: false,
                targets: vec![],
            },
            state: CronJobState::default(),
            created_at_ms: now,
            updated_at_ms: now,
            delete_after_run: false,
            expires_at_ms: None,
            max_runs: None,
        };

        svc.add_job(job).await.unwrap();
        svc.set_on_job(|_job| Box::pin(async { Ok(Some("done".to_string())) }))
            .await;

        svc.run_job("cnt-1", false).await.unwrap();
        svc.run_job("cnt-1", false).await.unwrap();

        let jobs = svc.list_jobs(false).await.unwrap();
        let j = jobs.iter().find(|j| j.id == "cnt-1").unwrap();
        assert_eq!(
            j.state.run_count, 2,
            "run_count should be 2 after 2 manual runs"
        );
    }
}
