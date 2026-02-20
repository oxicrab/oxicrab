/// Task tracker for managing background tasks
///
/// Provides centralized tracking and cleanup of background tasks spawned with `tokio::spawn`.
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

pub struct TaskTracker {
    tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

impl TaskTracker {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Spawn a tracked background task
    pub async fn spawn(&self, name: String, handle: JoinHandle<()>) {
        let mut tasks = self.tasks.lock().await;
        // If a task with this name already exists, abort it first
        if let Some(old_handle) = tasks.remove(&name) {
            warn!("Aborting existing task '{}' before spawning new one", name);
            old_handle.abort();
        }
        tasks.insert(name, handle);
    }

    /// Spawn a tracked background task that removes itself on completion
    pub async fn spawn_auto_cleanup<F>(&self, name: String, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let tasks = self.tasks.clone();
        let name_clone = name.clone();

        // Hold the lock while spawning and inserting to prevent a race condition:
        // the spawned task removes itself on completion, so we must insert the handle
        // before the task can finish and try to remove itself.
        let mut tasks_guard = self.tasks.lock().await;
        let handle = tokio::spawn(async move {
            future.await;
            tasks.lock().await.remove(&name_clone);
            debug!("Task '{}' completed and removed from tracker", name_clone);
        });
        tasks_guard.insert(name, handle);
    }

    /// Cancel all tracked tasks
    pub async fn cancel_all(&self) {
        let tasks: HashMap<String, JoinHandle<()>> = {
            let mut guard = self.tasks.lock().await;
            guard.drain().collect()
        };
        let count = tasks.len();
        for (name, handle) in tasks {
            handle.abort();
            debug!("Cancelled task '{}'", name);
        }
        if count > 0 {
            info!("Cancelled {} tracked tasks", count);
        }
    }
}

impl Default for TaskTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_and_cancel_all() {
        let tracker = TaskTracker::new();
        let handle = tokio::spawn(async {
            tokio::time::sleep(tokio::time::Duration::from_mins(1)).await;
        });
        tracker.spawn("long_task".to_string(), handle).await;

        // Task should be tracked
        assert_eq!(tracker.tasks.lock().await.len(), 1);

        tracker.cancel_all().await;
        assert!(tracker.tasks.lock().await.is_empty());
    }

    #[tokio::test]
    async fn test_spawn_replaces_existing() {
        let tracker = TaskTracker::new();
        let h1 = tokio::spawn(async {
            tokio::time::sleep(tokio::time::Duration::from_mins(1)).await;
        });
        tracker.spawn("task".to_string(), h1).await;

        let h2 = tokio::spawn(async {
            tokio::time::sleep(tokio::time::Duration::from_mins(1)).await;
        });
        // Re-spawning same name should abort old and replace
        tracker.spawn("task".to_string(), h2).await;

        assert_eq!(tracker.tasks.lock().await.len(), 1);
        tracker.cancel_all().await;
    }

    #[tokio::test]
    async fn test_spawn_auto_cleanup() {
        let tracker = TaskTracker::new();
        tracker
            .spawn_auto_cleanup("quick".to_string(), async {
                // Complete immediately
            })
            .await;

        // Give the spawned task time to run and remove itself
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        assert!(
            tracker.tasks.lock().await.is_empty(),
            "auto-cleanup task should remove itself on completion"
        );
    }

    #[tokio::test]
    async fn test_cancel_all_on_empty() {
        let tracker = TaskTracker::new();
        // Should not panic
        tracker.cancel_all().await;
        assert!(tracker.tasks.lock().await.is_empty());
    }

    #[tokio::test]
    async fn test_multiple_tasks() {
        let tracker = TaskTracker::new();
        for i in 0..5 {
            let handle = tokio::spawn(async {
                tokio::time::sleep(tokio::time::Duration::from_mins(1)).await;
            });
            tracker.spawn(format!("task_{}", i), handle).await;
        }
        assert_eq!(tracker.tasks.lock().await.len(), 5);
        tracker.cancel_all().await;
        assert!(tracker.tasks.lock().await.is_empty());
    }

    #[test]
    fn test_default() {
        let tracker = TaskTracker::default();
        // Should be equivalent to new()
        let tasks = tracker.tasks.try_lock().unwrap();
        assert!(tasks.is_empty());
    }
}
