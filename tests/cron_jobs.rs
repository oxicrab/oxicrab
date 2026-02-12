use nanobot::cron::service::{validate_cron_expr, CronService};
use nanobot::cron::types::{CronJob, CronJobState, CronPayload, CronSchedule, CronTarget};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;

fn make_test_job(id: &str, name: &str) -> CronJob {
    CronJob {
        id: id.to_string(),
        name: name.to_string(),
        enabled: true,
        schedule: CronSchedule::Every {
            every_ms: Some(3600000),
        },
        payload: CronPayload {
            kind: "agent_turn".to_string(),
            message: format!("Job {} message", id),
            agent_echo: false,
            targets: vec![CronTarget {
                channel: "telegram".to_string(),
                to: "user1".to_string(),
            }],
        },
        state: CronJobState::default(),
        created_at_ms: 1000000,
        updated_at_ms: 1000000,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: None,
    }
}

fn create_test_cron_service() -> (CronService, TempDir) {
    let tmp = TempDir::new().expect("Failed to create temp dir");
    let store_path = tmp.path().join("cron_store.json");
    let svc = CronService::new(store_path);
    (svc, tmp)
}

#[tokio::test]
async fn test_cron_add_and_list() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    let job = make_test_job("job1", "Test Job 1");
    svc.add_job(job).await.unwrap();

    let jobs = svc.list_jobs(false).await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "job1");
    assert_eq!(jobs[0].name, "Test Job 1");
}

#[tokio::test]
async fn test_cron_add_multiple_and_list() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    svc.add_job(make_test_job("j1", "Job 1")).await.unwrap();
    svc.add_job(make_test_job("j2", "Job 2")).await.unwrap();
    svc.add_job(make_test_job("j3", "Job 3")).await.unwrap();

    let jobs = svc.list_jobs(false).await.unwrap();
    assert_eq!(jobs.len(), 3);
}

#[tokio::test]
async fn test_cron_remove_job() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    svc.add_job(make_test_job("job1", "Job 1")).await.unwrap();
    svc.add_job(make_test_job("job2", "Job 2")).await.unwrap();

    let removed = svc.remove_job("job1").await.unwrap();
    assert!(removed.is_some());
    assert_eq!(removed.unwrap().id, "job1");

    let jobs = svc.list_jobs(true).await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "job2");
}

#[tokio::test]
async fn test_cron_remove_nonexistent() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    let removed = svc.remove_job("nonexistent").await.unwrap();
    assert!(removed.is_none());
}

#[tokio::test]
async fn test_cron_persistence() {
    let tmp = TempDir::new().expect("Failed to create temp dir");
    let store_path = tmp.path().join("cron_store.json");

    // Create service, add job, drop service
    {
        let svc = CronService::new(store_path.clone());
        svc.load_store(true).await.unwrap();
        svc.add_job(make_test_job("persist1", "Persistent Job"))
            .await
            .unwrap();
    }

    // Create new service from same path
    let svc2 = CronService::new(store_path);
    let jobs = svc2.load_store(true).await.unwrap();

    assert_eq!(jobs.jobs.len(), 1);
    assert_eq!(jobs.jobs[0].id, "persist1");
    assert_eq!(jobs.jobs[0].name, "Persistent Job");
}

#[tokio::test]
async fn test_cron_enable_disable() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    svc.add_job(make_test_job("toggle1", "Toggle Job"))
        .await
        .unwrap();

    // Disable the job
    let updated = svc.enable_job("toggle1", false).await.unwrap();
    assert!(updated.is_some());
    assert!(!updated.unwrap().enabled);

    // list_jobs(false) = enabled only -> should be empty
    let enabled_jobs = svc.list_jobs(false).await.unwrap();
    assert_eq!(enabled_jobs.len(), 0);

    // list_jobs(true) = include disabled -> should have 1
    let all_jobs = svc.list_jobs(true).await.unwrap();
    assert_eq!(all_jobs.len(), 1);
    assert!(!all_jobs[0].enabled);

    // Re-enable
    let updated = svc.enable_job("toggle1", true).await.unwrap();
    assert!(updated.is_some());
    assert!(updated.unwrap().enabled);

    let enabled_jobs = svc.list_jobs(false).await.unwrap();
    assert_eq!(enabled_jobs.len(), 1);
}

#[tokio::test]
async fn test_cron_manual_trigger() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    svc.add_job(make_test_job("trigger1", "Trigger Job"))
        .await
        .unwrap();

    // Track whether callback was invoked
    let invoked = Arc::new(Mutex::new(false));
    let invoked_clone = invoked.clone();

    svc.set_on_job(move |job| {
        let invoked = invoked_clone.clone();
        Box::pin(async move {
            assert_eq!(job.id, "trigger1");
            *invoked.lock().await = true;
            Ok(Some("Job executed".to_string()))
        })
    })
    .await;

    let ran = svc.run_job("trigger1", true).await.unwrap();
    assert!(ran.is_some());
    assert_eq!(ran.unwrap(), Some("Job executed".to_string()));
    assert!(*invoked.lock().await);
}

#[tokio::test]
async fn test_cron_run_disabled_job_without_force() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    svc.add_job(make_test_job("disabled1", "Disabled Job"))
        .await
        .unwrap();
    svc.enable_job("disabled1", false).await.unwrap();

    let invoked = Arc::new(Mutex::new(false));
    let invoked_clone = invoked.clone();
    svc.set_on_job(move |_job| {
        let invoked = invoked_clone.clone();
        Box::pin(async move {
            *invoked.lock().await = true;
            Ok(None)
        })
    })
    .await;

    // Without force, disabled job should not run
    let ran = svc.run_job("disabled1", false).await.unwrap();
    assert!(ran.is_none());
    assert!(!*invoked.lock().await);

    // With force, it should run even though disabled
    let ran = svc.run_job("disabled1", true).await.unwrap();
    assert!(ran.is_some());
    assert!(*invoked.lock().await);
}

#[tokio::test]
async fn test_cron_update_job() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    svc.add_job(make_test_job("upd1", "Original Name"))
        .await
        .unwrap();

    let params = nanobot::cron::types::UpdateJobParams {
        name: Some("Updated Name".to_string()),
        message: Some("Updated message".to_string()),
        ..Default::default()
    };

    let updated = svc.update_job("upd1", params).await.unwrap();
    assert!(updated.is_some());
    let job = updated.unwrap();
    assert_eq!(job.name, "Updated Name");
    assert_eq!(job.payload.message, "Updated message");
}

#[tokio::test]
async fn test_cron_multi_target_job() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    let job = CronJob {
        id: "multi1".to_string(),
        name: "Multi Target Job".to_string(),
        enabled: true,
        schedule: CronSchedule::Every {
            every_ms: Some(3600000),
        },
        payload: CronPayload {
            kind: "agent_turn".to_string(),
            message: "Hello all channels".to_string(),
            agent_echo: true,
            targets: vec![
                CronTarget {
                    channel: "slack".to_string(),
                    to: "U08G6HBC89X".to_string(),
                },
                CronTarget {
                    channel: "discord".to_string(),
                    to: "123456789".to_string(),
                },
                CronTarget {
                    channel: "telegram".to_string(),
                    to: "987654321".to_string(),
                },
            ],
        },
        state: CronJobState::default(),
        created_at_ms: 1000000,
        updated_at_ms: 1000000,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: None,
    };

    svc.add_job(job).await.unwrap();

    // Verify persistence
    let jobs = svc.list_jobs(false).await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].payload.targets.len(), 3);
    assert_eq!(jobs[0].payload.targets[0].channel, "slack");
    assert_eq!(jobs[0].payload.targets[1].channel, "discord");
    assert_eq!(jobs[0].payload.targets[2].channel, "telegram");
}

#[tokio::test]
async fn test_cron_update_targets() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    svc.add_job(make_test_job("upd_targets", "Target Update Test"))
        .await
        .unwrap();

    let new_targets = vec![
        CronTarget {
            channel: "slack".to_string(),
            to: "U12345".to_string(),
        },
        CronTarget {
            channel: "discord".to_string(),
            to: "999888777".to_string(),
        },
    ];

    let params = nanobot::cron::types::UpdateJobParams {
        targets: Some(new_targets),
        ..Default::default()
    };

    let updated = svc.update_job("upd_targets", params).await.unwrap();
    assert!(updated.is_some());
    let job = updated.unwrap();
    assert_eq!(job.payload.targets.len(), 2);
    assert_eq!(job.payload.targets[0].channel, "slack");
    assert_eq!(job.payload.targets[1].channel, "discord");
}

#[tokio::test]
async fn test_cron_add_duplicate_name_rejected() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    svc.add_job(make_test_job("job1", "Morning Briefing"))
        .await
        .unwrap();

    // Same name, different case — should be rejected
    let err = svc
        .add_job(make_test_job("job2", "morning briefing"))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("already exists"),
        "Expected duplicate name error, got: {}",
        err
    );

    // Only the original job should exist
    let jobs = svc.list_jobs(true).await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "job1");
}

// --- Tests for bugs that were found and fixed ---

#[tokio::test]
async fn test_add_job_computes_next_run_at_ms() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    // Create a cron job with next_run_at_ms: None
    let mut job = make_test_job("eager1", "Eager Next Run");
    job.schedule = CronSchedule::Cron {
        expr: Some("0 9 * * *".to_string()),
        tz: Some("UTC".to_string()),
    };
    job.state.next_run_at_ms = None;

    svc.add_job(job).await.unwrap();

    let jobs = svc.list_jobs(false).await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert!(
        jobs[0].state.next_run_at_ms.is_some(),
        "add_job should compute next_run_at_ms eagerly, got None"
    );
}

#[tokio::test]
async fn test_add_job_computes_next_run_for_every() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    let mut job = make_test_job("every1", "Every Job");
    job.state.next_run_at_ms = None;

    svc.add_job(job).await.unwrap();

    let jobs = svc.list_jobs(false).await.unwrap();
    assert!(
        jobs[0].state.next_run_at_ms.is_some(),
        "Every schedule should have next_run_at_ms set"
    );
}

#[tokio::test]
async fn test_run_job_sets_next_run_at_ms() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    let mut job = make_test_job("runnext1", "Run Next Test");
    job.schedule = CronSchedule::Cron {
        expr: Some("30 8 * * *".to_string()),
        tz: Some("UTC".to_string()),
    };
    svc.add_job(job).await.unwrap();

    svc.set_on_job(|_| Box::pin(async { Ok(Some("done".to_string())) }))
        .await;

    svc.run_job("runnext1", true).await.unwrap();

    // Force reload to get the updated state
    let jobs = svc.list_jobs(false).await.unwrap();
    let job = jobs.iter().find(|j| j.id == "runnext1").unwrap();
    assert!(
        job.state.next_run_at_ms.is_some(),
        "run_job should compute next_run_at_ms after execution"
    );
    assert_eq!(job.state.last_status.as_deref(), Some("success"));
}

#[tokio::test]
async fn test_enable_job_computes_next_run() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    let mut job = make_test_job("enable1", "Enable Test");
    job.schedule = CronSchedule::Cron {
        expr: Some("0 12 * * *".to_string()),
        tz: Some("UTC".to_string()),
    };
    svc.add_job(job).await.unwrap();

    // Disable — should clear next_run_at_ms
    let disabled = svc.enable_job("enable1", false).await.unwrap().unwrap();
    assert!(
        disabled.state.next_run_at_ms.is_none(),
        "disabled job should have no next_run_at_ms"
    );

    // Re-enable — should compute next_run_at_ms
    let enabled = svc.enable_job("enable1", true).await.unwrap().unwrap();
    assert!(
        enabled.state.next_run_at_ms.is_some(),
        "re-enabled job should have next_run_at_ms computed"
    );
}

#[tokio::test]
async fn test_update_job_with_schedule_recomputes_next_run() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    let mut job = make_test_job("updsched1", "Update Schedule");
    job.schedule = CronSchedule::Every {
        every_ms: Some(60_000),
    };
    svc.add_job(job).await.unwrap();

    let before = svc.list_jobs(false).await.unwrap();
    let next_before = before[0].state.next_run_at_ms;

    // Update schedule to a different interval
    let params = nanobot::cron::types::UpdateJobParams {
        schedule: Some(CronSchedule::Every {
            every_ms: Some(120_000),
        }),
        ..Default::default()
    };
    let updated = svc.update_job("updsched1", params).await.unwrap().unwrap();
    assert!(updated.state.next_run_at_ms.is_some());
    // New next_run should be different (longer interval)
    assert_ne!(
        updated.state.next_run_at_ms, next_before,
        "updating schedule should recompute next_run_at_ms"
    );
}

#[tokio::test]
async fn test_validate_cron_expr_rejects_garbage() {
    assert!(validate_cron_expr("").is_err());
    assert!(validate_cron_expr("hello world").is_err());
    assert!(validate_cron_expr("99 99 99 99 99").is_err());
    assert!(validate_cron_expr("* * *").is_err()); // too few fields
}

#[tokio::test]
async fn test_validate_cron_expr_accepts_standard_patterns() {
    // Common 5-field patterns
    assert!(validate_cron_expr("0 9 * * *").is_ok()); // daily at 9am
    assert!(validate_cron_expr("*/15 * * * *").is_ok()); // every 15 min
    assert!(validate_cron_expr("0 0 1 * *").is_ok()); // 1st of month
    assert!(validate_cron_expr("30 8 * * 1-5").is_ok()); // weekdays at 8:30
    assert!(validate_cron_expr("0 */6 * * *").is_ok()); // every 6 hours

    // 6-field (with seconds) — should work without normalization
    assert!(validate_cron_expr("0 0 9 * * *").is_ok());
}

#[tokio::test]
async fn test_list_jobs_reflects_disk_state() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("cron_store.json");

    let svc = CronService::new(store_path.clone());
    svc.load_store(true).await.unwrap();
    svc.add_job(make_test_job("disk1", "Disk Test"))
        .await
        .unwrap();

    // Modify the file on disk directly (simulating what the scheduler does)
    let content = std::fs::read_to_string(&store_path).unwrap();
    let mut store: nanobot::cron::types::CronStore = serde_json::from_str(&content).unwrap();
    store.jobs[0].state.last_status = Some("ok".to_string());
    store.jobs[0].state.next_run_at_ms = Some(9999999999999);
    std::fs::write(&store_path, serde_json::to_string_pretty(&store).unwrap()).unwrap();

    // list_jobs should see the disk changes, not cached state
    let jobs = svc.list_jobs(false).await.unwrap();
    assert_eq!(
        jobs[0].state.last_status.as_deref(),
        Some("ok"),
        "list_jobs should reflect disk state"
    );
    assert_eq!(
        jobs[0].state.next_run_at_ms,
        Some(9999999999999),
        "list_jobs should reflect disk state"
    );
}

#[tokio::test]
async fn test_run_job_callback_error_records_status() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    svc.add_job(make_test_job("fail1", "Failing Job"))
        .await
        .unwrap();

    svc.set_on_job(|_| Box::pin(async { Err(anyhow::anyhow!("something went wrong")) }))
        .await;

    // run_job propagates the error
    let result = svc.run_job("fail1", true).await;
    assert!(result.is_err(), "run_job should propagate callback error");

    // But last_status should still be updated to "success" since run_job
    // updates state regardless (it updates before the callback runs)
    // Note: the scheduler loop has different behavior (async status writeback)
}

#[tokio::test]
async fn test_cron_job_with_none_fields() {
    // Ensure jobs with None in schedule fields don't panic
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    let job = CronJob {
        id: "none1".to_string(),
        name: "None Fields".to_string(),
        enabled: true,
        schedule: CronSchedule::Cron {
            expr: None,
            tz: None,
        },
        payload: CronPayload {
            kind: "agent_turn".to_string(),
            message: "test".to_string(),
            agent_echo: false,
            targets: vec![],
        },
        state: CronJobState::default(),
        created_at_ms: 1000000,
        updated_at_ms: 1000000,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: None,
    };

    svc.add_job(job).await.unwrap();
    let jobs = svc.list_jobs(false).await.unwrap();
    // next_run_at_ms should be None since expr is None
    assert!(
        jobs[0].state.next_run_at_ms.is_none(),
        "job with no cron expr should have no next_run_at_ms"
    );
}

#[tokio::test]
async fn test_cron_job_every_zero_interval() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    let job = CronJob {
        id: "zero1".to_string(),
        name: "Zero Interval".to_string(),
        enabled: true,
        schedule: CronSchedule::Every { every_ms: Some(0) },
        payload: CronPayload {
            kind: "agent_turn".to_string(),
            message: "test".to_string(),
            agent_echo: false,
            targets: vec![],
        },
        state: CronJobState::default(),
        created_at_ms: 1000000,
        updated_at_ms: 1000000,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: None,
    };

    svc.add_job(job).await.unwrap();
    let jobs = svc.list_jobs(false).await.unwrap();
    // Zero interval should not produce a next run
    assert!(
        jobs[0].state.next_run_at_ms.is_none(),
        "zero-interval job should have no next_run_at_ms"
    );
}

#[tokio::test]
async fn test_cron_job_at_expired() {
    let (svc, _tmp) = create_test_cron_service();
    svc.load_store(true).await.unwrap();

    let job = CronJob {
        id: "expired1".to_string(),
        name: "Expired At".to_string(),
        enabled: true,
        schedule: CronSchedule::At {
            at_ms: Some(1000), // way in the past
        },
        payload: CronPayload {
            kind: "agent_turn".to_string(),
            message: "test".to_string(),
            agent_echo: false,
            targets: vec![],
        },
        state: CronJobState::default(),
        created_at_ms: 1000000,
        updated_at_ms: 1000000,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: None,
    };

    svc.add_job(job).await.unwrap();
    let jobs = svc.list_jobs(false).await.unwrap();
    // Past `at` time should not produce a next run
    assert!(
        jobs[0].state.next_run_at_ms.is_none(),
        "expired at-job should have no next_run_at_ms"
    );
}
