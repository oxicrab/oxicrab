use super::*;

#[test]
fn test_cron_job_full_roundtrip() {
    let job = CronJob {
        id: "test-job-1".to_string(),
        name: "Test Job".to_string(),
        enabled: true,
        schedule: CronSchedule::Every {
            every_ms: Some(3600000),
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
            next_run_at_ms: Some(9999999999),
            last_run_at_ms: Some(8888888888),
            last_status: Some("success".to_string()),
            last_error: None,
        },
        created_at_ms: 1234567890,
        updated_at_ms: 1234567900,
        delete_after_run: false,
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
    assert_eq!(deserialized.state.next_run_at_ms, Some(9999999999));
    assert_eq!(deserialized.state.last_run_at_ms, Some(8888888888));
    assert_eq!(deserialized.state.last_status, Some("success".to_string()));
    assert_eq!(deserialized.state.last_error, None);
    assert_eq!(deserialized.created_at_ms, 1234567890);
    assert_eq!(deserialized.updated_at_ms, 1234567900);
    assert!(!deserialized.delete_after_run);
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
