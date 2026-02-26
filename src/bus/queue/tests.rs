use super::*;
use chrono::Utc;
use std::collections::HashMap;

fn make_inbound(channel: &str, sender_id: &str) -> InboundMessage {
    InboundMessage {
        channel: channel.to_string(),
        sender_id: sender_id.to_string(),
        chat_id: "chat1".to_string(),
        content: "hello".to_string(),
        timestamp: Utc::now(),
        media: vec![],
        metadata: HashMap::new(),
    }
}

fn make_outbound(channel: &str, chat_id: &str, content: &str) -> OutboundMessage {
    OutboundMessage {
        channel: channel.to_string(),
        chat_id: chat_id.to_string(),
        content: content.to_string(),
        reply_to: None,
        media: vec![],
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_publish_inbound_succeeds() {
    let mut bus = MessageBus::default();
    let mut rx = bus.take_inbound_rx().unwrap();

    let msg = make_inbound("test", "user1");
    bus.publish_inbound(msg).await.unwrap();

    let received = rx.try_recv().unwrap();
    assert_eq!(received.channel, "test");
    assert_eq!(received.sender_id, "user1");
}

#[tokio::test]
async fn test_inbound_rate_limit_enforced() {
    let mut bus = MessageBus::new(2, 60.0, 100, 100);
    let _rx = bus.take_inbound_rx().unwrap();

    // First two should succeed
    bus.publish_inbound(make_inbound("ch", "sender1"))
        .await
        .unwrap();
    bus.publish_inbound(make_inbound("ch", "sender1"))
        .await
        .unwrap();

    // Third should fail — rate limit of 2 per window
    let result = bus.publish_inbound(make_inbound("ch", "sender1")).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Rate limit exceeded")
    );
}

#[tokio::test]
async fn test_inbound_rate_limit_per_sender() {
    let mut bus = MessageBus::new(2, 60.0, 100, 100);
    let _rx = bus.take_inbound_rx().unwrap();

    // sender1 hits limit
    bus.publish_inbound(make_inbound("ch", "sender1"))
        .await
        .unwrap();
    bus.publish_inbound(make_inbound("ch", "sender1"))
        .await
        .unwrap();

    // sender2 should still be able to publish (separate rate bucket)
    bus.publish_inbound(make_inbound("ch", "sender2"))
        .await
        .unwrap();
}

#[tokio::test]
async fn test_outbound_rate_limit_enforced() {
    let mut bus = MessageBus::new(30, 60.0, 100, 100);
    // Override the outbound limit to a small number for testing
    bus.outbound_rate_limit = 2;
    let _rx = bus.take_outbound_rx().unwrap();

    bus.publish_outbound(make_outbound("ch", "dest1", "msg1"))
        .await
        .unwrap();
    bus.publish_outbound(make_outbound("ch", "dest1", "msg2"))
        .await
        .unwrap();

    let result = bus
        .publish_outbound(make_outbound("ch", "dest1", "msg3"))
        .await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Outbound rate limit exceeded")
    );
}

#[tokio::test]
async fn test_outbound_leak_detection_redacts() {
    let mut bus = MessageBus::default();
    let mut rx = bus.take_outbound_rx().unwrap();

    // add_known_secrets requires values >= 10 chars
    let secret = "sk-secret-1234567890";
    bus.add_known_secrets(&[("api_key", secret)]);

    let msg = make_outbound("ch", "dest", &format!("the key is {}", secret));
    bus.publish_outbound(msg).await.unwrap();

    let received = rx.try_recv().unwrap();
    // The secret should be redacted in the received message
    assert!(
        !received.content.contains(secret),
        "secret should be redacted, got: {}",
        received.content
    );
}

#[tokio::test]
async fn test_default_creates_valid_bus() {
    let mut bus = MessageBus::default();
    assert!(bus.take_inbound_rx().is_some());
    assert!(bus.take_outbound_rx().is_some());
}

#[tokio::test]
async fn test_take_rx_returns_none_second_time() {
    let mut bus = MessageBus::default();

    assert!(bus.take_inbound_rx().is_some());
    assert!(bus.take_inbound_rx().is_none());

    assert!(bus.take_outbound_rx().is_some());
    assert!(bus.take_outbound_rx().is_none());
}
