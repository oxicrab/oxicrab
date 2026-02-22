use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppConfig {
    pub enabled: bool,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: Vec<String>,
    #[serde(default = "default_dm_policy", rename = "dmPolicy")]
    pub dm_policy: DmPolicy,
}

impl Default for WhatsAppConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_from: Vec::new(),
            dm_policy: default_dm_policy(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: Vec<String>,
    #[serde(default = "default_dm_policy", rename = "dmPolicy")]
    pub dm_policy: DmPolicy,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token: String::new(),
            allow_from: Vec::new(),
            dm_policy: default_dm_policy(),
        }
    }
}

redact_debug!(
    TelegramConfig,
    enabled,
    redact(token),
    allow_from,
    dm_policy,
);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordCommand {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub options: Vec<DiscordCommandOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordCommandOption {
    pub name: String,
    pub description: String,
    #[serde(default = "super::default_true")]
    pub required: bool,
}

fn default_discord_commands() -> Vec<DiscordCommand> {
    vec![DiscordCommand {
        name: "ask".to_string(),
        description: "Ask the AI assistant".to_string(),
        options: vec![DiscordCommandOption {
            name: "question".to_string(),
            description: "Your question or message".to_string(),
            required: true,
        }],
    }]
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: Vec<String>,
    #[serde(default = "default_discord_commands")]
    pub commands: Vec<DiscordCommand>,
    #[serde(default = "default_dm_policy", rename = "dmPolicy")]
    pub dm_policy: DmPolicy,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token: String::new(),
            allow_from: Vec::new(),
            commands: default_discord_commands(),
            dm_policy: default_dm_policy(),
        }
    }
}

redact_debug!(
    DiscordConfig,
    enabled,
    redact(token),
    allow_from,
    commands,
    dm_policy,
);

#[derive(Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    pub enabled: bool,
    #[serde(default, rename = "botToken")]
    pub bot_token: String,
    #[serde(default, rename = "appToken")]
    pub app_token: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: Vec<String>,
    #[serde(default = "default_dm_policy", rename = "dmPolicy")]
    pub dm_policy: DmPolicy,
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: String::new(),
            app_token: String::new(),
            allow_from: Vec::new(),
            dm_policy: default_dm_policy(),
        }
    }
}

redact_debug!(
    SlackConfig,
    enabled,
    redact(bot_token),
    redact(app_token),
    allow_from,
    dm_policy,
);

fn default_webhook_port() -> u16 {
    8080
}

fn default_webhook_path() -> String {
    "/twilio/webhook".to_string()
}

/// Policy for handling DMs from unknown senders.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DmPolicy {
    /// Only allow senders on the allowlist (default). Unknown senders are silently denied.
    #[default]
    Allowlist,
    /// Send a pairing code to unknown senders so they can request access.
    Pairing,
    /// Allow all senders regardless of allowlist.
    Open,
}

impl std::fmt::Display for DmPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Allowlist => write!(f, "allowlist"),
            Self::Pairing => write!(f, "pairing"),
            Self::Open => write!(f, "open"),
        }
    }
}

fn default_dm_policy() -> DmPolicy {
    DmPolicy::default()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TwilioConfig {
    pub enabled: bool,
    #[serde(default, rename = "accountSid")]
    pub account_sid: String,
    #[serde(default, rename = "authToken")]
    pub auth_token: String,
    #[serde(default, rename = "phoneNumber")]
    pub phone_number: String,
    #[serde(default = "default_webhook_port", rename = "webhookPort")]
    pub webhook_port: u16,
    #[serde(default = "default_webhook_path", rename = "webhookPath")]
    pub webhook_path: String,
    #[serde(default, rename = "webhookUrl")]
    pub webhook_url: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: Vec<String>,
    #[serde(default = "default_dm_policy", rename = "dmPolicy")]
    pub dm_policy: DmPolicy,
}

impl Default for TwilioConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_sid: String::new(),
            auth_token: String::new(),
            phone_number: String::new(),
            webhook_port: default_webhook_port(),
            webhook_path: default_webhook_path(),
            webhook_url: String::new(),
            allow_from: Vec::new(),
            dm_policy: default_dm_policy(),
        }
    }
}

redact_debug!(
    TwilioConfig,
    enabled,
    redact(account_sid),
    redact(auth_token),
    phone_number,
    webhook_port,
    webhook_path,
    webhook_url,
    allow_from,
    dm_policy,
);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub whatsapp: WhatsAppConfig,
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub discord: DiscordConfig,
    #[serde(default)]
    pub slack: SlackConfig,
    #[serde(default)]
    pub twilio: TwilioConfig,
}
