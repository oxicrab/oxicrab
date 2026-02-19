use crate::utils::task_tracker::TaskTracker;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error, info};

const HEARTBEAT_PROMPT: &str = "Read HEARTBEAT.md in your workspace (if it exists).\nFollow any instructions or tasks listed there.\nIf nothing needs attention, reply with just: HEARTBEAT_OK";

/// Async callback that takes a prompt string and returns a result string.
type HeartbeatCallback = Arc<
    dyn Fn(String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>>
        + Send
        + Sync,
>;

pub struct HeartbeatService {
    workspace: PathBuf,
    on_heartbeat: Option<HeartbeatCallback>,
    interval_s: u64,
    enabled: bool,
    strategy_file: String,
    running: Arc<tokio::sync::Mutex<bool>>,
    task_tracker: Arc<TaskTracker>,
}

impl HeartbeatService {
    pub fn new(
        workspace: PathBuf,
        on_heartbeat: Option<HeartbeatCallback>,
        interval_s: u64,
        enabled: bool,
        strategy_file: String,
    ) -> Self {
        Self {
            workspace,
            on_heartbeat,
            interval_s,
            enabled,
            strategy_file,
            running: Arc::new(tokio::sync::Mutex::new(false)),
            task_tracker: Arc::new(TaskTracker::new()),
        }
    }

    pub async fn start(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        *self.running.lock().await = true;
        let running = self.running.clone();
        let interval = self.interval_s.max(1);
        let on_heartbeat = self.on_heartbeat.clone();
        let workspace = self.workspace.clone();
        let strategy_file = self.strategy_file.clone();
        let task_tracker = self.task_tracker.clone();

        let handle = tokio::spawn(async move {
            loop {
                if !*running.lock().await {
                    break;
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(interval)).await;

                if let Some(ref callback) = on_heartbeat {
                    let prompt = format!(
                        "{}\n\nRead {}/{}",
                        HEARTBEAT_PROMPT,
                        workspace.display(),
                        strategy_file
                    );
                    match callback(prompt).await {
                        Ok(result) => {
                            debug!("Heartbeat completed: {}", result);
                        }
                        Err(e) => {
                            error!("Heartbeat failed: {}", e);
                        }
                    }
                }
            }
        });

        // Track the heartbeat task
        task_tracker.spawn("heartbeat".to_string(), handle).await;

        info!("Heartbeat service started (every {}s)", interval);
        Ok(())
    }

    pub async fn stop(&self) {
        *self.running.lock().await = false;
        // Cancel tracked tasks
        self.task_tracker.cancel_all().await;
    }
}
