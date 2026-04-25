use super::*;
use oxicrab_core::tools::base::ExecutionContext;
use std::collections::HashMap;
use tempfile::tempdir;

fn empty_ctx() -> ExecutionContext {
    ExecutionContext {
        channel: "telegram".to_string(),
        chat_id: "abc".to_string(),
        context_summary: None,
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn returns_records_in_window() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("act.ndjson");
    let journal = Arc::new(ActivityJournal::new(path, 200).unwrap());
    journal
        .append("s1", "user", "thirty minutes ago note")
        .await
        .unwrap();

    let tool = QueryActivityTool::new(journal, 60, 1440);
    let result = tool
        .execute(
            json!({"time_expression": "now", "window_minutes": 60}),
            &empty_ctx(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("thirty minutes ago note"));
}

#[tokio::test]
async fn invalid_time_returns_error() {
    let dir = tempdir().unwrap();
    let journal = Arc::new(ActivityJournal::new(dir.path().join("act.ndjson"), 200).unwrap());
    let tool = QueryActivityTool::new(journal, 60, 1440);
    let result = tool
        .execute(json!({"time_expression": "splarflnax"}), &empty_ctx())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("could not parse"));
}

#[tokio::test]
async fn missing_expression_returns_error() {
    let dir = tempdir().unwrap();
    let journal = Arc::new(ActivityJournal::new(dir.path().join("act.ndjson"), 200).unwrap());
    let tool = QueryActivityTool::new(journal, 60, 1440);
    let result = tool.execute(json!({}), &empty_ctx()).await.unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn window_is_clamped() {
    let dir = tempdir().unwrap();
    let journal = Arc::new(ActivityJournal::new(dir.path().join("act.ndjson"), 200).unwrap());
    journal.append("s1", "user", "hi").await.unwrap();
    let tool = QueryActivityTool::new(journal, 60, 120);
    // Asking for 99999 minutes should clamp to 120 (max_window).
    let result = tool
        .execute(
            json!({"time_expression": "now", "window_minutes": 99_999}),
            &empty_ctx(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("(± 120 min)"));
}
