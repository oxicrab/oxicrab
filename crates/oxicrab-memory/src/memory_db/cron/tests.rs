use super::super::MemoryDB;
use oxicrab_core::cron_types::{
    CronJob, CronJobState, CronPayload, CronSchedule, CronTarget, UpdateJobParams,
};

fn make_test_job(id: &str, name: &str, schedule: CronSchedule) -> CronJob {
    CronJob {
        id: id.to_string(),
        name: name.to_string(),
        enabled: true,
        schedule,
        payload: CronPayload {
            kind: "agent_turn".to_string(),
            message: "hello world".to_string(),
            agent_echo: true,
            targets: vec![CronTarget {
                channel: "slack".to_string(),
                to: "C123".to_string(),
            }],
        },
        state: CronJobState::default(),
        created_at_ms: 1000,
        updated_at_ms: 1000,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: None,
        cooldown_secs: None,
        max_concurrent: None,
    }
}

#[test]
fn test_insert_and_list_cron_job() {
    let db = MemoryDB::new(":memory:").unwrap();
    let job = make_test_job(
        "job-1",
        "daily check",
        CronSchedule::Cron {
            expr: Some("0 9 * * *".to_string()),
            tz: Some("UTC".to_string()),
        },
    );

    db.insert_cron_job(&job).unwrap();

    let jobs = db.list_cron_jobs(true).unwrap();
    assert_eq!(jobs.len(), 1);

    let got = &jobs[0];
    assert_eq!(got.id, "job-1");
    assert_eq!(got.name, "daily check");
    assert!(got.enabled);
    assert_eq!(got.payload.kind, "agent_turn");
    assert_eq!(got.payload.message, "hello world");
    assert!(got.payload.agent_echo);
    assert_eq!(got.payload.targets.len(), 1);
    assert_eq!(got.payload.targets[0].channel, "slack");
    assert_eq!(got.payload.targets[0].to, "C123");
    assert_eq!(got.created_at_ms, 1000);
    assert_eq!(got.updated_at_ms, 1000);

    if let CronSchedule::Cron { expr, tz } = &got.schedule {
        assert_eq!(expr.as_deref(), Some("0 9 * * *"));
        assert_eq!(tz.as_deref(), Some("UTC"));
    } else {
        panic!("expected Cron schedule");
    }
}

#[test]
fn test_delete_cron_job() {
    let db = MemoryDB::new(":memory:").unwrap();
    let job = make_test_job(
        "job-del",
        "to delete",
        CronSchedule::Every {
            every_ms: Some(60000),
        },
    );
    db.insert_cron_job(&job).unwrap();

    assert!(db.delete_cron_job("job-del").unwrap());
    assert!(!db.delete_cron_job("job-del").unwrap());

    let jobs = db.list_cron_jobs(true).unwrap();
    assert!(jobs.is_empty());
}

#[test]
fn test_get_cron_job() {
    let db = MemoryDB::new(":memory:").unwrap();
    let job = make_test_job(
        "job-get",
        "get me",
        CronSchedule::At {
            at_ms: Some(999_999),
        },
    );
    db.insert_cron_job(&job).unwrap();

    let got = db.get_cron_job("job-get").unwrap().unwrap();
    assert_eq!(got.id, "job-get");
    assert_eq!(got.name, "get me");
    assert_eq!(got.payload.targets.len(), 1);

    if let CronSchedule::At { at_ms } = &got.schedule {
        assert_eq!(*at_ms, Some(999_999));
    } else {
        panic!("expected At schedule");
    }

    // Nonexistent returns None
    assert!(db.get_cron_job("no-such-job").unwrap().is_none());
}

#[test]
fn test_list_excludes_disabled() {
    let db = MemoryDB::new(":memory:").unwrap();

    let mut enabled_job = make_test_job(
        "job-e",
        "enabled",
        CronSchedule::Every {
            every_ms: Some(5000),
        },
    );
    enabled_job.enabled = true;
    db.insert_cron_job(&enabled_job).unwrap();

    let mut disabled_job = make_test_job(
        "job-d",
        "disabled",
        CronSchedule::Every {
            every_ms: Some(5000),
        },
    );
    disabled_job.enabled = false;
    db.insert_cron_job(&disabled_job).unwrap();

    // include_disabled=false should only return enabled
    let jobs = db.list_cron_jobs(false).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "job-e");

    // include_disabled=true should return both
    let all = db.list_cron_jobs(true).unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn test_update_cron_job_state() {
    let db = MemoryDB::new(":memory:").unwrap();
    let job = make_test_job(
        "job-state",
        "state test",
        CronSchedule::Every {
            every_ms: Some(1000),
        },
    );
    db.insert_cron_job(&job).unwrap();

    let ok = db
        .update_cron_job_state(
            "job-state",
            Some("success"),
            None,
            5,
            Some(2000),
            Some(1500),
            Some(1500),
            2000,
        )
        .unwrap();
    assert!(ok);

    let got = db.get_cron_job("job-state").unwrap().unwrap();
    assert_eq!(got.state.last_status.as_deref(), Some("success"));
    assert!(got.state.last_error.is_none());
    assert_eq!(got.state.run_count, 5);
    assert_eq!(got.state.next_run_at_ms, Some(2000));
    assert_eq!(got.state.last_run_at_ms, Some(1500));
    assert_eq!(got.state.last_fired_at_ms, Some(1500));
    assert_eq!(got.updated_at_ms, 2000);
}

#[test]
fn test_update_cron_job_enabled() {
    let db = MemoryDB::new(":memory:").unwrap();
    let job = make_test_job(
        "job-en",
        "enable test",
        CronSchedule::Every {
            every_ms: Some(1000),
        },
    );
    db.insert_cron_job(&job).unwrap();

    let ok = db
        .update_cron_job_enabled("job-en", false, None, 3000)
        .unwrap();
    assert!(ok);

    let got = db.get_cron_job("job-en").unwrap().unwrap();
    assert!(!got.enabled);
    assert!(got.state.next_run_at_ms.is_none());
    assert_eq!(got.updated_at_ms, 3000);
}

#[test]
fn test_update_cron_job_partial() {
    let db = MemoryDB::new(":memory:").unwrap();
    let job = make_test_job(
        "job-partial",
        "old name",
        CronSchedule::Every {
            every_ms: Some(1000),
        },
    );
    db.insert_cron_job(&job).unwrap();

    let params = UpdateJobParams {
        name: Some("new name".to_string()),
        ..Default::default()
    };
    let ok = db
        .update_cron_job("job-partial", &params, None, 5000)
        .unwrap();
    assert!(ok);

    let got = db.get_cron_job("job-partial").unwrap().unwrap();
    assert_eq!(got.name, "new name");
    // Message should be unchanged
    assert_eq!(got.payload.message, "hello world");
    assert_eq!(got.updated_at_ms, 5000);
}

#[test]
fn test_update_cron_job_targets() {
    let db = MemoryDB::new(":memory:").unwrap();
    let job = make_test_job(
        "job-targets",
        "targets test",
        CronSchedule::Every {
            every_ms: Some(1000),
        },
    );
    db.insert_cron_job(&job).unwrap();

    // Verify original targets
    let got = db.get_cron_job("job-targets").unwrap().unwrap();
    assert_eq!(got.payload.targets.len(), 1);
    assert_eq!(got.payload.targets[0].channel, "slack");

    // Update targets
    let params = UpdateJobParams {
        targets: Some(vec![
            CronTarget {
                channel: "telegram".to_string(),
                to: "12345".to_string(),
            },
            CronTarget {
                channel: "discord".to_string(),
                to: "67890".to_string(),
            },
        ]),
        ..Default::default()
    };
    db.update_cron_job("job-targets", &params, None, 6000)
        .unwrap();

    let got = db.get_cron_job("job-targets").unwrap().unwrap();
    assert_eq!(got.payload.targets.len(), 2);

    let channels: Vec<&str> = got
        .payload
        .targets
        .iter()
        .map(|t| t.channel.as_str())
        .collect();
    assert!(channels.contains(&"telegram"));
    assert!(channels.contains(&"discord"));
}

#[test]
fn test_count_cron_jobs_by_name() {
    let db = MemoryDB::new(":memory:").unwrap();

    let job1 = make_test_job(
        "job-c1",
        "Daily Report",
        CronSchedule::Every {
            every_ms: Some(1000),
        },
    );
    let job2 = make_test_job(
        "job-c2",
        "daily report",
        CronSchedule::Every {
            every_ms: Some(2000),
        },
    );
    let job3 = make_test_job(
        "job-c3",
        "other job",
        CronSchedule::Every {
            every_ms: Some(3000),
        },
    );
    db.insert_cron_job(&job1).unwrap();
    db.insert_cron_job(&job2).unwrap();
    db.insert_cron_job(&job3).unwrap();

    assert_eq!(db.count_cron_jobs_by_name("daily report").unwrap(), 2);
    assert_eq!(db.count_cron_jobs_by_name("DAILY REPORT").unwrap(), 2);
    assert_eq!(db.count_cron_jobs_by_name("other job").unwrap(), 1);
    assert_eq!(db.count_cron_jobs_by_name("nonexistent").unwrap(), 0);
}

#[test]
fn test_prune_disabled_cron_jobs() {
    let db = MemoryDB::new(":memory:").unwrap();

    // Old disabled job (should be pruned)
    let mut old = make_test_job(
        "old-disabled",
        "old",
        CronSchedule::Every {
            every_ms: Some(1000),
        },
    );
    old.enabled = false;
    old.updated_at_ms = 100;
    db.insert_cron_job(&old).unwrap();

    // Recent disabled job (should survive)
    let mut recent = make_test_job(
        "recent-disabled",
        "recent",
        CronSchedule::Every {
            every_ms: Some(1000),
        },
    );
    recent.enabled = false;
    recent.updated_at_ms = 9000;
    db.insert_cron_job(&recent).unwrap();

    // Enabled job (should survive)
    let mut enabled = make_test_job(
        "enabled-job",
        "enabled",
        CronSchedule::Every {
            every_ms: Some(1000),
        },
    );
    enabled.updated_at_ms = 100;
    db.insert_cron_job(&enabled).unwrap();

    let pruned = db.prune_disabled_cron_jobs(5000).unwrap();
    assert_eq!(pruned, 1);

    let all = db.list_cron_jobs(true).unwrap();
    assert_eq!(all.len(), 2);
    let ids: Vec<&str> = all.iter().map(|j| j.id.as_str()).collect();
    assert!(ids.contains(&"recent-disabled"));
    assert!(ids.contains(&"enabled-job"));
}

#[test]
fn test_recover_running_cron_jobs() {
    let db = MemoryDB::new(":memory:").unwrap();

    let mut running = make_test_job(
        "job-run",
        "running",
        CronSchedule::Every {
            every_ms: Some(1000),
        },
    );
    running.state.last_status = Some("running".to_string());
    db.insert_cron_job(&running).unwrap();

    let mut ok = make_test_job(
        "job-ok",
        "ok",
        CronSchedule::Every {
            every_ms: Some(1000),
        },
    );
    ok.state.last_status = Some("success".to_string());
    db.insert_cron_job(&ok).unwrap();

    let recovered = db.recover_running_cron_jobs().unwrap();
    assert_eq!(recovered, 1);

    let got = db.get_cron_job("job-run").unwrap().unwrap();
    assert_eq!(got.state.last_status.as_deref(), Some("interrupted"));
    assert_eq!(
        got.state.last_error.as_deref(),
        Some("process restarted while job was running")
    );

    // The "success" job should be unchanged
    let ok_got = db.get_cron_job("job-ok").unwrap().unwrap();
    assert_eq!(ok_got.state.last_status.as_deref(), Some("success"));
}

#[test]
fn test_fire_cron_job_increments_run_count() {
    let db = MemoryDB::new(":memory:").unwrap();
    let job = make_test_job(
        "job-fire",
        "fire test",
        CronSchedule::Every {
            every_ms: Some(5000),
        },
    );
    db.insert_cron_job(&job).unwrap();

    // Initial run_count is 0
    let got = db.get_cron_job("job-fire").unwrap().unwrap();
    assert_eq!(got.state.run_count, 0);

    // Fire, complete, fire again (atomic claiming requires status != 'running')
    db.fire_cron_job("job-fire", Some(10_000), 5000, 5000)
        .unwrap();
    db.update_cron_job_status("job-fire", "success", None)
        .unwrap();
    db.fire_cron_job("job-fire", Some(15_000), 10_000, 10_000)
        .unwrap();

    let got = db.get_cron_job("job-fire").unwrap().unwrap();
    assert_eq!(got.state.run_count, 2);
}

#[test]
fn test_update_status_preserves_run_count() {
    let db = MemoryDB::new(":memory:").unwrap();
    let job = make_test_job(
        "job-status",
        "status test",
        CronSchedule::Every {
            every_ms: Some(5000),
        },
    );
    db.insert_cron_job(&job).unwrap();

    // Fire once to set run_count=1
    db.fire_cron_job("job-status", Some(10_000), 5000, 5000)
        .unwrap();
    let got = db.get_cron_job("job-status").unwrap().unwrap();
    assert_eq!(got.state.run_count, 1);

    // Update status only — run_count must remain 1
    db.update_cron_job_status("job-status", "completed", None)
        .unwrap();

    let got = db.get_cron_job("job-status").unwrap().unwrap();
    assert_eq!(
        got.state.run_count, 1,
        "update_status must not touch run_count"
    );
    assert_eq!(got.state.last_status.as_deref(), Some("completed"));
}
