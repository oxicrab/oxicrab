use super::*;

fn make_inbound(channel: &str, chat_id: &str) -> InboundMessage {
    InboundMessage {
        channel: channel.to_string(),
        sender_id: "user1".to_string(),
        chat_id: chat_id.to_string(),
        content: "hello".to_string(),
        timestamp: Utc::now(),
        media: vec![],
        metadata: HashMap::new(),
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
    let msg = OutboundMessage {
        channel: "discord".to_string(),
        chat_id: "general".to_string(),
        content: "reply text".to_string(),
        reply_to: Some("msg123".to_string()),
        media: vec!["image.png".to_string()],
        metadata: HashMap::new(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let deserialized: OutboundMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.channel, "discord");
    assert_eq!(deserialized.reply_to, Some("msg123".to_string()));
    assert_eq!(deserialized.media, vec!["image.png"]);
}
