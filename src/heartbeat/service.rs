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
            interval_s: interval_s.max(1),
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

        let mut running_guard = self.running.lock().await;
        if *running_guard {
            return Ok(()); // Already running
        }
        *running_guard = true;
        drop(running_guard);

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

                // Re-check after sleep — stop() may have been called during the wait
                if !*running.lock().await {
                    break;
                }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_disabled_service_is_noop() {
        let svc = HeartbeatService::new(
            PathBuf::from("/tmp"),
            None,
            60,
            false,
            "HEARTBEAT.md".to_string(),
        );
        // start on a disabled service should return Ok immediately
        svc.start().await.unwrap();
        assert!(!*svc.running.lock().await);
    }

    #[tokio::test]
    async fn test_start_sets_running_flag() {
        let call_count = Arc::new(tokio::sync::Mutex::new(0u32));
        let cc = call_count.clone();
        let callback: HeartbeatCallback = Arc::new(move |_prompt| {
            let cc = cc.clone();
            Box::pin(async move {
                *cc.lock().await += 1;
                Ok("HEARTBEAT_OK".to_string())
            })
        });

        let svc = HeartbeatService::new(
            PathBuf::from("/tmp"),
            Some(callback),
            1,
            true,
            "HEARTBEAT.md".to_string(),
        );

        svc.start().await.unwrap();
        assert!(*svc.running.lock().await);

        svc.stop().await;
        assert!(!*svc.running.lock().await);
    }

    #[tokio::test]
    async fn test_callback_invoked() {
        let call_count = Arc::new(tokio::sync::Mutex::new(0u32));
        let cc = call_count.clone();
        let callback: HeartbeatCallback = Arc::new(move |_prompt| {
            let cc = cc.clone();
            Box::pin(async move {
                *cc.lock().await += 1;
                Ok("HEARTBEAT_OK".to_string())
            })
        });

        let svc = HeartbeatService::new(
            PathBuf::from("/tmp/test-workspace"),
            Some(callback),
            1, // 1 second interval
            true,
            "HEARTBEAT.md".to_string(),
        );

        svc.start().await.unwrap();
        // Wait enough for at least one callback invocation
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
        svc.stop().await;

        let count = *call_count.lock().await;
        assert!(count >= 1, "expected at least 1 callback, got {count}");
    }

    #[tokio::test]
    async fn test_callback_error_does_not_crash() {
        let callback: HeartbeatCallback =
            Arc::new(|_prompt| Box::pin(async { Err(anyhow::anyhow!("simulated failure")) }));

        let svc = HeartbeatService::new(
            PathBuf::from("/tmp"),
            Some(callback),
            1,
            true,
            "HEARTBEAT.md".to_string(),
        );

        svc.start().await.unwrap();
        // Let it fail at least once — should log error but not panic
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
        svc.stop().await;
    }

    #[tokio::test]
    async fn test_no_callback_runs_without_panic() {
        let svc = HeartbeatService::new(
            PathBuf::from("/tmp"),
            None, // no callback
            1,
            true,
            "HEARTBEAT.md".to_string(),
        );

        svc.start().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
        svc.stop().await;
    }

    #[tokio::test]
    async fn test_interval_zero_clamped_to_one() {
        let call_count = Arc::new(tokio::sync::Mutex::new(0u32));
        let cc = call_count.clone();
        let callback: HeartbeatCallback = Arc::new(move |_prompt| {
            let cc = cc.clone();
            Box::pin(async move {
                *cc.lock().await += 1;
                Ok("ok".to_string())
            })
        });

        // interval_s = 0 should be clamped to 1 (not spin-loop)
        let svc = HeartbeatService::new(
            PathBuf::from("/tmp"),
            Some(callback),
            0,
            true,
            "HEARTBEAT.md".to_string(),
        );

        svc.start().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
        svc.stop().await;

        let count = *call_count.lock().await;
        // With 1s clamped interval, 1.5s should produce at most 1-2 calls (not thousands)
        assert!(
            count <= 3,
            "expected <=3, got {count} (interval not clamped?)"
        );
    }

    #[tokio::test]
    async fn test_prompt_contains_workspace_and_strategy() {
        let captured = Arc::new(tokio::sync::Mutex::new(String::new()));
        let cap = captured.clone();
        let callback: HeartbeatCallback = Arc::new(move |prompt| {
            let cap = cap.clone();
            Box::pin(async move {
                *cap.lock().await = prompt;
                Ok("ok".to_string())
            })
        });

        let svc = HeartbeatService::new(
            PathBuf::from("/workspace/test"),
            Some(callback),
            1,
            true,
            "STRATEGY.md".to_string(),
        );

        svc.start().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
        svc.stop().await;

        let prompt = captured.lock().await;
        assert!(
            prompt.contains("/workspace/test"),
            "prompt missing workspace path"
        );
        assert!(
            prompt.contains("STRATEGY.md"),
            "prompt missing strategy file"
        );
        assert!(
            prompt.contains("HEARTBEAT"),
            "prompt missing HEARTBEAT_PROMPT"
        );
    }
}
