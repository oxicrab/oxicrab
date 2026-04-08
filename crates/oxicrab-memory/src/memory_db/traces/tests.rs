use super::*;

fn test_db() -> MemoryDB {
    MemoryDB::new(":memory:").expect("in-memory DB")
}

#[test]
fn insert_and_get_trace() {
    let db = test_db();
    let mut trace = CronTrace::new("trace-1".into(), "job-1".into(), "test job".into());
    trace.add_event(TraceEvent::Started {
        message: "hello".into(),
    });
    trace.add_event(TraceEvent::ToolCall {
        name: "cron".into(),
        params_summary: "{}".into(),
    });
    db.insert_cron_trace(&trace).unwrap();

    let loaded = db.get_cron_trace("trace-1").unwrap().unwrap();
    assert_eq!(loaded.id, "trace-1");
    assert_eq!(loaded.job_id, "job-1");
    assert_eq!(loaded.status, "running");
    assert_eq!(loaded.events.len(), 2);
    assert_eq!(loaded.tool_call_count, 1);
}

#[test]
fn update_trace() {
    let db = test_db();
    let mut trace = CronTrace::new("trace-2".into(), "job-2".into(), "test job 2".into());
    db.insert_cron_trace(&trace).unwrap();

    trace.complete(Some("done".into()));
    db.update_cron_trace(&trace).unwrap();

    let loaded = db.get_cron_trace("trace-2").unwrap().unwrap();
    assert_eq!(loaded.status, "completed");
    assert!(loaded.completed_at.is_some());
    assert_eq!(loaded.summary.as_deref(), Some("done"));
}

#[test]
fn list_traces_with_filter() {
    let db = test_db();
    for i in 0..5 {
        let job_id = if i < 3 { "job-a" } else { "job-b" };
        let trace = CronTrace::new(format!("trace-{i}"), job_id.into(), "test".into());
        db.insert_cron_trace(&trace).unwrap();
    }

    let all = db.list_cron_traces(None, 100).unwrap();
    assert_eq!(all.len(), 5);

    let filtered = db.list_cron_traces(Some("job-a"), 100).unwrap();
    assert_eq!(filtered.len(), 3);

    let limited = db.list_cron_traces(None, 2).unwrap();
    assert_eq!(limited.len(), 2);
}

#[test]
fn purge_old_traces() {
    let db = test_db();
    for i in 0..10 {
        let trace = CronTrace::new(format!("trace-{i}"), "job-1".into(), "test".into());
        db.insert_cron_trace(&trace).unwrap();
    }

    let purged = db.purge_old_cron_traces(3).unwrap();
    assert_eq!(purged, 7);

    let remaining = db.list_cron_traces(None, 100).unwrap();
    assert_eq!(remaining.len(), 3);
}

#[test]
fn trace_fail() {
    let db = test_db();
    let mut trace = CronTrace::new("trace-f".into(), "job-f".into(), "failing job".into());
    trace.fail("something went wrong");
    db.insert_cron_trace(&trace).unwrap();

    let loaded = db.get_cron_trace("trace-f").unwrap().unwrap();
    assert_eq!(loaded.status, "failed");
    assert!(loaded.completed_at.is_some());
    assert!(matches!(
        loaded.events.last(),
        Some(TraceEvent::Error { .. })
    ));
}
