use super::*;
use crate::cron::types::{CronJobState, CronPayload, CronTarget};

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
            message: "triggered".to_string(),
            agent_echo: false,
            targets: vec![CronTarget {
                channel: "telegram".to_string(),
                to: "user1".to_string(),
            }],
            origin_metadata: HashMap::new(),
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
fn test_basic_match() {
    let jobs = vec![make_event_job("e1", r"(?i)deploy", None)];
    let mut em = EventMatcher::from_jobs(&jobs);
    let hits = em.check_message("please deploy to prod", "slack", 1000);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "e1");
}

#[test]
fn test_no_match() {
    let jobs = vec![make_event_job("e1", r"(?i)deploy", None)];
    let mut em = EventMatcher::from_jobs(&jobs);
    let hits = em.check_message("hello world", "slack", 1000);
    assert!(hits.is_empty());
}

#[test]
fn test_channel_filter() {
    let jobs = vec![make_event_job("e1", r"deploy", Some("slack"))];
    let mut matcher = EventMatcher::from_jobs(&jobs);

    // Should match on slack
    assert_eq!(matcher.check_message("deploy now", "slack", 1000).len(), 1);
    // Should not match on discord
    assert!(
        matcher
            .check_message("deploy now", "discord", 1000)
            .is_empty()
    );
}

#[test]
fn test_cooldown() {
    let mut job = make_event_job("e1", r"deploy", None);
    job.cooldown_secs = Some(60);
    job.state.last_fired_at_ms = Some(950_000); // fired 50s ago at now_ms=1000000

    let mut matcher = EventMatcher::from_jobs(&[job]);
    // Within cooldown (50s < 60s)
    assert!(
        matcher
            .check_message("deploy now", "slack", 1_000_000)
            .is_empty()
    );
    // After cooldown (70s > 60s)
    assert_eq!(
        matcher
            .check_message("deploy now", "slack", 1_020_000)
            .len(),
        1
    );
}

#[test]
fn test_cooldown_tracks_across_calls() {
    let mut job = make_event_job("e1", r"deploy", None);
    job.cooldown_secs = Some(60);

    let mut matcher = EventMatcher::from_jobs(&[job]);
    // First call fires (no previous last_fired)
    assert_eq!(
        matcher
            .check_message("deploy now", "slack", 1_000_000)
            .len(),
        1
    );
    // Second call within cooldown should NOT fire
    assert!(
        matcher
            .check_message("deploy now", "slack", 1_030_000)
            .is_empty()
    );
    // Third call after cooldown should fire again
    assert_eq!(
        matcher
            .check_message("deploy now", "slack", 1_061_000)
            .len(),
        1
    );
}

#[test]
fn test_expired_job_skipped() {
    let mut job = make_event_job("e1", r"deploy", None);
    job.expires_at_ms = Some(500_000); // expired
    let mut matcher = EventMatcher::from_jobs(&[job]);
    assert!(
        matcher
            .check_message("deploy now", "slack", 1_000_000)
            .is_empty()
    );
}

#[test]
fn test_max_runs_exhausted_skipped() {
    let mut job = make_event_job("e1", r"deploy", None);
    job.max_runs = Some(2);
    job.state.run_count = 2;
    let mut matcher = EventMatcher::from_jobs(&[job]);
    assert!(
        matcher
            .check_message("deploy now", "slack", 1_000_000)
            .is_empty()
    );
}

#[test]
fn test_run_count_incremented_on_fire() {
    let mut job = make_event_job("e1", r"deploy", None);
    job.max_runs = Some(2);
    let mut matcher = EventMatcher::from_jobs(&[job]);
    // First fire
    assert_eq!(
        matcher
            .check_message("deploy now", "slack", 1_000_000)
            .len(),
        1
    );
    // Second fire (should still work, run_count now 1 -> 2 which will be returned but
    // the next call should be blocked)
    assert_eq!(
        matcher
            .check_message("deploy now", "slack", 1_001_000)
            .len(),
        1
    );
    // Third fire should be blocked (run_count=2, max_runs=2)
    assert!(
        matcher
            .check_message("deploy now", "slack", 1_002_000)
            .is_empty()
    );
}

#[test]
fn test_disabled_job_ignored() {
    let mut job = make_event_job("e1", r"deploy", None);
    job.enabled = false;
    let matcher = EventMatcher::from_jobs(&[job]);
    assert!(matcher.is_empty());
}

#[test]
fn test_invalid_regex_skipped() {
    let jobs = vec![
        make_event_job("e1", r"[invalid", None),
        make_event_job("e2", r"valid", None),
    ];
    let matcher = EventMatcher::from_jobs(&jobs);
    // Only the valid one should be registered
    assert_eq!(matcher.matchers.len(), 1);
    assert_eq!(matcher.matchers[0].0, "e2");
}
