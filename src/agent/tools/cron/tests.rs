use super::*;
use crate::config::{
    ChannelsConfig, DiscordConfig, SlackConfig, TelegramConfig, TwilioConfig, WhatsAppConfig,
};

fn make_test_channels_config() -> ChannelsConfig {
    ChannelsConfig {
        slack: SlackConfig {
            enabled: true,
            bot_token: String::new(),
            app_token: String::new(),
            allow_from: vec!["U08G6HBC89X".to_string()],
            dm_policy: crate::config::DmPolicy::Allowlist,
        },
        discord: DiscordConfig {
            enabled: true,
            token: String::new(),
            allow_from: vec!["123456789".to_string()],
            commands: Vec::new(),
            dm_policy: crate::config::DmPolicy::Allowlist,
        },
        telegram: TelegramConfig {
            enabled: true,
            token: String::new(),
            allow_from: vec!["987654321".to_string()],
            dm_policy: crate::config::DmPolicy::Allowlist,
        },
        whatsapp: WhatsAppConfig {
            enabled: true,
            allow_from: vec!["+15551234567".to_string()],
            dm_policy: crate::config::DmPolicy::Allowlist,
        },
        twilio: TwilioConfig::default(),
    }
}

#[test]
fn test_resolve_all_channels() {
    let cfg = make_test_channels_config();
    let targets = resolve_all_channel_targets_from_config(Some(&cfg));
    assert_eq!(targets.len(), 4);
    assert!(
        targets
            .iter()
            .any(|t| t.channel == "slack" && t.to == "U08G6HBC89X")
    );
    assert!(
        targets
            .iter()
            .any(|t| t.channel == "discord" && t.to == "123456789")
    );
    assert!(
        targets
            .iter()
            .any(|t| t.channel == "telegram" && t.to == "987654321")
    );
    assert!(
        targets
            .iter()
            .any(|t| t.channel == "whatsapp" && t.to == "15551234567@s.whatsapp.net")
    );
}

#[test]
fn test_resolve_disabled_channels_excluded() {
    let mut cfg = make_test_channels_config();
    cfg.discord.enabled = false;
    cfg.whatsapp.enabled = false;
    let targets = resolve_all_channel_targets_from_config(Some(&cfg));
    assert_eq!(targets.len(), 2);
    assert!(targets.iter().any(|t| t.channel == "slack"));
    assert!(targets.iter().any(|t| t.channel == "telegram"));
    assert!(!targets.iter().any(|t| t.channel == "discord"));
    assert!(!targets.iter().any(|t| t.channel == "whatsapp"));
}

#[test]
fn test_resolve_whatsapp_format() {
    assert_eq!(
        format_whatsapp_target("+15551234567"),
        "15551234567@s.whatsapp.net"
    );
    assert_eq!(
        format_whatsapp_target("15551234567"),
        "15551234567@s.whatsapp.net"
    );
    assert_eq!(
        format_whatsapp_target("15551234567@s.whatsapp.net"),
        "15551234567@s.whatsapp.net"
    );
}

#[test]
fn test_resolve_no_config() {
    let targets = resolve_all_channel_targets_from_config(None);
    assert!(targets.is_empty());
}

#[test]
fn test_first_concrete_target_skips_wildcard() {
    let list = vec!["*".to_string(), "user123".to_string()];
    assert_eq!(first_concrete_target(&list), "user123");
}

#[test]
fn test_first_concrete_target_empty_list() {
    let list: Vec<String> = vec![];
    assert_eq!(first_concrete_target(&list), "");
}

#[test]
fn test_first_concrete_target_only_wildcard() {
    let list = vec!["*".to_string()];
    assert_eq!(first_concrete_target(&list), "");
}

#[test]
fn test_first_concrete_target_no_wildcard() {
    let list = vec!["alice".to_string(), "bob".to_string()];
    assert_eq!(first_concrete_target(&list), "alice");
}

#[test]
fn test_format_whatsapp_target_with_plus() {
    assert_eq!(
        format_whatsapp_target("+441234567890"),
        "441234567890@s.whatsapp.net"
    );
}

#[test]
fn test_resolve_empty_allow_from_excluded() {
    let mut cfg = make_test_channels_config();
    cfg.slack.allow_from = vec![];
    let targets = resolve_all_channel_targets_from_config(Some(&cfg));
    // Slack should be excluded (empty allow_from -> first_concrete_target returns "")
    assert!(!targets.iter().any(|t| t.channel == "slack"));
}

#[test]
fn test_resolve_wildcard_only_excluded() {
    let mut cfg = make_test_channels_config();
    cfg.telegram.allow_from = vec!["*".to_string()];
    let targets = resolve_all_channel_targets_from_config(Some(&cfg));
    // Telegram wildcard has no concrete target -> excluded
    assert!(!targets.iter().any(|t| t.channel == "telegram"));
}
