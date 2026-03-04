use super::*;

fn make_inbound(channel: &str, chat_id: &str) -> InboundMessage {
    InboundMessage {
        channel: channel.to_string(),
        sender_id: "user1".to_string(),
        chat_id: chat_id.to_string(),
        content: "hello".to_string(),
        timestamp: Utc::now(),
        ..Default::default()
    }
}

#[test]
fn test_session_key_format() {
    let msg = make_inbound("telegram", "12345");
    assert_eq!(msg.session_key(), "telegram:12345");
}

#[test]
fn test_session_key_different_channels() {
    let a = make_inbound("discord", "abc");
    let b = make_inbound("slack", "abc");
    assert_ne!(a.session_key(), b.session_key());
}

#[test]
fn test_session_key_different_chats() {
    let a = make_inbound("telegram", "111");
    let b = make_inbound("telegram", "222");
    assert_ne!(a.session_key(), b.session_key());
}

#[test]
fn test_session_key_same_inputs() {
    let a = make_inbound("slack", "C123");
    let b = make_inbound("slack", "C123");
    assert_eq!(a.session_key(), b.session_key());
}

#[test]
fn test_inbound_serde_roundtrip() {
    let msg = make_inbound("telegram", "42");
    let json = serde_json::to_string(&msg).unwrap();
    let deserialized: InboundMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.channel, "telegram");
    assert_eq!(deserialized.chat_id, "42");
    assert_eq!(deserialized.content, "hello");
}

#[test]
fn test_outbound_serde_roundtrip() {
    let msg = OutboundMessage::builder("discord", "general", "reply text")
        .reply_to("msg123")
        .media(vec!["image.png".to_string()])
        .build();
    let json = serde_json::to_string(&msg).unwrap();
    let deserialized: OutboundMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.channel, "discord");
    assert_eq!(deserialized.reply_to, Some("msg123".to_string()));
    assert_eq!(deserialized.media, vec!["image.png"]);
}

#[test]
fn test_inbound_builder_defaults() {
    let before = Utc::now();
    let msg = InboundMessage::builder("telegram", "user1", "chat1", "hi").build();
    assert_eq!(msg.channel, "telegram");
    assert_eq!(msg.sender_id, "user1");
    assert_eq!(msg.chat_id, "chat1");
    assert_eq!(msg.content, "hi");
    assert!(msg.timestamp >= before);
    assert!(msg.media.is_empty());
    assert!(msg.metadata.is_empty());
}

#[test]
fn test_inbound_builder_is_group() {
    let msg = InboundMessage::builder("discord", "u1", "c1", "hey")
        .is_group(true)
        .build();
    assert_eq!(
        msg.metadata.get(meta::IS_GROUP),
        Some(&serde_json::Value::Bool(true))
    );
}

#[test]
fn test_inbound_builder_meta_chaining() {
    let msg = InboundMessage::builder("slack", "u1", "c1", "msg")
        .meta(meta::TS, serde_json::json!("123.456"))
        .meta(meta::THREAD_TS, serde_json::json!("100.000"))
        .build();
    assert_eq!(msg.metadata.len(), 2);
    assert_eq!(msg.metadata[meta::TS], serde_json::json!("123.456"));
    assert_eq!(msg.metadata[meta::THREAD_TS], serde_json::json!("100.000"));
}

#[test]
fn test_outbound_from_inbound() {
    let inbound = InboundMessage::builder("telegram", "user1", "chat42", "question")
        .meta(meta::TS, serde_json::json!("999"))
        .build();
    let outbound = OutboundMessage::from_inbound(inbound, "answer").build();
    assert_eq!(outbound.channel, "telegram");
    assert_eq!(outbound.chat_id, "chat42");
    assert_eq!(outbound.content, "answer");
    assert_eq!(outbound.metadata[meta::TS], serde_json::json!("999"));
    assert!(outbound.reply_to.is_none());
    assert!(outbound.media.is_empty());
}

#[test]
fn test_outbound_builder_defaults() {
    let msg = OutboundMessage::builder("discord", "general", "hello").build();
    assert_eq!(msg.channel, "discord");
    assert_eq!(msg.chat_id, "general");
    assert_eq!(msg.content, "hello");
    assert!(msg.reply_to.is_none());
    assert!(msg.media.is_empty());
    assert!(msg.metadata.is_empty());
}
