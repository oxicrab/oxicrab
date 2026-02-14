use serde::{Deserialize, Serialize};

/// Channel type enumeration for type-safe channel identification
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Telegram,
    Discord,
    Slack,
    WhatsApp,
    Twilio,
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
            ChannelType::Twilio => "twilio",
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
            "twilio" => Ok(ChannelType::Twilio),
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
}
