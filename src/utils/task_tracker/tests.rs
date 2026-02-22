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
