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

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "telegram" => Some(ChannelType::Telegram),
            "discord" => Some(ChannelType::Discord),
            "slack" => Some(ChannelType::Slack),
            "whatsapp" => Some(ChannelType::WhatsApp),
            "system" => Some(ChannelType::System),
            "cli" => Some(ChannelType::Cli),
            _ => None,
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
