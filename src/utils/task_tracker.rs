/// Task tracker for managing background tasks
/// 
/// Provides centralized tracking and cleanup of background tasks spawned with `tokio::spawn`.
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Task tracker for managing background tasks
/// 
/// Provides centralized tracking and cleanup of background tasks spawned with `tokio::spawn`.
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
    pub fn spawn_auto_cleanup<F>(&self, name: String, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let tasks = self.tasks.clone();
        let name_clone = name.clone();
        let handle = tokio::spawn(async move {
            future.await;
            let mut tasks_guard = tasks.lock().await;
            tasks_guard.remove(&name_clone);
            debug!("Task '{}' completed and removed from tracker", name_clone);
        });
        
        // Track the task
        let tasks_clone = self.tasks.clone();
        let name_clone2 = name.clone();
        tokio::spawn(async move {
            let mut tasks_guard = tasks_clone.lock().await;
            tasks_guard.insert(name_clone2, handle);
        });
    }

    /// Cancel all tracked tasks
    pub async fn cancel_all(&self) {
        let mut tasks = self.tasks.lock().await;
        let count = tasks.len();
        for (name, handle) in tasks.drain() {
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
