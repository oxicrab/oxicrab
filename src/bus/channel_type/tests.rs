use super::*;
use std::str::FromStr;

#[test]
fn test_from_str_returns_err_for_unknown() {
    let result = ChannelType::from_str("unknown");
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "Unknown channel type: unknown");
}

#[test]
fn test_serde_json_roundtrip() {
    let variants = vec![
        ChannelType::Telegram,
        ChannelType::Discord,
        ChannelType::Slack,
        ChannelType::WhatsApp,
        ChannelType::Twilio,
        ChannelType::System,
        ChannelType::Cli,
    ];

    for variant in variants {
        let json = serde_json::to_string(&variant).unwrap();
        let deserialized: ChannelType = serde_json::from_str(&json).unwrap();
        assert_eq!(variant, deserialized);
    }
}

#[test]
fn test_as_str_all_variants() {
    assert_eq!(ChannelType::Telegram.as_str(), "telegram");
    assert_eq!(ChannelType::Discord.as_str(), "discord");
    assert_eq!(ChannelType::Slack.as_str(), "slack");
    assert_eq!(ChannelType::WhatsApp.as_str(), "whatsapp");
    assert_eq!(ChannelType::Twilio.as_str(), "twilio");
    assert_eq!(ChannelType::System.as_str(), "system");
    assert_eq!(ChannelType::Cli.as_str(), "cli");
}

#[test]
fn test_from_str_all_variants() {
    assert_eq!(
        ChannelType::from_str("telegram").unwrap(),
        ChannelType::Telegram
    );
    assert_eq!(
        ChannelType::from_str("discord").unwrap(),
        ChannelType::Discord
    );
    assert_eq!(ChannelType::from_str("slack").unwrap(), ChannelType::Slack);
    assert_eq!(
        ChannelType::from_str("whatsapp").unwrap(),
        ChannelType::WhatsApp
    );
    assert_eq!(
        ChannelType::from_str("twilio").unwrap(),
        ChannelType::Twilio
    );
    assert_eq!(
        ChannelType::from_str("system").unwrap(),
        ChannelType::System
    );
    assert_eq!(ChannelType::from_str("cli").unwrap(), ChannelType::Cli);
}

#[test]
fn test_display_matches_as_str() {
    let variants = [
        ChannelType::Telegram,
        ChannelType::Discord,
        ChannelType::Slack,
        ChannelType::WhatsApp,
        ChannelType::Twilio,
        ChannelType::System,
        ChannelType::Cli,
    ];
    for v in &variants {
        assert_eq!(format!("{v}"), v.as_str());
    }
}

#[test]
fn test_into_string() {
    let s: String = ChannelType::Discord.into();
    assert_eq!(s, "discord");
}

#[test]
fn test_from_str_roundtrip_via_as_str() {
    let variants = [
        ChannelType::Telegram,
        ChannelType::Discord,
        ChannelType::Slack,
        ChannelType::WhatsApp,
        ChannelType::Twilio,
        ChannelType::System,
        ChannelType::Cli,
    ];
    for v in &variants {
        let roundtripped = ChannelType::from_str(v.as_str()).unwrap();
        assert_eq!(&roundtripped, v);
    }
}

#[test]
fn test_from_str_case_sensitive() {
    assert!(ChannelType::from_str("Telegram").is_err());
    assert!(ChannelType::from_str("DISCORD").is_err());
}
