use serde::{Deserialize, Serialize};

/// An access control list where empty means "deny all".
/// Use `["*"]` for allow-all. Serializes/deserializes as a JSON/TOML array of strings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct DenyByDefaultList(Vec<String>);

impl DenyByDefaultList {
    /// Create a new list from a vector of entries.
    pub fn new(entries: Vec<String>) -> Self {
        Self(entries)
    }

    /// Check if the given ID is allowed by this list.
    /// Empty list = deny all. `["*"]` = allow all.
    pub fn allows(&self, id: &str) -> bool {
        !self.0.is_empty() && self.0.iter().any(|entry| entry == id || entry == "*")
    }

    /// Check if the given ID is allowed, normalizing by stripping leading '+' and control chars.
    pub fn allows_normalized(&self, id: &str) -> bool {
        if self.0.is_empty() {
            return false;
        }
        let normalized = id.trim_start_matches('+');
        let normalized: String = normalized.chars().filter(|c| !c.is_control()).collect();
        self.0.iter().any(|entry| {
            let entry_normalized = entry.trim_start_matches('+');
            let entry_normalized: String = entry_normalized
                .chars()
                .filter(|c| !c.is_control())
                .collect();
            entry_normalized == normalized || entry_normalized == "*"
        })
    }

    /// Returns true if the list is empty (deny-all state).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Get access to the inner entries (for iteration, logging, etc.)
    pub fn entries(&self) -> &[String] {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: DenyByDefaultList,
    /// Restrict which group chats the bot responds in. Empty = deny all groups.
    #[serde(default, rename = "allowGroups")]
    pub allow_groups: DenyByDefaultList,
    #[serde(default = "default_dm_policy", rename = "dmPolicy")]
    pub dm_policy: DmPolicy,
}

impl Default for WhatsAppConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_from: DenyByDefaultList::default(),
            allow_groups: DenyByDefaultList::default(),
            dm_policy: default_dm_policy(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: DenyByDefaultList,
    /// Restrict which group chats the bot responds in. Empty = deny all groups.
    #[serde(default, rename = "allowGroups")]
    pub allow_groups: DenyByDefaultList,
    #[serde(default = "default_dm_policy", rename = "dmPolicy")]
    pub dm_policy: DmPolicy,
    /// When true, only respond in groups when the bot is @mentioned or replied to.
    #[serde(default, rename = "mentionOnly")]
    pub mention_only: bool,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token: String::new(),
            allow_from: DenyByDefaultList::default(),
            allow_groups: DenyByDefaultList::default(),
            dm_policy: default_dm_policy(),
            mention_only: false,
        }
    }
}

redact_debug!(
    TelegramConfig,
    enabled,
    redact(token),
    allow_from,
    allow_groups,
    dm_policy,
    mention_only,
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
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: DenyByDefaultList,
    /// Restrict which guild/group chats the bot responds in. Empty = deny all groups.
    #[serde(default, rename = "allowGroups")]
    pub allow_groups: DenyByDefaultList,
    #[serde(default = "default_discord_commands")]
    pub commands: Vec<DiscordCommand>,
    #[serde(default = "default_dm_policy", rename = "dmPolicy")]
    pub dm_policy: DmPolicy,
    /// When true, only respond in guilds when the bot is @mentioned. DMs are unaffected.
    #[serde(default, rename = "mentionOnly")]
    pub mention_only: bool,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token: String::new(),
            allow_from: DenyByDefaultList::default(),
            allow_groups: DenyByDefaultList::default(),
            commands: default_discord_commands(),
            dm_policy: default_dm_policy(),
            mention_only: false,
        }
    }
}

redact_debug!(
    DiscordConfig,
    enabled,
    redact(token),
    allow_from,
    allow_groups,
    commands,
    dm_policy,
    mention_only,
);

fn default_thinking_emoji() -> String {
    "eyes".to_string()
}

fn default_done_emoji() -> String {
    "white_check_mark".to_string()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "botToken")]
    pub bot_token: String,
    #[serde(default, rename = "appToken")]
    pub app_token: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: DenyByDefaultList,
    /// Restrict which channels/groups the bot responds in. Empty = deny all.
    #[serde(default, rename = "allowGroups")]
    pub allow_groups: DenyByDefaultList,
    #[serde(default = "default_dm_policy", rename = "dmPolicy")]
    pub dm_policy: DmPolicy,
    /// Emoji added when a message is received (default: "eyes")
    #[serde(default = "default_thinking_emoji", rename = "thinkingEmoji")]
    pub thinking_emoji: String,
    /// Emoji added after response is sent (default: `white_check_mark`).
    #[serde(default = "default_done_emoji", rename = "doneEmoji")]
    pub done_emoji: String,
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: String::new(),
            app_token: String::new(),
            allow_from: DenyByDefaultList::default(),
            allow_groups: DenyByDefaultList::default(),
            dm_policy: default_dm_policy(),
            thinking_emoji: default_thinking_emoji(),
            done_emoji: default_done_emoji(),
        }
    }
}

redact_debug!(
    SlackConfig,
    enabled,
    redact(bot_token),
    redact(app_token),
    allow_from,
    allow_groups,
    dm_policy,
    thinking_emoji,
    done_emoji,
);

fn default_webhook_port() -> u16 {
    8080
}

fn default_webhook_host() -> String {
    "0.0.0.0".to_string()
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
    #[serde(default)]
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
    #[serde(default = "default_webhook_host", rename = "webhookHost")]
    pub webhook_host: String,
    #[serde(default, rename = "webhookUrl")]
    pub webhook_url: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: DenyByDefaultList,
    /// Restrict which Conversations the bot responds in. Empty = deny all.
    #[serde(default, rename = "allowGroups")]
    pub allow_groups: DenyByDefaultList,
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
            webhook_host: default_webhook_host(),
            webhook_url: String::new(),
            allow_from: DenyByDefaultList::default(),
            allow_groups: DenyByDefaultList::default(),
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
    webhook_host,
    webhook_url,
    allow_from,
    allow_groups,
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
