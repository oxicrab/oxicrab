use serde::{Deserialize, Serialize};

/// Channel type enumeration for type-safe channel identification
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Telegram,
    Discord,
    Slack,
    WhatsApp,
    System,
    Cli,
}

impl ChannelType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChannelType::Telegram => "telegram",
            ChannelType::Discord => "discord",
            ChannelType::Slack => "slack",
            ChannelType::WhatsApp => "whatsapp",
            ChannelType::System => "system",
            ChannelType::Cli => "cli",
        }
    }
}

impl std::str::FromStr for ChannelType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "telegram" => Ok(ChannelType::Telegram),
            "discord" => Ok(ChannelType::Discord),
            "slack" => Ok(ChannelType::Slack),
            "whatsapp" => Ok(ChannelType::WhatsApp),
            "system" => Ok(ChannelType::System),
            "cli" => Ok(ChannelType::Cli),
            _ => Err(format!("Unknown channel type: {}", s)),
        }
    }
}

impl From<ChannelType> for String {
    fn from(channel: ChannelType) -> Self {
        channel.as_str().to_string()
    }
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_as_str_returns_correct_string() {
        assert_eq!(ChannelType::Telegram.as_str(), "telegram");
        assert_eq!(ChannelType::Discord.as_str(), "discord");
        assert_eq!(ChannelType::Slack.as_str(), "slack");
        assert_eq!(ChannelType::WhatsApp.as_str(), "whatsapp");
        assert_eq!(ChannelType::System.as_str(), "system");
        assert_eq!(ChannelType::Cli.as_str(), "cli");
    }

    #[test]
    fn test_from_str_parses_valid_strings() {
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
            ChannelType::from_str("system").unwrap(),
            ChannelType::System
        );
        assert_eq!(ChannelType::from_str("cli").unwrap(), ChannelType::Cli);
    }

    #[test]
    fn test_from_str_returns_err_for_unknown() {
        let result = ChannelType::from_str("unknown");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Unknown channel type: unknown");
    }

    #[test]
    fn test_display_matches_as_str() {
        assert_eq!(format!("{}", ChannelType::Telegram), "telegram");
        assert_eq!(format!("{}", ChannelType::Discord), "discord");
        assert_eq!(format!("{}", ChannelType::Slack), "slack");
        assert_eq!(format!("{}", ChannelType::WhatsApp), "whatsapp");
        assert_eq!(format!("{}", ChannelType::System), "system");
        assert_eq!(format!("{}", ChannelType::Cli), "cli");
    }

    #[test]
    fn test_from_channel_type_to_string() {
        assert_eq!(String::from(ChannelType::Telegram), "telegram");
        assert_eq!(String::from(ChannelType::Discord), "discord");
        assert_eq!(String::from(ChannelType::Slack), "slack");
        assert_eq!(String::from(ChannelType::WhatsApp), "whatsapp");
        assert_eq!(String::from(ChannelType::System), "system");
        assert_eq!(String::from(ChannelType::Cli), "cli");
    }

    #[test]
    fn test_serde_json_roundtrip() {
        let variants = vec![
            ChannelType::Telegram,
            ChannelType::Discord,
            ChannelType::Slack,
            ChannelType::WhatsApp,
            ChannelType::System,
            ChannelType::Cli,
        ];

        for variant in variants {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: ChannelType = serde_json::from_str(&json).unwrap();
            assert_eq!(variant, deserialized);
        }
    }
}
