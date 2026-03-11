use oxicrab::cron::event_matcher::EventMatcher;
use oxicrab::cron::service::{CronService, validate_cron_expr};
use oxicrab::cron::types::{CronJob, CronJobState, CronPayload, CronSchedule, CronTarget};
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
        cooldown_secs: None,
        max_concurrent: None,
    }
}

fn create_test_cron_service() -> CronService {
    let db = Arc::new(
        oxicrab::agent::memory::memory_db::MemoryDB::new(":memory:").expect("create test db"),
    );
    CronService::new(db)
}

fn create_test_db_persistent(
    path: &std::path::Path,
) -> Arc<oxicrab::agent::memory::memory_db::MemoryDB> {
    Arc::new(oxicrab::agent::memory::memory_db::MemoryDB::new(path).expect("create test db"))
}

#[tokio::test]
async fn test_cron_add_and_list() {
    let svc = create_test_cron_service();

    let job = make_test_job("job1", "Test Job 1");
    svc.add_job(job).expect("add cron job");

    let jobs = svc.list_jobs(false).expect("list cron jobs");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "job1");
    assert_eq!(jobs[0].name, "Test Job 1");
}

#[tokio::test]
async fn test_cron_add_multiple_and_list() {
    let svc = create_test_cron_service();

    svc.add_job(make_test_job("j1", "Job 1"))
        .expect("add cron job");
    svc.add_job(make_test_job("j2", "Job 2"))
        .expect("add cron job");
    svc.add_job(make_test_job("j3", "Job 3"))
        .expect("add cron job");

    let jobs = svc.list_jobs(false).expect("list cron jobs");
    assert_eq!(jobs.len(), 3);
}

#[tokio::test]
async fn test_cron_remove_job() {
    let svc = create_test_cron_service();

    svc.add_job(make_test_job("job1", "Job 1"))
        .expect("add cron job");
    svc.add_job(make_test_job("job2", "Job 2"))
        .expect("add cron job");

    let removed = svc.remove_job("job1").expect("remove cron job");
    assert!(removed.is_some());
    assert_eq!(removed.unwrap().id, "job1");

    let jobs = svc.list_jobs(true).expect("list cron jobs");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "job2");
}

#[tokio::test]
async fn test_cron_remove_nonexistent() {
    let svc = create_test_cron_service();

    let removed = svc.remove_job("nonexistent").expect("remove cron job");
    assert!(removed.is_none());
}

#[tokio::test]
async fn test_cron_persistence() {
    let tmp = TempDir::new().expect("create temp dir");
    let db_path = tmp.path().join("test.db");

    // Create service, add job, drop service
    {
        let db = create_test_db_persistent(&db_path);
        let svc = CronService::new(db);
        svc.add_job(make_test_job("persist1", "Persistent Job"))
            .expect("add cron job");
    }

    // Re-open DB from same path — job should persist
    let db2 = create_test_db_persistent(&db_path);
    let svc2 = CronService::new(db2);
    let jobs = svc2.list_jobs(true).expect("list cron jobs");

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "persist1");
    assert_eq!(jobs[0].name, "Persistent Job");
}

#[tokio::test]
async fn test_cron_enable_disable() {
    let svc = create_test_cron_service();

    svc.add_job(make_test_job("toggle1", "Toggle Job"))
        .expect("add cron job");

    // Disable the job
    let updated = svc.enable_job("toggle1", false).expect("disable cron job");
    assert!(updated.is_some());
    assert!(!updated.unwrap().enabled);

    // list_jobs(false) = enabled only -> should be empty
    let enabled_jobs = svc.list_jobs(false).expect("list cron jobs");
    assert_eq!(enabled_jobs.len(), 0);

    // list_jobs(true) = include disabled -> should have 1
    let all_jobs = svc.list_jobs(true).expect("list cron jobs");
    assert_eq!(all_jobs.len(), 1);
    assert!(!all_jobs[0].enabled);

    // Re-enable
    let updated = svc.enable_job("toggle1", true).expect("enable cron job");
    assert!(updated.is_some());
    assert!(updated.unwrap().enabled);

    let enabled_jobs = svc.list_jobs(false).expect("list cron jobs");
    assert_eq!(enabled_jobs.len(), 1);
}

#[tokio::test]
async fn test_cron_manual_trigger() {
    let svc = create_test_cron_service();

    svc.add_job(make_test_job("trigger1", "Trigger Job"))
        .expect("add cron job");

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

    let ran = svc.run_job("trigger1", true).await.expect("run cron job");
    assert!(ran.is_some());
    assert_eq!(ran.unwrap(), Some("Job executed".to_string()));
    assert!(*invoked.lock().await);
}

#[tokio::test]
async fn test_cron_run_disabled_job_without_force() {
    let svc = create_test_cron_service();

    svc.add_job(make_test_job("disabled1", "Disabled Job"))
        .expect("add cron job");
    svc.enable_job("disabled1", false)
        .expect("disable cron job");

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
    let ran = svc.run_job("disabled1", false).await.expect("run cron job");
    assert!(ran.is_none());
    assert!(!*invoked.lock().await);

    // With force, it should run even though disabled
    let ran = svc.run_job("disabled1", true).await.expect("run cron job");
    assert!(ran.is_some());
    assert!(*invoked.lock().await);
}

#[tokio::test]
async fn test_cron_update_job() {
    let svc = create_test_cron_service();

    svc.add_job(make_test_job("upd1", "Original Name"))
        .expect("add cron job");

    let params = oxicrab::cron::types::UpdateJobParams {
        name: Some("Updated Name".to_string()),
        message: Some("Updated message".to_string()),
        ..Default::default()
    };

    let updated = svc.update_job("upd1", &params).expect("update cron job");
    assert!(updated.is_some());
    let job = updated.unwrap();
    assert_eq!(job.name, "Updated Name");
    assert_eq!(job.payload.message, "Updated message");
}

#[tokio::test]
async fn test_cron_multi_target_job() {
    let svc = create_test_cron_service();

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
        cooldown_secs: None,
        max_concurrent: None,
    };

    svc.add_job(job).expect("add cron job");

    // Verify persistence
    let jobs = svc.list_jobs(false).expect("list cron jobs");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].payload.targets.len(), 3);
    assert_eq!(jobs[0].payload.targets[0].channel, "slack");
    assert_eq!(jobs[0].payload.targets[1].channel, "discord");
    assert_eq!(jobs[0].payload.targets[2].channel, "telegram");
}

#[tokio::test]
async fn test_cron_update_targets() {
    let svc = create_test_cron_service();

    svc.add_job(make_test_job("upd_targets", "Target Update Test"))
        .expect("add cron job");

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

    let params = oxicrab::cron::types::UpdateJobParams {
        targets: Some(new_targets),
        ..Default::default()
    };

    let updated = svc
        .update_job("upd_targets", &params)
        .expect("update cron job");
    assert!(updated.is_some());
    let job = updated.unwrap();
    assert_eq!(job.payload.targets.len(), 2);
    assert_eq!(job.payload.targets[0].channel, "slack");
    assert_eq!(job.payload.targets[1].channel, "discord");
}

#[tokio::test]
async fn test_cron_add_duplicate_name_deduplicates() {
    let svc = create_test_cron_service();

    svc.add_job(make_test_job("job1", "Morning Briefing"))
        .expect("add cron job");

    // Same name, different case — should auto-deduplicate with suffix
    svc.add_job(make_test_job("job2", "morning briefing"))
        .expect("add cron job");

    let jobs = svc.list_jobs(true).expect("list cron jobs");
    assert_eq!(jobs.len(), 2);
    let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
    assert!(names.contains(&"Morning Briefing"));
    assert!(names.contains(&"morning briefing (2)"));
}

// --- Tests for bugs that were found and fixed ---

#[tokio::test]
async fn test_add_job_computes_next_run_at_ms() {
    let svc = create_test_cron_service();

    // Create a cron job with next_run_at_ms: None
    let mut job = make_test_job("eager1", "Eager Next Run");
    job.schedule = CronSchedule::Cron {
        expr: Some("0 9 * * *".to_string()),
        tz: Some("UTC".to_string()),
    };
    job.state.next_run_at_ms = None;

    svc.add_job(job).expect("add cron job");

    let jobs = svc.list_jobs(false).expect("list cron jobs");
    assert_eq!(jobs.len(), 1);
    assert!(
        jobs[0].state.next_run_at_ms.is_some(),
        "add_job should compute next_run_at_ms eagerly, got None"
    );
}

#[tokio::test]
async fn test_add_job_computes_next_run_for_every() {
    let svc = create_test_cron_service();

    let mut job = make_test_job("every1", "Every Job");
    job.state.next_run_at_ms = None;

    svc.add_job(job).expect("add cron job");

    let jobs = svc.list_jobs(false).expect("list cron jobs");
    assert!(
        jobs[0].state.next_run_at_ms.is_some(),
        "Every schedule should have next_run_at_ms set"
    );
}

#[tokio::test]
async fn test_run_job_sets_next_run_at_ms() {
    let svc = create_test_cron_service();

    let mut job = make_test_job("runnext1", "Run Next Test");
    job.schedule = CronSchedule::Cron {
        expr: Some("30 8 * * *".to_string()),
        tz: Some("UTC".to_string()),
    };
    svc.add_job(job).expect("add cron job");

    svc.set_on_job(|_| Box::pin(async { Ok(Some("done".to_string())) }))
        .await;

    svc.run_job("runnext1", true).await.expect("run cron job");

    let jobs = svc.list_jobs(false).expect("list cron jobs");
    let job = jobs.iter().find(|j| j.id == "runnext1").unwrap();
    assert!(
        job.state.next_run_at_ms.is_some(),
        "run_job should compute next_run_at_ms after execution"
    );
    assert_eq!(job.state.last_status.as_deref(), Some("success"));
}

#[tokio::test]
async fn test_enable_job_computes_next_run() {
    let svc = create_test_cron_service();

    let mut job = make_test_job("enable1", "Enable Test");
    job.schedule = CronSchedule::Cron {
        expr: Some("0 12 * * *".to_string()),
        tz: Some("UTC".to_string()),
    };
    svc.add_job(job).expect("add cron job");

    // Disable — should clear next_run_at_ms
    let disabled = svc
        .enable_job("enable1", false)
        .expect("disable cron job")
        .unwrap();
    assert!(
        disabled.state.next_run_at_ms.is_none(),
        "disabled job should have no next_run_at_ms"
    );

    // Re-enable — should compute next_run_at_ms
    let enabled = svc
        .enable_job("enable1", true)
        .expect("enable cron job")
        .unwrap();
    assert!(
        enabled.state.next_run_at_ms.is_some(),
        "re-enabled job should have next_run_at_ms computed"
    );
}

#[tokio::test]
async fn test_update_job_with_schedule_recomputes_next_run() {
    let svc = create_test_cron_service();

    let mut job = make_test_job("updsched1", "Update Schedule");
    job.schedule = CronSchedule::Every {
        every_ms: Some(60_000),
    };
    svc.add_job(job).expect("add cron job");

    let before = svc.list_jobs(false).expect("list cron jobs");
    let next_before = before[0].state.next_run_at_ms;

    // Update schedule to a different interval
    let params = oxicrab::cron::types::UpdateJobParams {
        schedule: Some(CronSchedule::Every {
            every_ms: Some(120_000),
        }),
        ..Default::default()
    };
    let updated = svc
        .update_job("updsched1", &params)
        .expect("update cron job")
        .unwrap();
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
async fn test_list_jobs_reflects_db_state() {
    let svc = create_test_cron_service();

    svc.add_job(make_test_job("disk1", "Disk Test"))
        .expect("add cron job");

    // Run the job to update its state
    svc.set_on_job(|_| Box::pin(async { Ok(Some("done".to_string())) }))
        .await;
    svc.run_job("disk1", true).await.expect("run cron job");

    // list_jobs should reflect updated state
    let jobs = svc.list_jobs(false).expect("list cron jobs");
    assert_eq!(
        jobs[0].state.last_status.as_deref(),
        Some("success"),
        "list_jobs should reflect DB state"
    );
    assert!(
        jobs[0].state.last_run_at_ms.is_some(),
        "list_jobs should reflect DB state"
    );
}

#[tokio::test]
async fn test_run_job_callback_error_records_status() {
    let svc = create_test_cron_service();

    svc.add_job(make_test_job("fail1", "Failing Job"))
        .expect("add cron job");

    svc.set_on_job(|_| Box::pin(async { Err(anyhow::anyhow!("something went wrong")) }))
        .await;

    // run_job propagates the error
    let result = svc.run_job("fail1", true).await;
    assert!(result.is_err(), "run_job should propagate callback error");
}

#[tokio::test]
async fn test_cron_job_with_none_fields() {
    // Ensure jobs with None in schedule fields don't panic
    let svc = create_test_cron_service();

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
        cooldown_secs: None,
        max_concurrent: None,
    };

    svc.add_job(job).expect("add cron job");
    let jobs = svc.list_jobs(false).expect("list cron jobs");
    // next_run_at_ms should be None since expr is None
    assert!(
        jobs[0].state.next_run_at_ms.is_none(),
        "job with no cron expr should have no next_run_at_ms"
    );
}

#[tokio::test]
async fn test_cron_job_every_zero_interval() {
    let svc = create_test_cron_service();

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
        cooldown_secs: None,
        max_concurrent: None,
    };

    svc.add_job(job).expect("add cron job");
    let jobs = svc.list_jobs(false).expect("list cron jobs");
    // Zero interval should not produce a next run
    assert!(
        jobs[0].state.next_run_at_ms.is_none(),
        "zero-interval job should have no next_run_at_ms"
    );
}

#[tokio::test]
async fn test_cron_job_at_expired() {
    let svc = create_test_cron_service();

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
        cooldown_secs: None,
        max_concurrent: None,
    };

    svc.add_job(job).expect("add cron job");
    let jobs = svc.list_jobs(false).expect("list cron jobs");
    // Past `at` time should not produce a next run
    assert!(
        jobs[0].state.next_run_at_ms.is_none(),
        "expired at-job should have no next_run_at_ms"
    );
}

// --- Event-triggered cron integration tests ---

fn make_event_job(id: &str, pattern: &str, channel: Option<&str>) -> CronJob {
    CronJob {
        id: id.to_string(),
        name: format!("Event {}", id),
        enabled: true,
        schedule: CronSchedule::Event {
            pattern: Some(pattern.to_string()),
            channel: channel.map(str::to_string),
        },
        payload: CronPayload {
            kind: "agent_turn".to_string(),
            message: "event triggered".to_string(),
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
        cooldown_secs: None,
        max_concurrent: None,
    }
}

#[tokio::test]
async fn test_event_job_persistence() {
    let svc = create_test_cron_service();

    let job = make_event_job("evt1", r"(?i)deploy", None);
    svc.add_job(job).expect("add cron job");

    let jobs = svc.list_jobs(false).expect("list cron jobs");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "evt1");

    // Event jobs should have no next_run_at_ms (they fire on message, not schedule)
    assert!(
        jobs[0].state.next_run_at_ms.is_none(),
        "event job should have no next_run_at_ms"
    );
}

#[tokio::test]
async fn test_event_job_roundtrip_serialization() {
    let tmp = TempDir::new().expect("create temp dir");
    let db_path = tmp.path().join("test.db");

    // Write event job, drop service
    {
        let db = create_test_db_persistent(&db_path);
        let svc = CronService::new(db);
        let mut job = make_event_job("evt_ser", r"(?i)help\s+me", Some("slack"));
        job.cooldown_secs = Some(120);
        svc.add_job(job).expect("add cron job");
    }

    // Read back from DB
    let db2 = create_test_db_persistent(&db_path);
    let svc2 = CronService::new(db2);
    let jobs = svc2.list_jobs(true).expect("list cron jobs");
    assert_eq!(jobs.len(), 1);

    let job = &jobs[0];
    assert_eq!(job.id, "evt_ser");
    assert_eq!(job.cooldown_secs, Some(120));
    match &job.schedule {
        CronSchedule::Event { pattern, channel } => {
            assert_eq!(pattern.as_deref(), Some(r"(?i)help\s+me"));
            assert_eq!(channel.as_deref(), Some("slack"));
        }
        other => panic!("expected Event schedule, got {:?}", other),
    }
}

#[tokio::test]
async fn test_event_matcher_from_stored_jobs() {
    let svc = create_test_cron_service();

    // Add a mix of event and regular jobs
    svc.add_job(make_event_job("evt1", r"(?i)deploy", None))
        .expect("add cron job");
    svc.add_job(make_test_job("reg1", "Regular Job"))
        .expect("add cron job");
    svc.add_job(make_event_job("evt2", r"(?i)rollback", Some("slack")))
        .expect("add cron job");

    // Build event matcher from stored jobs
    let jobs = svc.list_jobs(false).expect("list cron jobs");
    let mut matcher = EventMatcher::from_jobs(&jobs);

    // Should only have 2 event matchers (regular job ignored)
    assert!(!matcher.is_empty());

    // "deploy" matches evt1 on any channel
    let hits = matcher.check_message("please deploy to prod", "discord", 1000);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "evt1");

    // "rollback" matches evt2 only on slack
    let hits = matcher.check_message("rollback the release", "slack", 1000);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "evt2");

    // "rollback" on discord should NOT match (channel filter)
    let hits = matcher.check_message("rollback the release", "discord", 1000);
    assert!(hits.is_empty());

    // No match
    let hits = matcher.check_message("hello world", "slack", 1000);
    assert!(hits.is_empty());
}

#[tokio::test]
async fn test_event_trigger_fires_cron_service() {
    let svc = create_test_cron_service();

    svc.add_job(make_event_job("evt_fire", r"(?i)deploy", None))
        .expect("add cron job");

    // Set up a callback to track execution
    let fired_ids = Arc::new(Mutex::new(Vec::<String>::new()));
    let fired_clone = fired_ids.clone();
    svc.set_on_job(move |job| {
        let fired = fired_clone.clone();
        Box::pin(async move {
            fired.lock().await.push(job.id.clone());
            Ok(Some("event handled".to_string()))
        })
    })
    .await;

    // Build matcher from stored jobs
    let jobs = svc.list_jobs(false).expect("list cron jobs");
    let mut matcher = EventMatcher::from_jobs(&jobs);

    // Simulate an incoming message that matches
    let triggered = matcher.check_message("deploy to staging", "slack", 1000);
    assert_eq!(triggered.len(), 1);

    // Fire the matched jobs through cron service (like the agent loop does)
    for job in &triggered {
        svc.run_job(&job.id, true).await.expect("run cron job");
    }

    let fired = fired_ids.lock().await;
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0], "evt_fire");
}

#[tokio::test]
async fn test_event_job_with_cooldown_integration() {
    let svc = create_test_cron_service();

    let mut job = make_event_job("evt_cool", r"(?i)alert", None);
    job.cooldown_secs = Some(60);
    svc.add_job(job).expect("add cron job");

    // Build matcher — initially no cooldown barrier
    let jobs = svc.list_jobs(false).expect("list cron jobs");
    let mut matcher = EventMatcher::from_jobs(&jobs);

    // First message should trigger (no last_fired_at_ms)
    let hits = matcher.check_message("alert: disk full", "slack", 1_000_000);
    assert_eq!(hits.len(), 1);

    // Same matcher tracks last_fired locally — cooldown should now apply
    // Within cooldown (30s later) — should NOT trigger
    let hits = matcher.check_message("alert: disk full", "slack", 1_030_000);
    assert!(hits.is_empty());

    // After cooldown (90s later) — should trigger
    let hits = matcher.check_message("alert: disk full", "slack", 1_090_000);
    assert_eq!(hits.len(), 1);
}

#[tokio::test]
async fn test_event_job_disabled_not_matched() {
    let svc = create_test_cron_service();

    svc.add_job(make_event_job("evt_dis", r"(?i)deploy", None))
        .expect("add cron job");
    svc.enable_job("evt_dis", false).expect("disable cron job");

    let jobs = svc.list_jobs(true).expect("list cron jobs"); // include disabled
    let matcher = EventMatcher::from_jobs(&jobs);

    // Disabled job should not be in the matcher
    assert!(matcher.is_empty());
    let mut matcher = matcher; // need mut for check_message
    let hits = matcher.check_message("deploy now", "slack", 1000);
    assert!(hits.is_empty());
}

// --- Multi-tool cron job patterns ---
//
// These tests verify that the cron service supports realistic multi-tool job
// configurations: daily briefings that use calendar + todoist + weather, deploy
// monitoring, approval workflows, and echo-mode reminders with buttons.

fn make_multi_tool_job(
    id: &str,
    name: &str,
    message: &str,
    schedule: CronSchedule,
    targets: Vec<CronTarget>,
) -> CronJob {
    CronJob {
        id: id.to_string(),
        name: name.to_string(),
        enabled: true,
        schedule,
        payload: CronPayload {
            kind: "agent_turn".to_string(),
            message: message.to_string(),
            agent_echo: true,
            targets,
        },
        state: CronJobState::default(),
        created_at_ms: 1000000,
        updated_at_ms: 1000000,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: None,
        cooldown_secs: None,
        max_concurrent: None,
    }
}

fn make_echo_job(
    id: &str,
    name: &str,
    message: &str,
    schedule: CronSchedule,
    targets: Vec<CronTarget>,
) -> CronJob {
    CronJob {
        id: id.to_string(),
        name: name.to_string(),
        enabled: true,
        schedule,
        payload: CronPayload {
            kind: "echo".to_string(),
            message: message.to_string(),
            agent_echo: false,
            targets,
        },
        state: CronJobState::default(),
        created_at_ms: 1000000,
        updated_at_ms: 1000000,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: None,
        cooldown_secs: None,
        max_concurrent: None,
    }
}

fn slack_target() -> CronTarget {
    CronTarget {
        channel: "slack".to_string(),
        to: "U08G6HBC89X".to_string(),
    }
}

fn multi_channel_targets() -> Vec<CronTarget> {
    vec![
        CronTarget {
            channel: "slack".to_string(),
            to: "U08G6HBC89X".to_string(),
        },
        CronTarget {
            channel: "discord".to_string(),
            to: "123456789".to_string(),
        },
    ]
}

/// Daily briefing: agent fetches calendar + todoist + weather and summarizes.
#[tokio::test]
async fn test_cron_daily_briefing_pattern() {
    let svc = create_test_cron_service();

    let job = make_multi_tool_job(
        "briefing",
        "Daily Briefing",
        "Give me a morning briefing: check my Google Calendar for today's events, \
         list my Todoist tasks due today, and get the current weather for New York. \
         Format it with tables and use the add_buttons tool to add a 'Snooze 30m' \
         and 'Mark all done' button.",
        CronSchedule::Cron {
            expr: Some("0 8 * * 1-5".to_string()),
            tz: Some("America/New_York".to_string()),
        },
        multi_channel_targets(),
    );

    svc.add_job(job).expect("add briefing job");

    let jobs = svc.list_jobs(false).expect("list jobs");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].payload.targets.len(), 2);
    assert!(jobs[0].payload.message.contains("Google Calendar"));
    assert!(jobs[0].payload.message.contains("Todoist"));
    assert!(jobs[0].payload.message.contains("weather"));
    assert!(jobs[0].payload.message.contains("add_buttons"));
    assert!(jobs[0].payload.agent_echo);
    assert!(jobs[0].state.next_run_at_ms.is_some());
}

/// Deploy monitor: event-triggered job that runs on "deploy" messages and
/// uses github tool to check workflow status.
#[tokio::test]
async fn test_cron_deploy_monitor_pattern() {
    let svc = create_test_cron_service();

    let mut job = make_multi_tool_job(
        "deploy_monitor",
        "Deploy Monitor",
        "A deploy was just mentioned. Use the github tool to check the latest \
         workflow runs for the main repo. If any are failing, use add_buttons \
         to offer 'Retry workflow' and 'View logs' options.",
        CronSchedule::Event {
            pattern: Some(r"(?i)\bdeploy(ed|ing|ment)?\b".to_string()),
            channel: None,
        },
        vec![slack_target()],
    );
    job.cooldown_secs = Some(300); // 5 min cooldown to avoid spam

    svc.add_job(job).expect("add deploy monitor");

    let jobs = svc.list_jobs(false).expect("list jobs");
    let job = &jobs[0];
    assert_eq!(job.cooldown_secs, Some(300));
    assert!(job.payload.message.contains("github"));
    assert!(job.payload.message.contains("add_buttons"));
    match &job.schedule {
        CronSchedule::Event { pattern, channel } => {
            assert!(pattern.is_some());
            assert!(channel.is_none()); // fires on any channel
        }
        _ => panic!("expected Event schedule"),
    }
}

/// Email digest: hourly job that summarizes unread emails.
#[tokio::test]
async fn test_cron_email_digest_pattern() {
    let svc = create_test_cron_service();

    let mut job = make_multi_tool_job(
        "email_digest",
        "Email Digest",
        "Check Gmail for unread messages from the last hour. Summarize the \
         important ones (skip newsletters and automated notifications). If there \
         are any that need a reply, use add_buttons with 'Draft reply' and \
         'Mark read' options.",
        CronSchedule::Every {
            every_ms: Some(3_600_000),
        },
        vec![slack_target()],
    );
    job.max_runs = Some(24); // run 24 times then disable (1 day)

    svc.add_job(job).expect("add email digest");

    let jobs = svc.list_jobs(false).expect("list jobs");
    assert_eq!(jobs[0].max_runs, Some(24));
    assert!(jobs[0].payload.message.contains("Gmail"));
}

/// Echo-mode reminder: no LLM call, just deliver the text directly.
#[tokio::test]
async fn test_cron_echo_reminder_pattern() {
    let svc = create_test_cron_service();

    let job = make_echo_job(
        "standup_remind",
        "Standup Reminder",
        ":wave: Standup in 5 minutes! Get your updates ready.",
        CronSchedule::Cron {
            expr: Some("55 8 * * 1-5".to_string()),
            tz: Some("America/New_York".to_string()),
        },
        multi_channel_targets(),
    );

    svc.add_job(job).expect("add echo reminder");

    let jobs = svc.list_jobs(false).expect("list jobs");
    assert_eq!(jobs[0].payload.kind, "echo");
    assert!(!jobs[0].payload.agent_echo); // echo mode doesn't re-echo
    assert_eq!(jobs[0].payload.targets.len(), 2);

    // Echo jobs should execute immediately (no LLM) — verify with callback
    let delivered = Arc::new(Mutex::new(Vec::<String>::new()));
    let delivered_clone = delivered.clone();
    svc.set_on_job(move |job| {
        let delivered = delivered_clone.clone();
        Box::pin(async move {
            delivered.lock().await.push(job.payload.message.clone());
            Ok(Some(job.payload.message.clone()))
        })
    })
    .await;

    svc.run_job("standup_remind", true)
        .await
        .expect("run echo job");
    let msgs = delivered.lock().await;
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].contains("Standup in 5 minutes"));
}

/// One-shot delayed job: sends a follow-up after a meeting.
#[tokio::test]
async fn test_cron_one_shot_followup_pattern() {
    let svc = create_test_cron_service();

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let mut job = make_multi_tool_job(
        "followup",
        "Meeting Followup",
        "The standup meeting just ended. Check my Google Calendar for the meeting \
         that just finished and any action items. Search my Todoist for related \
         tasks. Compose a follow-up summary and use add_buttons with 'Create \
         tasks' and 'Send summary to channel' options.",
        CronSchedule::At {
            at_ms: Some(now_ms + 1_800_000), // 30 min from now
        },
        vec![slack_target()],
    );
    job.delete_after_run = true; // one-shot: auto-delete

    svc.add_job(job).expect("add followup job");

    let jobs = svc.list_jobs(false).expect("list jobs");
    assert!(jobs[0].delete_after_run);
    assert!(jobs[0].payload.message.contains("Google Calendar"));
    assert!(jobs[0].payload.message.contains("Todoist"));
    assert!(jobs[0].payload.message.contains("add_buttons"));
}

/// Callback metadata forwarding: verify that the callback receives
/// the job payload and targets, enabling the executor to forward
/// response metadata (like buttons) to all target channels.
#[tokio::test]
async fn test_cron_callback_receives_full_job_context() {
    let svc = create_test_cron_service();

    let job = make_multi_tool_job(
        "ctx_test",
        "Context Test",
        "Check weather and add buttons",
        CronSchedule::Every {
            every_ms: Some(60_000),
        },
        multi_channel_targets(),
    );
    svc.add_job(job).expect("add job");

    let captured = Arc::new(Mutex::new(Option::<CronJob>::None));
    let captured_clone = captured.clone();
    svc.set_on_job(move |job| {
        let captured = captured_clone.clone();
        Box::pin(async move {
            *captured.lock().await = Some(job);
            Ok(Some("done".to_string()))
        })
    })
    .await;

    svc.run_job("ctx_test", true).await.expect("run job");

    let captured_job = captured.lock().await.clone().expect("job was captured");
    // Verify the callback received the complete job with all targets
    assert_eq!(captured_job.id, "ctx_test");
    assert_eq!(captured_job.payload.targets.len(), 2);
    assert_eq!(captured_job.payload.targets[0].channel, "slack");
    assert_eq!(captured_job.payload.targets[1].channel, "discord");
    assert!(captured_job.payload.agent_echo);
    // This enables the executor to forward response_metadata (buttons)
    // to all targets via OutboundMessage::builder().merge_metadata()
}

/// Expiry: job that auto-disables after a datetime.
#[tokio::test]
async fn test_cron_job_with_expiry() {
    let svc = create_test_cron_service();

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let mut job = make_multi_tool_job(
        "expiring",
        "Expiring Job",
        "Check for new PRs and summarize",
        CronSchedule::Every {
            every_ms: Some(3_600_000),
        },
        vec![slack_target()],
    );
    job.expires_at_ms = Some(now_ms + 86_400_000); // expires in 24h
    job.max_runs = Some(12); // also caps at 12 runs

    svc.add_job(job).expect("add expiring job");

    let jobs = svc.list_jobs(false).expect("list jobs");
    assert!(jobs[0].expires_at_ms.is_some());
    assert_eq!(jobs[0].max_runs, Some(12));
}

/// Run count tracking: verify execution count increments correctly.
/// Note: auto-disable on max_runs happens in the scheduler tick loop,
/// not in run_job() itself — so we test count tracking here.
#[tokio::test]
async fn test_cron_max_runs_tracking() {
    let svc = create_test_cron_service();

    let mut job = make_multi_tool_job(
        "limited",
        "Limited Runs",
        "Send a reminder",
        CronSchedule::Every {
            every_ms: Some(60_000),
        },
        vec![slack_target()],
    );
    job.max_runs = Some(5);

    svc.add_job(job).expect("add limited job");
    svc.set_on_job(|_| Box::pin(async { Ok(Some("done".to_string())) }))
        .await;

    // Run 3 times
    for _ in 0..3 {
        svc.run_job("limited", true).await.expect("run job");
    }

    let jobs = svc.list_jobs(true).expect("list all jobs");
    let job = jobs.iter().find(|j| j.id == "limited").unwrap();
    assert_eq!(job.state.run_count, 3);
    assert_eq!(job.max_runs, Some(5));
    // Job is still enabled — auto-disable runs on the next scheduler tick,
    // not synchronously in run_job(). The EventMatcher also checks max_runs
    // before firing event-triggered jobs.
    assert!(job.enabled);
}
