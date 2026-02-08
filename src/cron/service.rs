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
use tracing::{info, warn};

const POLL_WHEN_EMPTY_SEC: u64 = 30;

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
                if let Ok(sched) = expr_str.parse::<Schedule>() {
                    let now_sec = now_ms / 1000;
                    let now_dt: Option<DateTime<Tz>> = if let Some(tz_str) = tz {
                        if let Ok(tz_val) = tz_str.parse::<Tz>() {
                            DateTime::from_timestamp(now_sec, 0).map(|dt| dt.with_timezone(&tz_val))
                        } else {
                            DateTime::from_timestamp(now_sec, 0)
                                .map(|dt| dt.with_timezone(&Tz::UTC))
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
            let mut store: CronStore = serde_json::from_str(&content)?;
            // Migrate old channel/to format to targets
            let mut migrated = false;
            let raw: serde_json::Value = serde_json::from_str(&content)?;
            if let Some(jobs) = raw.get("jobs").and_then(|j| j.as_array()) {
                for (i, raw_job) in jobs.iter().enumerate() {
                    if let Some(payload) = raw_job.get("payload") {
                        let has_old_channel =
                            payload.get("channel").and_then(|v| v.as_str()).is_some();
                        let has_old_to = payload.get("to").and_then(|v| v.as_str()).is_some();
                        let has_targets =
                            payload.get("targets").and_then(|v| v.as_array()).is_some();
                        if has_old_channel && has_old_to && !has_targets {
                            let channel = payload["channel"].as_str().unwrap().to_string();
                            let to = payload["to"].as_str().unwrap().to_string();
                            if let Some(job) = store.jobs.get_mut(i) {
                                job.payload.targets =
                                    vec![crate::cron::types::CronTarget { channel, to }];
                                migrated = true;
                            }
                        }
                    }
                }
            }
            if migrated {
                info!("Migrated cron jobs from old channel/to format to targets");
                // Save migrated store
                drop(store_guard);
                let mut sg = self.store.lock().await;
                *sg = Some(store.clone());
                drop(sg);
                self.save_store().await?;
                return Ok(store);
            }
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

                    let job_next = job
                        .state
                        .next_run_at_ms
                        .or_else(|| compute_next_run(&job.schedule, now));

                    if let Some(job_next) = job_next {
                        if job_next <= now {
                            // Advance next_run_at_ms BEFORE executing so the job
                            // won't re-fire on the next tick.
                            job.state.last_run_at_ms = Some(now);
                            job.state.last_status = Some("ok".to_string());
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
                                // Use spawn_auto_cleanup since cron jobs are one-off executions
                                let task_tracker_for_job = task_tracker_for_jobs.clone();
                                let job_id_for_tracking = job_id.clone();
                                task_tracker_for_job
                                    .spawn_auto_cleanup(
                                        format!("cron_job_{}", job_id_for_tracking),
                                        async move {
                                            match callback(job_clone).await {
                                                Ok(Some(result)) => {
                                                    tracing::debug!(
                                                        "Cron job completed successfully: {}",
                                                        result
                                                    );
                                                }
                                                Ok(None) => {
                                                    tracing::debug!(
                                                        "Cron job completed (no result)"
                                                    );
                                                }
                                                Err(e) => {
                                                    tracing::error!("Cron job failed: {}", e);
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

                // Persist updated state so fired jobs don't re-trigger
                if store_dirty {
                    if let Ok(content) = serde_json::to_string_pretty(&store) {
                        if let Err(e) = crate::utils::atomic_write(&store_path, &content) {
                            warn!("Failed to persist cron store after job execution: {}", e);
                        }
                    }
                }

                let delay = if let Some(next) = next_run {
                    (next - now).max(1000) as u64
                } else {
                    POLL_WHEN_EMPTY_SEC * 1000
                };

                tokio::time::sleep(tokio::time::Duration::from_millis(delay.min(30000))).await;
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

    pub async fn add_job(&self, job: CronJob) -> Result<()> {
        let mut store_guard = self.store.lock().await;
        let store = store_guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("CronService store is not initialized"))?;
        store.jobs.push(job);
        drop(store_guard);
        self.save_store().await?;
        Ok(())
    }

    pub async fn remove_job(&self, job_id: &str) -> Result<Option<CronJob>> {
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
        let store = self.load_store(false).await?;
        let mut jobs: Vec<CronJob> = if include_disabled {
            store.jobs
        } else {
            store.jobs.into_iter().filter(|j| j.enabled).collect()
        };
        jobs.sort_by_key(|j| j.state.next_run_at_ms.unwrap_or(i64::MAX));
        Ok(jobs)
    }

    pub async fn enable_job(&self, job_id: &str, enabled: bool) -> Result<Option<CronJob>> {
        let mut store_guard = self.store.lock().await;
        let store = store_guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("CronService store is not initialized"))?;

        for job in &mut store.jobs {
            if job.id == job_id {
                job.enabled = enabled;
                job.updated_at_ms = now_ms();
                if enabled {
                    job.state.next_run_at_ms = compute_next_run(&job.schedule, now_ms());
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

    pub async fn run_job(&self, job_id: &str, force: bool) -> Result<bool> {
        let store = self.load_store(false).await?;
        let job = store.jobs.iter().find(|j| j.id == job_id);

        if let Some(job) = job {
            if !force && !job.enabled {
                return Ok(false);
            }

            let on_job_guard = self.on_job.lock().await;
            if let Some(ref callback) = *on_job_guard {
                let job_clone = job.clone();
                let callback = callback.clone();
                drop(on_job_guard);
                callback(job_clone).await?;

                // Update last run time
                let mut store_guard = self.store.lock().await;
                if let Some(ref mut store) = *store_guard {
                    for j in &mut store.jobs {
                        if j.id == job_id {
                            j.state.last_run_at_ms = Some(now_ms());
                            j.state.last_status = Some("success".to_string());
                            break;
                        }
                    }
                }
                drop(store_guard);
                self.save_store().await?;
                Ok(true)
            } else {
                warn!("Cron job callback not set, cannot run job");
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }
}
