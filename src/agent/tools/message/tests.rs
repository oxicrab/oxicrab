use super::*;
use crate::agent::tools::base::ExecutionContext;

fn test_ctx(channel: &str, chat_id: &str) -> ExecutionContext {
    ExecutionContext {
        channel: channel.to_string(),
        chat_id: chat_id.to_string(),
        context_summary: None,
    }
}

#[tokio::test]
async fn test_missing_content_param() {
    let tool = MessageTool::new(None);
    let result = tool
        .execute(serde_json::json!({}), &ExecutionContext::default())
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_empty_context_returns_error() {
    let tool = MessageTool::new(None);
    let result = tool
        .execute(
            serde_json::json!({"content": "hello"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("No target channel"));
}

#[tokio::test]
async fn test_no_send_tx_returns_error() {
    let tool = MessageTool::new(None);
    let ctx = test_ctx("telegram", "12345");
    let result = tool
        .execute(serde_json::json!({"content": "hello"}), &ctx)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("not configured"));
}

#[tokio::test]
async fn test_successful_send() {
    let (tx, mut rx) = mpsc::channel(1);
    let tool = MessageTool::new(Some(Arc::new(tx)));
    let ctx = test_ctx("telegram", "12345");
    let result = tool
        .execute(serde_json::json!({"content": "hello"}), &ctx)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Message sent"));

    let msg = rx.recv().await.unwrap();
    assert_eq!(msg.channel, "telegram");
    assert_eq!(msg.chat_id, "12345");
    assert_eq!(msg.content, "hello");
}

#[tokio::test]
async fn test_explicit_channel_overrides_default() {
    let (tx, mut rx) = mpsc::channel(1);
    let tool = MessageTool::new(Some(Arc::new(tx)));
    let ctx = test_ctx("telegram", "12345");
    let result = tool
        .execute(
            serde_json::json!({
                "content": "hello",
                "channel": "discord",
                "chat_id": "99999"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("discord:99999"));

    let msg = rx.recv().await.unwrap();
    assert_eq!(msg.channel, "discord");
    assert_eq!(msg.chat_id, "99999");
}
