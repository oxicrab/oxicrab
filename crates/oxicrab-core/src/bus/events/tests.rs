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

#[test]
fn test_merge_metadata_adds_missing_keys() {
    let mut extra = HashMap::new();
    extra.insert(
        meta::BUTTONS.to_string(),
        serde_json::json!([{"id": "ok", "label": "OK"}]),
    );
    extra.insert(
        meta::TOOLS_USED.to_string(),
        serde_json::json!(["cron", "todoist"]),
    );

    let msg = OutboundMessage::builder("slack", "C123", "response")
        .merge_metadata(extra)
        .build();
    assert!(msg.metadata.contains_key(meta::BUTTONS));
    assert!(msg.metadata.contains_key(meta::TOOLS_USED));
}

#[test]
fn test_merge_metadata_preserves_existing_keys() {
    let mut extra = HashMap::new();
    extra.insert(meta::TS.to_string(), serde_json::json!("overwritten"));
    extra.insert(
        meta::BUTTONS.to_string(),
        serde_json::json!([{"id": "new", "label": "New"}]),
    );

    // Inbound TS metadata should NOT be overwritten by merge
    let inbound = InboundMessage::builder("slack", "u1", "C123", "msg")
        .meta(meta::TS, serde_json::json!("original"))
        .build();
    let msg = OutboundMessage::from_inbound(inbound, "reply")
        .merge_metadata(extra)
        .build();
    // TS should keep the original value (from inbound)
    assert_eq!(msg.metadata[meta::TS], serde_json::json!("original"));
    // BUTTONS should be added (new key)
    assert!(msg.metadata.contains_key(meta::BUTTONS));
}

#[test]
fn test_merge_metadata_empty_is_noop() {
    let msg = OutboundMessage::builder("slack", "C123", "text")
        .meta(meta::TS, serde_json::json!("123"))
        .merge_metadata(HashMap::new())
        .build();
    assert_eq!(msg.metadata.len(), 1);
    assert_eq!(msg.metadata[meta::TS], serde_json::json!("123"));
}
