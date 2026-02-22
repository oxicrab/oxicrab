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
