use super::*;

fn ctx() -> ExecutionContext {
    ExecutionContext {
        channel: "telegram".to_string(),
        chat_id: "x".to_string(),
        context_summary: None,
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn success_path_sets_metadata() {
    let tool = FinishCronTool::new();
    let result = tool
        .execute(
            json!({"summary": "Sent 3 emails, archived 2 threads."}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    let meta = result.metadata.expect("expected metadata");
    let payload = meta.get(FINISH_CRON_META).expect("expected finish payload");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["summary"], "Sent 3 emails, archived 2 threads.");
}

#[tokio::test]
async fn failure_requires_reason() {
    let tool = FinishCronTool::new();
    let result = tool
        .execute(json!({"summary": "didn't work", "success": false}), &ctx())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("'reason' is required"));
}

#[tokio::test]
async fn failure_with_reason_succeeds() {
    let tool = FinishCronTool::new();
    let result = tool
        .execute(
            json!({"summary": "rolled back", "success": false, "reason": "API rate limit"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    let meta = result.metadata.unwrap();
    let payload = &meta[FINISH_CRON_META];
    assert_eq!(payload["success"], false);
    assert_eq!(payload["reason"], "API rate limit");
}

#[tokio::test]
async fn missing_summary_is_error() {
    let tool = FinishCronTool::new();
    let result = tool.execute(json!({}), &ctx()).await.unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn long_summary_truncates() {
    let tool = FinishCronTool::new();
    let big = "x".repeat(2_000);
    let result = tool.execute(json!({"summary": big}), &ctx()).await.unwrap();
    assert!(!result.is_error);
    let payload_summary = result.metadata.unwrap()[FINISH_CRON_META]["summary"]
        .as_str()
        .unwrap()
        .to_string();
    let chars = payload_summary.chars().count();
    // 1000 cap + 1 ellipsis.
    assert_eq!(chars, 1_001);
    assert!(payload_summary.ends_with('…'));
}
