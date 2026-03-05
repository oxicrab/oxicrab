use super::*;

#[test]
fn test_cron_job_full_roundtrip() {
    let job = CronJob {
        id: "test-job-1".to_string(),
        name: "Test Job".to_string(),
        enabled: true,
        schedule: CronSchedule::Every {
            every_ms: Some(3_600_000),
        },
        payload: CronPayload {
            kind: "agent_turn".to_string(),
            message: "Hello World".to_string(),
            agent_echo: true,
            targets: vec![
                CronTarget {
                    channel: "telegram".to_string(),
                    to: "user123".to_string(),
                },
                CronTarget {
                    channel: "slack".to_string(),
                    to: "U08G6HBC89X".to_string(),
                },
            ],
        },
        state: CronJobState {
            next_run_at_ms: Some(9_999_999_999),
            last_run_at_ms: Some(8_888_888_888),
            last_status: Some("success".to_string()),
            last_error: None,
            run_count: 3,
            last_fired_at_ms: None,
        },
        created_at_ms: 1_234_567_890,
        updated_at_ms: 1_234_567_900,
        delete_after_run: false,
        expires_at_ms: Some(9_999_999_999_999),
        max_runs: Some(10),
        cooldown_secs: None,
        max_concurrent: None,
    };

    let json = serde_json::to_string(&job).unwrap();
    let deserialized: CronJob = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.id, "test-job-1");
    assert_eq!(deserialized.name, "Test Job");
    assert!(deserialized.enabled);
    assert_eq!(deserialized.payload.kind, "agent_turn");
    assert_eq!(deserialized.payload.message, "Hello World");
    assert!(deserialized.payload.agent_echo);
    assert_eq!(deserialized.payload.targets.len(), 2);
    assert_eq!(deserialized.payload.targets[0].channel, "telegram");
    assert_eq!(deserialized.payload.targets[0].to, "user123");
    assert_eq!(deserialized.payload.targets[1].channel, "slack");
    assert_eq!(deserialized.payload.targets[1].to, "U08G6HBC89X");
    assert_eq!(deserialized.state.next_run_at_ms, Some(9_999_999_999));
    assert_eq!(deserialized.state.last_run_at_ms, Some(8_888_888_888));
    assert_eq!(deserialized.state.last_status, Some("success".to_string()));
    assert_eq!(deserialized.state.last_error, None);
    assert_eq!(deserialized.created_at_ms, 1_234_567_890);
    assert_eq!(deserialized.updated_at_ms, 1_234_567_900);
    assert!(!deserialized.delete_after_run);
    assert_eq!(deserialized.expires_at_ms, Some(9_999_999_999_999));
    assert_eq!(deserialized.max_runs, Some(10));
    assert_eq!(deserialized.state.run_count, 3);
}

#[test]
fn test_backward_compat_missing_new_fields() {
    // Jobs created before expires_at/max_runs existed should deserialize fine
    let json = r#"{
        "id": "old-job",
        "name": "Old Job",
        "enabled": true,
        "schedule": {"kind": "every", "everyMs": 60000},
        "payload": {"kind": "echo", "message": "ping", "agentEcho": false, "targets": []},
        "state": {"nextRunAtMs": null, "lastRunAtMs": null, "lastStatus": null, "lastError": null},
        "createdAtMs": 1000,
        "updatedAtMs": 1000,
        "deleteAfterRun": false
    }"#;
    let job: CronJob = serde_json::from_str(json).unwrap();
    assert_eq!(job.expires_at_ms, None);
    assert_eq!(job.max_runs, None);
    assert_eq!(job.state.run_count, 0);
}

#[test]
fn test_expires_at_and_max_runs_omitted_from_json_when_none() {
    let job = CronJob {
        id: "no-limits".to_string(),
        name: "No Limits".to_string(),
        enabled: true,
        schedule: CronSchedule::Every {
            every_ms: Some(1000),
        },
        payload: CronPayload {
            kind: "echo".to_string(),
            message: "hi".to_string(),
            agent_echo: false,
            targets: vec![],
        },
        state: CronJobState::default(),
        created_at_ms: 1000,
        updated_at_ms: 1000,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: None,
        cooldown_secs: None,
        max_concurrent: None,
    };
    let json = serde_json::to_string(&job).unwrap();
    assert!(
        !json.contains("expiresAtMs"),
        "should skip None expires_at_ms"
    );
    assert!(!json.contains("maxRuns"), "should skip None max_runs");
}

#[test]
fn test_cron_schedule_cron_missing_tz() {
    let schedule = CronSchedule::Cron {
        expr: Some("0 0 * * *".to_string()),
        tz: None,
    };
    let json = serde_json::to_string(&schedule).unwrap();
    let deserialized: CronSchedule = serde_json::from_str(&json).unwrap();

    match deserialized {
        CronSchedule::Cron { expr, tz } => {
            assert_eq!(expr, Some("0 0 * * *".to_string()));
            assert_eq!(tz, None);
        }
        _ => panic!("Expected Cron variant"),
    }
}
