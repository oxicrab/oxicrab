use super::*;

#[tokio::test]
async fn test_missing_content_param() {
    let tool = MessageTool::new(None);
    let result = tool.execute(serde_json::json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_empty_context_returns_error() {
    let tool = MessageTool::new(None);
    let result = tool
        .execute(serde_json::json!({"content": "hello"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("No target channel"));
}

#[tokio::test]
async fn test_no_send_tx_returns_error() {
    let tool = MessageTool::new(None);
    tool.set_context("telegram", "12345").await;
    let result = tool
        .execute(serde_json::json!({"content": "hello"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("not configured"));
}

#[tokio::test]
async fn test_successful_send() {
    let (tx, mut rx) = mpsc::channel(1);
    let tool = MessageTool::new(Some(Arc::new(tx)));
    tool.set_context("telegram", "12345").await;
    let result = tool
        .execute(serde_json::json!({"content": "hello"}))
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
    tool.set_context("telegram", "12345").await;
    let result = tool
        .execute(serde_json::json!({
            "content": "hello",
            "channel": "discord",
            "chat_id": "99999"
        }))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("discord:99999"));

    let msg = rx.recv().await.unwrap();
    assert_eq!(msg.channel, "discord");
    assert_eq!(msg.chat_id, "99999");
}

#[tokio::test]
async fn test_set_context() {
    let tool = MessageTool::new(None);
    tool.set_context("slack", "C123").await;
    assert_eq!(*tool.default_channel.lock().await, "slack");
    assert_eq!(*tool.default_chat_id.lock().await, "C123");
}
