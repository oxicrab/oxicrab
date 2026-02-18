use super::*;
use crate::cron::types::{CronJob, CronJobState, CronPayload, CronSchedule};

#[test]
fn test_five_field_cron_needs_normalization() {
    // The `cron` crate requires 6+ fields — raw 5-field expressions fail
    let expr = "0 9 * * *";
    assert!(expr.parse::<Schedule>().is_err());

    // But normalized to 6-field (prepend seconds) it works
    let normalized = format!("0 {}", expr);
    assert!(normalized.parse::<Schedule>().is_ok());
}

#[test]
fn test_compute_next_run_five_field_cron() {
    let schedule = CronSchedule::Cron {
        expr: Some("0 9 * * *".to_string()),
        tz: None,
    };
    let now = now_ms();
    let result = compute_next_run(&schedule, now);
    assert!(
        result.is_some(),
        "compute_next_run should normalize 5-field cron and return Some"
    );
}

#[test]
fn test_compute_next_run_six_field_cron() {
    let schedule = CronSchedule::Cron {
        expr: Some("0 30 8 * * *".to_string()),
        tz: None,
    };
    let result = compute_next_run(&schedule, now_ms());
    assert!(result.is_some(), "6-field cron should work directly");
}

#[test]
fn test_compute_next_run_with_timezone() {
    let schedule = CronSchedule::Cron {
        expr: Some("0 9 * * *".to_string()),
        tz: Some("America/New_York".to_string()),
    };
    let result = compute_next_run(&schedule, now_ms());
    assert!(result.is_some(), "cron with timezone should return Some");
}

#[test]
fn test_compute_next_run_every() {
    let schedule = CronSchedule::Every {
        every_ms: Some(60_000),
    };
    let now = now_ms();
    let result = compute_next_run(&schedule, now);
    assert_eq!(result, Some(now + 60_000));
}

#[test]
fn test_compute_next_run_at_future() {
    let future = now_ms() + 100_000;
    let schedule = CronSchedule::At {
        at_ms: Some(future),
    };
    assert_eq!(compute_next_run(&schedule, now_ms()), Some(future));
}

#[test]
fn test_compute_next_run_at_past() {
    let past = now_ms() - 100_000;
    let schedule = CronSchedule::At { at_ms: Some(past) };
    assert_eq!(compute_next_run(&schedule, now_ms()), None);
}

#[test]
fn test_validate_cron_expr_five_field() {
    let result = validate_cron_expr("0 9 * * *");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "0 0 9 * * *");
}

#[test]
fn test_validate_cron_expr_six_field() {
    let result = validate_cron_expr("0 30 8 * * *");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "0 30 8 * * *");
}

#[test]
fn test_validate_cron_expr_invalid() {
    let result = validate_cron_expr("not a cron");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Invalid cron expression"));
}

#[test]
fn test_detect_system_timezone() {
    let tz = detect_system_timezone();
    // Should succeed on any standard Linux/macOS system
    assert!(tz.is_some(), "should detect system timezone");
    let tz = tz.unwrap();
    assert!(
        tz.contains('/'),
        "timezone should be IANA format like Area/City, got: {}",
        tz
    );
}

#[tokio::test]
async fn test_expired_job_auto_disables() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store_path = tmp.path().join("cron_jobs.json");
    let svc = CronService::new(store_path.clone());

    let now = now_ms();
    let job = CronJob {
        id: "exp-1".to_string(),
        name: "Expired Job".to_string(),
        enabled: true,
        schedule: CronSchedule::Every {
            every_ms: Some(1000),
        },
        payload: CronPayload {
            kind: "echo".to_string(),
            message: "ping".to_string(),
            agent_echo: false,
            targets: vec![],
        },
        state: CronJobState {
            next_run_at_ms: Some(now + 5000),
            ..Default::default()
        },
        created_at_ms: now,
        updated_at_ms: now,
        delete_after_run: false,
        expires_at_ms: Some(now - 1000), // already expired
        max_runs: None,
        cooldown_secs: None,
        max_concurrent: None,
    };

    svc.add_job(job).await.unwrap();

    // Start the service — it should disable the expired job on first tick
    svc.start().await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    svc.stop().await;

    let jobs = svc.list_jobs(true).await.unwrap();
    let j = jobs.iter().find(|j| j.id == "exp-1").unwrap();
    assert!(!j.enabled, "expired job should be disabled");
}

#[tokio::test]
async fn test_max_runs_auto_disables() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store_path = tmp.path().join("cron_jobs.json");
    let svc = CronService::new(store_path.clone());

    let now = now_ms();
    let job = CronJob {
        id: "max-1".to_string(),
        name: "Max Runs Job".to_string(),
        enabled: true,
        schedule: CronSchedule::Every {
            every_ms: Some(1000),
        },
        payload: CronPayload {
            kind: "echo".to_string(),
            message: "ping".to_string(),
            agent_echo: false,
            targets: vec![],
        },
        state: CronJobState {
            next_run_at_ms: Some(now + 5000),
            run_count: 5,
            ..Default::default()
        },
        created_at_ms: now,
        updated_at_ms: now,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: Some(5), // already at max
        cooldown_secs: None,
        max_concurrent: None,
    };

    svc.add_job(job).await.unwrap();

    svc.start().await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    svc.stop().await;

    let jobs = svc.list_jobs(true).await.unwrap();
    let j = jobs.iter().find(|j| j.id == "max-1").unwrap();
    assert!(!j.enabled, "job at max runs should be disabled");
}

#[tokio::test]
async fn test_add_job_deduplicates_names() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store_path = tmp.path().join("cron_jobs.json");
    let svc = CronService::new(store_path.clone());

    let now = now_ms();
    let make_job = |id: &str| CronJob {
        id: id.to_string(),
        name: "Daily Reminder".to_string(),
        enabled: true,
        schedule: CronSchedule::Every {
            every_ms: Some(60_000),
        },
        payload: CronPayload {
            kind: "echo".to_string(),
            message: "ping".to_string(),
            agent_echo: false,
            targets: vec![],
        },
        state: CronJobState::default(),
        created_at_ms: now,
        updated_at_ms: now,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: None,
        cooldown_secs: None,
        max_concurrent: None,
    };

    // First job keeps its name
    svc.add_job(make_job("a1")).await.unwrap();
    // Second job with same name gets "(2)" suffix
    svc.add_job(make_job("a2")).await.unwrap();
    // Third gets "(3)"
    svc.add_job(make_job("a3")).await.unwrap();

    let jobs = svc.list_jobs(false).await.unwrap();
    let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
    assert!(names.contains(&"Daily Reminder"));
    assert!(names.contains(&"Daily Reminder (2)"));
    assert!(names.contains(&"Daily Reminder (3)"));
}

#[tokio::test]
async fn test_run_job_increments_run_count() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store_path = tmp.path().join("cron_jobs.json");
    let svc = CronService::new(store_path.clone());

    let now = now_ms();
    let job = CronJob {
        id: "cnt-1".to_string(),
        name: "Counter Job".to_string(),
        enabled: true,
        schedule: CronSchedule::Every {
            every_ms: Some(60_000),
        },
        payload: CronPayload {
            kind: "echo".to_string(),
            message: "ping".to_string(),
            agent_echo: false,
            targets: vec![],
        },
        state: CronJobState::default(),
        created_at_ms: now,
        updated_at_ms: now,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: None,
        cooldown_secs: None,
        max_concurrent: None,
    };

    svc.add_job(job).await.unwrap();
    svc.set_on_job(|_job| Box::pin(async { Ok(Some("done".to_string())) }))
        .await;

    svc.run_job("cnt-1", false).await.unwrap();
    svc.run_job("cnt-1", false).await.unwrap();

    let jobs = svc.list_jobs(false).await.unwrap();
    let j = jobs.iter().find(|j| j.id == "cnt-1").unwrap();
    assert_eq!(
        j.state.run_count, 2,
        "run_count should be 2 after 2 manual runs"
    );
}
