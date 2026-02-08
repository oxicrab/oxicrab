use nanobot::cron::service::CronService;
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
    assert!(ran);
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
    assert!(!ran);
    assert!(!*invoked.lock().await);

    // With force, it should run even though disabled
    let ran = svc.run_job("disabled1", true).await.unwrap();
    assert!(ran);
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
async fn test_cron_migration_from_old_format() {
    let tmp = TempDir::new().expect("Failed to create temp dir");
    let store_path = tmp.path().join("cron_store.json");

    // Write old-format store to disk
    let old_store = r#"{
        "version": 1,
        "jobs": [{
            "id": "old1",
            "name": "Old Format Job",
            "enabled": true,
            "schedule": { "kind": "every", "everyMs": 3600000 },
            "payload": {
                "kind": "agent_turn",
                "message": "Hello",
                "agentEcho": false,
                "channel": "slack",
                "to": "C0AD9B466G5"
            },
            "state": {},
            "createdAtMs": 1000000,
            "updatedAtMs": 1000000,
            "deleteAfterRun": false
        }]
    }"#;
    std::fs::write(&store_path, old_store).unwrap();

    let svc = CronService::new(store_path.clone());
    let store = svc.load_store(true).await.unwrap();

    // Should have migrated channel/to into targets
    assert_eq!(store.jobs.len(), 1);
    assert_eq!(store.jobs[0].payload.targets.len(), 1);
    assert_eq!(store.jobs[0].payload.targets[0].channel, "slack");
    assert_eq!(store.jobs[0].payload.targets[0].to, "C0AD9B466G5");

    // Verify the migrated store was saved to disk
    let content = std::fs::read_to_string(&store_path).unwrap();
    assert!(content.contains("targets"));
}
