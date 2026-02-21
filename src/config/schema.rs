use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::path::PathBuf;
use tracing::warn;

/// Generates a `Debug` impl that redacts secret fields.
///
/// Field specifiers:
/// - `field_name`            — printed normally via `&self.field_name`
/// - `redact(field_name)`    — `String` field: shows `[empty]` or `[REDACTED]`
/// - `redact_option(field_name)` — `Option<String>` field: shows `None` or `Some("[REDACTED]")`
macro_rules! redact_debug {
    // Internal: emit a single .field() call
    (@field $builder:ident, $self:ident, redact($field:ident)) => {
        $builder.field(
            stringify!($field),
            &if $self.$field.is_empty() {
                "[empty]"
            } else {
                "[REDACTED]"
            },
        );
    };
    (@field $builder:ident, $self:ident, redact_option($field:ident)) => {
        $builder.field(
            stringify!($field),
            &$self.$field.as_ref().map(|_| "[REDACTED]"),
        );
    };
    (@field $builder:ident, $self:ident, $field:ident) => {
        $builder.field(stringify!($field), &$self.$field);
    };

    // Internal: recursive TT muncher
    (@fields $builder:ident, $self:ident,) => {};
    (@fields $builder:ident, $self:ident, redact($field:ident), $($rest:tt)*) => {
        redact_debug!(@field $builder, $self, redact($field));
        redact_debug!(@fields $builder, $self, $($rest)*);
    };
    (@fields $builder:ident, $self:ident, redact_option($field:ident), $($rest:tt)*) => {
        redact_debug!(@field $builder, $self, redact_option($field));
        redact_debug!(@fields $builder, $self, $($rest)*);
    };
    (@fields $builder:ident, $self:ident, $field:ident, $($rest:tt)*) => {
        redact_debug!(@field $builder, $self, $field);
        redact_debug!(@fields $builder, $self, $($rest)*);
    };

    // Entry point
    ($struct_name:ident, $($fields:tt)*) => {
        impl std::fmt::Debug for $struct_name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let mut builder = f.debug_struct(stringify!($struct_name));
                redact_debug!(@fields builder, self, $($fields)*);
                builder.finish()
            }
        }
    };
}

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
    #[serde(default = "default_true")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_checkpoint_interval", rename = "intervalIterations")]
    pub interval_iterations: u32,
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_iterations: default_checkpoint_interval(),
        }
    }
}

fn default_checkpoint_interval() -> u32 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_threshold_tokens", rename = "thresholdTokens")]
    pub threshold_tokens: u32,
    #[serde(default = "default_keep_recent", rename = "keepRecent")]
    pub keep_recent: usize,
    #[serde(default = "default_true", rename = "extractionEnabled")]
    pub extraction_enabled: bool,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub checkpoint: CheckpointConfig,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_tokens: default_threshold_tokens(),
            keep_recent: default_keep_recent(),
            extraction_enabled: true,
            model: None,
            checkpoint: CheckpointConfig::default(),
        }
    }
}

fn default_threshold_tokens() -> u32 {
    40000
}

fn default_keep_recent() -> usize {
    10
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default, rename = "executionModel")]
    pub execution_model: Option<String>,
    #[serde(default, rename = "executionProvider")]
    pub execution_provider: Option<String>,
    #[serde(default = "default_strategy_file", rename = "strategyFile")]
    pub strategy_file: String,
    #[serde(default = "default_max_iterations", rename = "maxIterations")]
    pub max_iterations: usize,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval: default_interval(),
            execution_model: None,
            execution_provider: None,
            strategy_file: default_strategy_file(),
            max_iterations: default_max_iterations(),
        }
    }
}

fn default_interval() -> u64 {
    300
}

fn default_strategy_file() -> String {
    "HEARTBEAT.md".to_string()
}

fn default_max_iterations() -> usize {
    25
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCost {
    #[serde(default, rename = "inputPerMillion")]
    pub input_per_million: f64,
    #[serde(default, rename = "outputPerMillion")]
    pub output_per_million: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostGuardConfig {
    #[serde(default, rename = "dailyBudgetCents")]
    pub daily_budget_cents: Option<u64>,
    #[serde(default, rename = "maxActionsPerHour")]
    pub max_actions_per_hour: Option<u64>,
    #[serde(default, rename = "modelCosts")]
    pub model_costs: std::collections::HashMap<String, ModelCost>,
}

fn default_failure_threshold() -> u32 {
    5
}

fn default_recovery_timeout_secs() -> u64 {
    60
}

fn default_half_open_probes() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_failure_threshold", rename = "failureThreshold")]
    pub failure_threshold: u32,
    #[serde(
        default = "default_recovery_timeout_secs",
        rename = "recoveryTimeoutSecs"
    )]
    pub recovery_timeout_secs: u64,
    #[serde(default = "default_half_open_probes", rename = "halfOpenProbes")]
    pub half_open_probes: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            failure_threshold: default_failure_threshold(),
            recovery_timeout_secs: default_recovery_timeout_secs(),
            half_open_probes: default_half_open_probes(),
        }
    }
}

fn default_exfil_blocked_tools() -> Vec<String> {
    vec!["http".into(), "web_fetch".into(), "browser".into()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExfiltrationGuardConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Tool names to hide from LLM when enabled (default: http, `web_fetch`, browser)
    #[serde(default = "default_exfil_blocked_tools", rename = "blockedTools")]
    pub blocked_tools: Vec<String>,
}

impl Default for ExfiltrationGuardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            blocked_tools: default_exfil_blocked_tools(),
        }
    }
}

/// Action to take when prompt injection is detected.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptGuardAction {
    /// Log a warning and continue processing (default).
    #[default]
    Warn,
    /// Reject the message entirely.
    Block,
}

impl std::fmt::Display for PromptGuardAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Warn => write!(f, "warn"),
            Self::Block => write!(f, "block"),
        }
    }
}

fn default_prompt_guard_action() -> PromptGuardAction {
    PromptGuardAction::default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptGuardConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Action on detection: `Warn` (log + continue) or `Block` (reject message)
    #[serde(default = "default_prompt_guard_action")]
    pub action: PromptGuardAction,
}

impl PromptGuardConfig {
    /// Whether detected prompt injections should block the message.
    pub fn should_block(&self) -> bool {
        self.action == PromptGuardAction::Block
    }
}

impl Default for PromptGuardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            action: default_prompt_guard_action(),
        }
    }
}

fn default_gentle_threshold() -> u32 {
    12
}
fn default_firm_threshold() -> u32 {
    20
}
fn default_urgent_threshold() -> u32 {
    30
}
fn default_recent_tools_window() -> usize {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_gentle_threshold", rename = "gentleThreshold")]
    pub gentle_threshold: u32,
    #[serde(default = "default_firm_threshold", rename = "firmThreshold")]
    pub firm_threshold: u32,
    #[serde(default = "default_urgent_threshold", rename = "urgentThreshold")]
    pub urgent_threshold: u32,
    #[serde(default = "default_recent_tools_window", rename = "recentToolsWindow")]
    pub recent_tools_window: usize,
}

impl Default for CognitiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gentle_threshold: default_gentle_threshold(),
            firm_threshold: default_firm_threshold(),
            urgent_threshold: default_urgent_threshold(),
            recent_tools_window: default_recent_tools_window(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefaults {
    #[serde(default = "default_workspace")]
    pub workspace: String,
    #[serde(default = "default_model")]
    pub model: String,
    /// Explicit LLM provider override. When set, bypasses model-name inference.
    /// Examples: "anthropic", "openai", "groq", "ollama"
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default = "default_max_tokens", rename = "maxTokens")]
    pub max_tokens: u32,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_max_tool_iterations", rename = "maxToolIterations")]
    pub max_tool_iterations: usize,
    #[serde(default)]
    pub compaction: CompactionConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default = "default_session_ttl_days", rename = "sessionTtlDays")]
    pub session_ttl_days: u32,
    #[serde(
        default = "default_memory_indexer_interval",
        rename = "memoryIndexerInterval"
    )]
    pub memory_indexer_interval: u64,
    #[serde(default = "default_media_ttl_days", rename = "mediaTtlDays")]
    pub media_ttl_days: u32,
    #[serde(
        default = "default_max_concurrent_subagents",
        rename = "maxConcurrentSubagents"
    )]
    pub max_concurrent_subagents: usize,
    #[serde(default, rename = "localModel")]
    pub local_model: Option<String>,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default, rename = "costGuard")]
    pub cost_guard: CostGuardConfig,
    #[serde(default)]
    pub cognitive: CognitiveConfig,
    #[serde(default, rename = "promptGuard")]
    pub prompt_guard: PromptGuardConfig,
}

impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            workspace: default_workspace(),
            model: default_model(),
            provider: None,
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            max_tool_iterations: default_max_tool_iterations(),
            compaction: CompactionConfig::default(),
            daemon: DaemonConfig::default(),
            session_ttl_days: default_session_ttl_days(),
            memory_indexer_interval: default_memory_indexer_interval(),
            media_ttl_days: default_media_ttl_days(),
            max_concurrent_subagents: default_max_concurrent_subagents(),
            local_model: None,
            memory: MemoryConfig::default(),
            cost_guard: CostGuardConfig::default(),
            cognitive: CognitiveConfig::default(),
            prompt_guard: PromptGuardConfig::default(),
        }
    }
}

fn default_memory_indexer_interval() -> u64 {
    300
}

fn default_memory_archive_after_days() -> u32 {
    30
}

fn default_memory_purge_after_days() -> u32 {
    90
}

fn default_embeddings_model() -> String {
    "BAAI/bge-small-en-v1.5".to_string()
}

fn default_hybrid_weight() -> f32 {
    0.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(
        default = "default_memory_archive_after_days",
        rename = "archiveAfterDays"
    )]
    pub archive_after_days: u32,
    #[serde(default = "default_memory_purge_after_days", rename = "purgeAfterDays")]
    pub purge_after_days: u32,
    #[serde(default, rename = "embeddingsEnabled")]
    pub embeddings_enabled: bool,
    #[serde(default = "default_embeddings_model", rename = "embeddingsModel")]
    pub embeddings_model: String,
    /// 0.0 = keyword only, 1.0 = vector only, 0.5 = equal blend
    #[serde(default = "default_hybrid_weight", rename = "hybridWeight")]
    pub hybrid_weight: f32,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            archive_after_days: default_memory_archive_after_days(),
            purge_after_days: default_memory_purge_after_days(),
            embeddings_enabled: false,
            embeddings_model: default_embeddings_model(),
            hybrid_weight: default_hybrid_weight(),
        }
    }
}

fn default_media_ttl_days() -> u32 {
    7
}

fn default_max_concurrent_subagents() -> usize {
    5
}

fn default_session_ttl_days() -> u32 {
    30
}

fn default_workspace() -> String {
    "~/.oxicrab/workspace".to_string()
}

fn default_model() -> String {
    "claude-sonnet-4-5-20250929".to_string()
}

/// Normalize provider aliases to canonical names.
///
/// Maps common aliases (e.g. "claude" → "anthropic", "gpt" → "openai")
/// to the canonical provider name used for routing. Unknown values pass through.
pub fn normalize_provider(provider: &str) -> Cow<'_, str> {
    match provider.to_lowercase().as_str() {
        "claude" | "anthropic" => Cow::Borrowed("anthropic"),
        "gpt" | "openai" => Cow::Borrowed("openai"),
        "google" | "gemini" => Cow::Borrowed("gemini"),
        "openrouter" => Cow::Borrowed("openrouter"),
        "deepseek" => Cow::Borrowed("deepseek"),
        "groq" => Cow::Borrowed("groq"),
        "moonshot" => Cow::Borrowed("moonshot"),
        "zhipu" => Cow::Borrowed("zhipu"),
        "dashscope" => Cow::Borrowed("dashscope"),
        "vllm" => Cow::Borrowed("vllm"),
        "ollama" => Cow::Borrowed("ollama"),
        _ => Cow::Owned(provider.to_lowercase()),
    }
}

fn default_max_tokens() -> u32 {
    8192
}

fn default_temperature() -> f32 {
    0.7
}

fn default_max_tool_iterations() -> usize {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentsConfig {
    #[serde(default)]
    pub defaults: AgentDefaults,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
    #[serde(default, rename = "apiBase")]
    pub api_base: Option<String>,
    /// Custom HTTP headers injected into every request to this provider.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub headers: std::collections::HashMap<String, String>,
    /// Enable prompt-guided tool calling for local models that don't support
    /// native function calling. Injects tool definitions into the system prompt
    /// and parses `<tool_call>` XML blocks from text responses.
    #[serde(default, rename = "promptGuidedTools")]
    pub prompt_guided_tools: bool,
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redacted_headers: std::collections::HashMap<&String, &str> =
            self.headers.keys().map(|k| (k, "[REDACTED]")).collect();
        f.debug_struct("ProviderConfig")
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field("api_base", &self.api_base)
            .field("headers", &redacted_headers)
            .field("prompt_guided_tools", &self.prompt_guided_tools)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AnthropicOAuthConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "accessToken")]
    pub access_token: String,
    #[serde(default, rename = "refreshToken")]
    pub refresh_token: String,
    #[serde(default, rename = "expiresAt")]
    pub expires_at: i64,
    #[serde(default, rename = "credentialsPath")]
    pub credentials_path: Option<String>,
    #[serde(default = "default_true", rename = "autoDetect")]
    pub auto_detect: bool,
}

redact_debug!(
    AnthropicOAuthConfig,
    enabled,
    redact(access_token),
    redact(refresh_token),
    expires_at,
    credentials_path,
    auto_detect,
);

impl Default for AnthropicOAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            access_token: String::new(),
            refresh_token: String::new(),
            expires_at: 0,
            credentials_path: None,
            auto_detect: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvidersConfig {
    #[serde(default)]
    pub anthropic: ProviderConfig,
    #[serde(default, rename = "anthropicOAuth")]
    pub anthropic_oauth: AnthropicOAuthConfig,
    #[serde(default)]
    pub openai: ProviderConfig,
    #[serde(default)]
    pub openrouter: ProviderConfig,
    #[serde(default)]
    pub deepseek: ProviderConfig,
    #[serde(default)]
    pub groq: ProviderConfig,
    #[serde(default)]
    pub zhipu: ProviderConfig,
    #[serde(default)]
    pub dashscope: ProviderConfig,
    #[serde(default)]
    pub vllm: ProviderConfig,
    #[serde(default)]
    pub gemini: ProviderConfig,
    #[serde(default)]
    pub moonshot: ProviderConfig,
    #[serde(default)]
    pub ollama: ProviderConfig,
    #[serde(default, rename = "circuitBreaker")]
    pub circuit_breaker: CircuitBreakerConfig,
}

impl ProvidersConfig {
    /// Get the API key for a given model by resolving the provider name.
    ///
    /// Uses the same 3-tier resolution as `ProviderFactory`: explicit prefix,
    /// model-name inference, then fallback to first available key.
    pub fn get_api_key(&self, model: &str) -> Option<&str> {
        use crate::providers::strategy::{infer_provider_from_model, parse_model_ref};

        let model_ref = parse_model_ref(model);
        let provider_name = model_ref
            .provider
            .or_else(|| infer_provider_from_model(model_ref.model));

        if let Some(name) = provider_name {
            let normalized = normalize_provider(name);
            if let Some(key) = self.get_api_key_for_provider(&normalized) {
                return Some(key);
            }
        }

        // Fallback: first available key
        self.first_available_key()
    }

    /// Get the API key for a specific provider by canonical name.
    pub fn get_api_key_for_provider(&self, provider: &str) -> Option<&str> {
        let normalized = normalize_provider(provider);
        let config = match normalized.as_ref() {
            "anthropic" => &self.anthropic,
            "openai" => &self.openai,
            "gemini" => &self.gemini,
            "openrouter" => &self.openrouter,
            "deepseek" => &self.deepseek,
            "groq" => &self.groq,
            "moonshot" => &self.moonshot,
            "zhipu" => &self.zhipu,
            "dashscope" => &self.dashscope,
            "vllm" => &self.vllm,
            "ollama" => &self.ollama,
            _ => return None,
        };
        if config.api_key.is_empty() {
            None
        } else {
            Some(&config.api_key)
        }
    }

    /// Return the first available API key across all providers.
    fn first_available_key(&self) -> Option<&str> {
        for config in [
            &self.openrouter,
            &self.anthropic,
            &self.openai,
            &self.gemini,
        ] {
            if !config.api_key.is_empty() {
                return Some(&config.api_key);
            }
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    18790
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GoogleConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "clientId")]
    pub client_id: String,
    #[serde(default, rename = "clientSecret")]
    pub client_secret: String,
    #[serde(default = "default_google_scopes")]
    pub scopes: Vec<String>,
}

redact_debug!(
    GoogleConfig,
    enabled,
    client_id,
    redact(client_secret),
    scopes,
);

impl Default for GoogleConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            client_id: String::new(),
            client_secret: String::new(),
            scopes: default_google_scopes(),
        }
    }
}

fn default_google_scopes() -> Vec<String> {
    vec![
        "https://www.googleapis.com/auth/gmail.modify".to_string(),
        "https://www.googleapis.com/auth/gmail.send".to_string(),
        "https://www.googleapis.com/auth/calendar.events".to_string(),
        "https://www.googleapis.com/auth/calendar.readonly".to_string(),
    ]
}

#[derive(Clone, Serialize, Deserialize)]
pub struct WebSearchConfig {
    /// Search provider: "brave" (default) or "duckduckgo"
    #[serde(default = "default_search_provider")]
    pub provider: String,
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
    #[serde(default = "default_max_results", rename = "maxResults")]
    pub max_results: usize,
}

redact_debug!(WebSearchConfig, provider, redact(api_key), max_results,);

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            provider: default_search_provider(),
            api_key: String::new(),
            max_results: default_max_results(),
        }
    }
}

fn default_search_provider() -> String {
    "brave".to_string()
}

fn default_max_results() -> usize {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebToolsConfig {
    #[serde(default)]
    pub search: WebSearchConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Enable Landlock filesystem/network sandboxing for shell commands.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Extra paths to grant read-only access (beyond default system dirs).
    #[serde(default, rename = "additionalReadPaths")]
    pub additional_read_paths: Vec<String>,
    /// Extra paths to grant read-write access (beyond workspace + /tmp).
    #[serde(default, rename = "additionalWritePaths")]
    pub additional_write_paths: Vec<String>,
    /// Block all outbound network connections from shell commands.
    #[serde(default = "default_true", rename = "blockNetwork")]
    pub block_network: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            additional_read_paths: Vec::new(),
            additional_write_paths: Vec::new(),
            block_network: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecToolConfig {
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default = "default_allowed_commands", rename = "allowedCommands")]
    pub allowed_commands: Vec<String>,
    #[serde(default)]
    pub sandbox: SandboxConfig,
}

impl Default for ExecToolConfig {
    fn default() -> Self {
        Self {
            timeout: default_timeout(),
            allowed_commands: default_allowed_commands(),
            sandbox: SandboxConfig::default(),
        }
    }
}

fn default_timeout() -> u64 {
    60
}

fn default_allowed_commands() -> Vec<String> {
    [
        // File listing & navigation
        "ls",
        "find",
        "tree",
        "pwd",
        "basename",
        "dirname",
        "realpath",
        "stat",
        "file",
        // File reading
        "cat",
        "head",
        "tail",
        "less",
        "wc",
        "md5sum",
        "sha256sum",
        // Text processing
        "grep",
        "awk",
        "sed",
        "sort",
        "uniq",
        "cut",
        "tr",
        "diff",
        "comm",
        "paste",
        // Search
        "rg",
        "ag",
        "fd",
        // JSON/YAML/data
        "jq",
        "yq",
        // Git
        "git",
        // Development tools
        "cargo",
        "rustc",
        "npm",
        "npx",
        "node",
        "python3",
        "pip3",
        "make",
        "go",
        // System info
        "date",
        "cal",
        "whoami",
        "hostname",
        "uname",
        "uptime",
        "df",
        "du",
        "free",
        "ps",
        "env",
        "printenv",
        "which",
        "type",
        // Networking (read-only)
        "curl",
        "wget",
        "dig",
        "nslookup",
        "ping",
        "host",
        // Misc utilities
        "echo",
        "printf",
        "test",
        "true",
        "false",
        "yes",
        "seq",
        "xargs",
        "tar",
        "zip",
        "unzip",
        "gzip",
        "gunzip",
        "zcat",
        "tee",
        "touch",
        "mkdir",
        "cp",
        "mv",
        "ln",
    ]
    .iter()
    .map(std::string::ToString::to_string)
    .collect()
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct GitHubConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
}

redact_debug!(GitHubConfig, enabled, redact(token),);

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct WeatherConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
}

redact_debug!(WeatherConfig, enabled, redact(api_key),);

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct TodoistConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
}

redact_debug!(TodoistConfig, enabled, redact(token),);

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct MediaServiceConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
}

redact_debug!(MediaServiceConfig, url, redact(api_key),);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MediaConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub radarr: MediaServiceConfig,
    #[serde(default)]
    pub sonarr: MediaServiceConfig,
}

fn default_obsidian_sync_interval() -> u64 {
    300
}

fn default_obsidian_timeout() -> u64 {
    15
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ObsidianConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "apiUrl")]
    pub api_url: String,
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
    #[serde(default, rename = "vaultName")]
    pub vault_name: String,
    #[serde(default = "default_obsidian_sync_interval", rename = "syncInterval")]
    pub sync_interval: u64,
    #[serde(default = "default_obsidian_timeout")]
    pub timeout: u64,
}

impl Default for ObsidianConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_url: String::new(),
            api_key: String::new(),
            vault_name: String::new(),
            sync_interval: default_obsidian_sync_interval(),
            timeout: default_obsidian_timeout(),
        }
    }
}

redact_debug!(
    ObsidianConfig,
    enabled,
    api_url,
    redact(api_key),
    vault_name,
    sync_interval,
    timeout,
);

fn default_browser_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub headless: bool,
    #[serde(default, rename = "chromePath")]
    pub chrome_path: Option<String>,
    #[serde(default = "default_browser_timeout")]
    pub timeout: u64,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            headless: true,
            chrome_path: None,
            timeout: default_browser_timeout(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsConfig {
    #[serde(default)]
    pub web: WebToolsConfig,
    #[serde(default)]
    pub exec: ExecToolConfig,
    #[serde(default, rename = "restrictToWorkspace")]
    pub restrict_to_workspace: bool,
    #[serde(default)]
    pub google: GoogleConfig,
    #[serde(default)]
    pub github: GitHubConfig,
    #[serde(default)]
    pub weather: WeatherConfig,
    #[serde(default)]
    pub todoist: TodoistConfig,
    #[serde(default)]
    pub media: MediaConfig,
    #[serde(default)]
    pub obsidian: ObsidianConfig,
    #[serde(default)]
    pub browser: BrowserConfig,
    #[serde(default, rename = "imageGen")]
    pub image_gen: ImageGenConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default, rename = "exfiltrationGuard")]
    pub exfiltration_guard: ExfiltrationGuardConfig,
}

fn default_image_gen_provider() -> String {
    "openai".to_string()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ImageGenConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_image_gen_provider", rename = "defaultProvider")]
    pub default_provider: String,
    /// Runtime-injected from providers.openai.apiKey
    #[serde(skip)]
    pub openai_api_key: Option<String>,
    /// Runtime-injected from providers.gemini.apiKey
    #[serde(skip)]
    pub google_api_key: Option<String>,
}

redact_debug!(
    ImageGenConfig,
    enabled,
    default_provider,
    redact_option(openai_api_key),
    redact_option(google_api_key),
);

impl Default for ImageGenConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_provider: default_image_gen_provider(),
            openai_api_key: None,
            google_api_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: std::collections::HashMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Trust level for this MCP server: "local" (full access), "verified"
    /// (requires approval), or "community" (read-only safe tools only).
    #[serde(default = "default_mcp_trust")]
    pub trust: String,
    /// Landlock sandbox config for the MCP server child process.
    /// Defaults to enabled with network blocked (same as shell tool).
    #[serde(default)]
    pub sandbox: SandboxConfig,
}

fn default_mcp_trust() -> String {
    "local".to_string()
}

fn default_transcription_api_base() -> String {
    "https://api.groq.com/openai/v1/audio/transcriptions".to_string()
}

fn default_transcription_model() -> String {
    "whisper-large-v3-turbo".to_string()
}

fn default_whisper_threads() -> u16 {
    4
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TranscriptionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
    #[serde(default = "default_transcription_api_base", rename = "apiBase")]
    pub api_base: String,
    #[serde(default = "default_transcription_model")]
    pub model: String,
    #[serde(default, rename = "localModelPath")]
    pub local_model_path: String,
    #[serde(default = "default_true", rename = "preferLocal")]
    pub prefer_local: bool,
    #[serde(default = "default_whisper_threads")]
    pub threads: u16,
}

impl Default for TranscriptionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: String::new(),
            api_base: default_transcription_api_base(),
            model: default_transcription_model(),
            local_model_path: String::new(),
            prefer_local: true,
            threads: default_whisper_threads(),
        }
    }
}

redact_debug!(
    TranscriptionConfig,
    enabled,
    redact(api_key),
    api_base,
    model,
    local_model_path,
    prefer_local,
    threads,
);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VoiceConfig {
    #[serde(default)]
    pub transcription: TranscriptionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CredentialHelperConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Format adapter: "json" (default), "1password", "bitwarden", "line"
    #[serde(default)]
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub agents: AgentsConfig,
    #[serde(default)]
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub voice: VoiceConfig,
    #[serde(default, rename = "credentialHelper")]
    pub credential_helper: CredentialHelperConfig,
}

impl Config {
    pub fn workspace_path(&self) -> PathBuf {
        crate::utils::get_workspace_path(&self.agents.defaults.workspace)
    }

    /// Validate configuration values
    pub fn validate(&self) -> Result<(), crate::errors::OxicrabError> {
        self.validate_agent_defaults()?;
        self.validate_compaction()?;
        self.validate_memory()?;
        self.validate_cognitive()?;
        self.validate_gateway()?;
        self.validate_tools()?;
        self.validate_channels()?;
        Ok(())
    }

    fn validate_agent_defaults(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;
        let d = &self.agents.defaults;

        if d.max_tokens == 0 {
            return Err(OxicrabError::Config(
                "agents.defaults.maxTokens must be > 0".into(),
            ));
        }
        if d.max_tokens > 1_000_000 {
            return Err(OxicrabError::Config(
                "agents.defaults.maxTokens is unreasonably large (> 1,000,000)".into(),
            ));
        }
        if d.temperature < 0.0 || d.temperature > 2.0 {
            return Err(OxicrabError::Config(
                "agents.defaults.temperature must be between 0.0 and 2.0".into(),
            ));
        }
        if d.max_tool_iterations == 0 {
            return Err(OxicrabError::Config(
                "agents.defaults.maxToolIterations must be > 0".into(),
            ));
        }
        if d.max_tool_iterations > 1000 {
            return Err(OxicrabError::Config(
                "agents.defaults.maxToolIterations is unreasonably large (> 1000)".into(),
            ));
        }
        if d.daemon.enabled {
            if d.daemon.interval == 0 {
                return Err(OxicrabError::Config(
                    "agents.defaults.daemon.interval must be > 0 when enabled".into(),
                ));
            }
            if d.daemon.interval < 60 {
                warn!("Daemon interval is very short (< 60s), this may cause high resource usage");
            }
        }
        Ok(())
    }

    fn validate_compaction(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;
        let c = &self.agents.defaults.compaction;

        if c.enabled {
            if c.threshold_tokens == 0 {
                return Err(OxicrabError::Config(
                    "agents.defaults.compaction.thresholdTokens must be > 0 when enabled".into(),
                ));
            }
            if c.keep_recent == 0 {
                return Err(OxicrabError::Config(
                    "agents.defaults.compaction.keepRecent must be > 0 when enabled".into(),
                ));
            }
        }
        if c.checkpoint.enabled && c.checkpoint.interval_iterations == 0 {
            return Err(OxicrabError::Config(
                "agents.defaults.compaction.checkpoint.intervalIterations must be > 0 when enabled"
                    .into(),
            ));
        }
        Ok(())
    }

    fn validate_memory(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;
        let m = &self.agents.defaults.memory;

        if !(0.0..=1.0).contains(&m.hybrid_weight) {
            return Err(OxicrabError::Config(
                "agents.defaults.memory.hybridWeight must be between 0.0 and 1.0".into(),
            ));
        }
        if m.archive_after_days > 0
            && m.purge_after_days > 0
            && m.purge_after_days <= m.archive_after_days
        {
            return Err(OxicrabError::Config(
                "agents.defaults.memory.purgeAfterDays must be > archiveAfterDays".into(),
            ));
        }
        Ok(())
    }

    fn validate_cognitive(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;
        let c = &self.agents.defaults.cognitive;

        if c.enabled
            && (c.gentle_threshold >= c.firm_threshold || c.firm_threshold >= c.urgent_threshold)
        {
            return Err(OxicrabError::Config(
                "agents.defaults.cognitive thresholds must be ordered: gentle < firm < urgent"
                    .into(),
            ));
        }
        Ok(())
    }

    fn validate_gateway(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;

        if self.gateway.port == 0 {
            return Err(OxicrabError::Config("gateway.port must be > 0".into()));
        }
        Ok(())
    }

    fn validate_tools(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;

        if self.tools.exec.timeout == 0 {
            return Err(OxicrabError::Config(
                "tools.exec.timeout must be > 0".into(),
            ));
        }
        if self.tools.exec.timeout > 3600 {
            warn!("tools.exec.timeout is very long (> 3600s), this may cause timeouts");
        }
        if self.tools.browser.timeout == 0 {
            return Err(OxicrabError::Config(
                "tools.browser.timeout must be > 0".into(),
            ));
        }
        if self.tools.obsidian.enabled {
            if self.tools.obsidian.api_url.is_empty() {
                return Err(OxicrabError::Config(
                    "tools.obsidian.apiUrl is required when obsidian is enabled".into(),
                ));
            }
            if self.tools.obsidian.api_key.is_empty() {
                return Err(OxicrabError::Config(
                    "tools.obsidian.apiKey is required when obsidian is enabled".into(),
                ));
            }
            if self.tools.obsidian.vault_name.is_empty() {
                return Err(OxicrabError::Config(
                    "tools.obsidian.vaultName is required when obsidian is enabled".into(),
                ));
            }
        }
        if self.tools.web.search.max_results == 0 {
            return Err(OxicrabError::Config(
                "tools.web.search.maxResults must be > 0".into(),
            ));
        }
        if self.tools.web.search.max_results > 100 {
            warn!("tools.web.search.maxResults is very large (> 100), this may be slow");
        }
        Ok(())
    }

    fn validate_channels(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;
        let tw = &self.channels.twilio;

        if tw.enabled {
            if tw.account_sid.is_empty() {
                return Err(OxicrabError::Config(
                    "channels.twilio.accountSid is required when twilio is enabled".into(),
                ));
            }
            if tw.auth_token.is_empty() {
                return Err(OxicrabError::Config(
                    "channels.twilio.authToken is required when twilio is enabled".into(),
                ));
            }
            if tw.webhook_url.is_empty() {
                return Err(OxicrabError::Config(
                    "channels.twilio.webhookUrl is required when twilio is enabled".into(),
                ));
            }
            if tw.webhook_port == 0 {
                return Err(OxicrabError::Config(
                    "channels.twilio.webhookPort must be > 0 when twilio is enabled".into(),
                ));
            }
            if tw.webhook_path.is_empty() || !tw.webhook_path.starts_with('/') {
                return Err(OxicrabError::Config(
                    "channels.twilio.webhookPath must start with '/' when twilio is enabled".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn get_api_key(&self, model: Option<&str>) -> Option<&str> {
        let model = model.unwrap_or(&self.agents.defaults.model);
        self.providers.get_api_key(model)
    }

    /// Collect all non-empty secret values for leak detection.
    ///
    /// Returns `(name, value)` pairs covering provider API keys, channel tokens,
    /// and tool credentials. The leak detector uses these to scan outbound
    /// messages for encoded variants (raw, base64, hex).
    pub fn collect_secrets(&self) -> Vec<(&str, &str)> {
        let mut secrets = Vec::new();
        let candidates: &[(&str, &str)] = &[
            ("anthropic_api_key", &self.providers.anthropic.api_key),
            ("openai_api_key", &self.providers.openai.api_key),
            ("openrouter_api_key", &self.providers.openrouter.api_key),
            ("deepseek_api_key", &self.providers.deepseek.api_key),
            ("groq_api_key", &self.providers.groq.api_key),
            ("gemini_api_key", &self.providers.gemini.api_key),
            ("moonshot_api_key", &self.providers.moonshot.api_key),
            ("zhipu_api_key", &self.providers.zhipu.api_key),
            ("dashscope_api_key", &self.providers.dashscope.api_key),
            (
                "anthropic_oauth_access",
                &self.providers.anthropic_oauth.access_token,
            ),
            (
                "anthropic_oauth_refresh",
                &self.providers.anthropic_oauth.refresh_token,
            ),
            ("telegram_token", &self.channels.telegram.token),
            ("discord_token", &self.channels.discord.token),
            ("slack_bot_token", &self.channels.slack.bot_token),
            ("slack_app_token", &self.channels.slack.app_token),
            ("twilio_auth_token", &self.channels.twilio.auth_token),
            ("github_token", &self.tools.github.token),
            ("weather_api_key", &self.tools.weather.api_key),
            ("todoist_token", &self.tools.todoist.token),
            ("obsidian_api_key", &self.tools.obsidian.api_key),
            ("web_search_api_key", &self.tools.web.search.api_key),
            ("vllm_api_key", &self.providers.vllm.api_key),
            ("ollama_api_key", &self.providers.ollama.api_key),
            ("google_client_secret", &self.tools.google.client_secret),
            ("radarr_api_key", &self.tools.media.radarr.api_key),
            ("sonarr_api_key", &self.tools.media.sonarr.api_key),
            ("transcription_api_key", &self.voice.transcription.api_key),
            ("twilio_account_sid", &self.channels.twilio.account_sid),
        ];
        for &(name, value) in candidates {
            if !value.is_empty() {
                secrets.push((name, value));
            }
        }

        // Include custom header values from all providers (may contain auth tokens)
        let provider_configs = [
            &self.providers.anthropic,
            &self.providers.openai,
            &self.providers.openrouter,
            &self.providers.deepseek,
            &self.providers.groq,
            &self.providers.zhipu,
            &self.providers.dashscope,
            &self.providers.vllm,
            &self.providers.gemini,
            &self.providers.moonshot,
            &self.providers.ollama,
        ];
        for cfg in provider_configs {
            for value in cfg.headers.values() {
                if !value.is_empty() {
                    secrets.push(("provider_header", value.as_str()));
                }
            }
        }

        secrets
    }

    /// Create an LLM provider instance based on configuration.
    ///
    /// Uses a 3-tier resolution strategy: explicit provider field → prefix
    /// notation → model-name inference.
    pub fn create_provider(
        &self,
        model: Option<&str>,
    ) -> anyhow::Result<std::sync::Arc<dyn crate::providers::base::LLMProvider>> {
        use crate::providers::strategy::ProviderFactory;

        let model = model.unwrap_or(&self.agents.defaults.model);
        let factory = ProviderFactory::new(self);

        if let Some(ref local_model) = self.agents.defaults.local_model
            && !local_model.is_empty()
        {
            let cloud = factory.create_provider(model)?;
            let mut local = factory.create_provider(local_model)?;
            if self.should_use_prompt_guided_tools(local_model) {
                local = crate::providers::prompt_guided::PromptGuidedToolsProvider::wrap(local);
            }
            return Ok(std::sync::Arc::new(
                crate::providers::fallback::FallbackProvider::new(
                    cloud,
                    local,
                    model.to_string(),
                    local_model.clone(),
                ),
            ));
        }

        let provider = factory.create_provider(model)?;
        if self.should_use_prompt_guided_tools(model) {
            return Ok(crate::providers::prompt_guided::PromptGuidedToolsProvider::wrap(provider));
        }

        Ok(provider)
    }

    /// Check if a model should use prompt-guided tool calling based on its
    /// resolved provider config.
    fn should_use_prompt_guided_tools(&self, model: &str) -> bool {
        use crate::providers::strategy::{infer_provider_from_model, parse_model_ref};

        // Check explicit provider field first
        if let Some(ref provider) = self.agents.defaults.provider {
            let normalized = normalize_provider(provider);
            return match normalized.as_ref() {
                "ollama" => self.providers.ollama.prompt_guided_tools,
                "vllm" => self.providers.vllm.prompt_guided_tools,
                _ => false,
            };
        }

        // Check prefix notation
        let model_ref = parse_model_ref(model);
        if let Some(prefix_provider) = model_ref.provider {
            let normalized = normalize_provider(prefix_provider);
            return match normalized.as_ref() {
                "ollama" => self.providers.ollama.prompt_guided_tools,
                "vllm" => self.providers.vllm.prompt_guided_tools,
                _ => false,
            };
        }

        // Check model-name inference
        if let Some(inferred) = infer_provider_from_model(model_ref.model) {
            return match inferred {
                "ollama" => self.providers.ollama.prompt_guided_tools,
                "vllm" => self.providers.vllm.prompt_guided_tools,
                _ => false,
            };
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_validates() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_zero_max_tokens() {
        let mut config = Config::default();
        config.agents.defaults.max_tokens = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_temperature_negative() {
        let mut config = Config::default();
        config.agents.defaults.temperature = -1.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_temperature_too_high() {
        let mut config = Config::default();
        config.agents.defaults.temperature = 3.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_zero_max_tool_iterations() {
        let mut config = Config::default();
        config.agents.defaults.max_tool_iterations = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_zero_port() {
        let mut config = Config::default();
        config.gateway.port = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_zero_exec_timeout() {
        let mut config = Config::default();
        config.tools.exec.timeout = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_zero_max_results() {
        let mut config = Config::default();
        config.tools.web.search.max_results = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_memory_purge_before_archive() {
        let mut config = Config::default();
        config.agents.defaults.memory.archive_after_days = 30;
        config.agents.defaults.memory.purge_after_days = 10; // less than archive
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_get_api_key_with_anthropic_model() {
        let mut config = Config::default();
        config.providers.anthropic.api_key = "test-anthropic-key".to_string();
        let api_key = config.get_api_key(Some("claude-sonnet-4-5-20250929"));
        assert_eq!(api_key, Some("test-anthropic-key"));
    }

    #[test]
    fn test_get_api_key_with_openai_model() {
        let mut config = Config::default();
        config.providers.openai.api_key = "test-openai-key".to_string();
        let api_key = config.get_api_key(Some("gpt-4"));
        assert_eq!(api_key, Some("test-openai-key"));
    }

    #[test]
    fn test_get_api_key_fallback_order() {
        let mut config = Config::default();
        config.providers.anthropic.api_key = "test-anthropic-key".to_string();
        // Call with no model parameter and no match - should fall back to first available
        let api_key = config.get_api_key(Some("unknown-model"));
        assert_eq!(api_key, Some("test-anthropic-key"));
    }

    #[test]
    fn test_valid_dm_policy_values() {
        for policy in &[DmPolicy::Allowlist, DmPolicy::Pairing, DmPolicy::Open] {
            let mut config = Config::default();
            config.channels.telegram.dm_policy = policy.clone();
            assert!(
                config.validate().is_ok(),
                "policy '{:?}' should be valid",
                policy
            );
        }
    }

    #[test]
    fn test_invalid_dm_policy_rejected() {
        // Invalid dm_policy values are now rejected at deserialization time (serde enum)
        let json = r#"{ "channels": { "telegram": { "enabled": false, "dmPolicy": "invalid" } } }"#;
        let result: Result<Config, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "invalid dmPolicy should be rejected by serde"
        );
    }

    #[test]
    fn test_dm_policy_default_is_allowlist() {
        let config = Config::default();
        assert_eq!(config.channels.telegram.dm_policy, DmPolicy::Allowlist);
        assert_eq!(config.channels.discord.dm_policy, DmPolicy::Allowlist);
        assert_eq!(config.channels.slack.dm_policy, DmPolicy::Allowlist);
        assert_eq!(config.channels.whatsapp.dm_policy, DmPolicy::Allowlist);
        assert_eq!(config.channels.twilio.dm_policy, DmPolicy::Allowlist);
    }

    #[test]
    fn test_dm_policy_deserializes_from_json() {
        let json = r#"{
            "channels": {
                "telegram": { "enabled": false, "dmPolicy": "pairing" },
                "discord": { "enabled": false, "dmPolicy": "open" }
            }
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.channels.telegram.dm_policy, DmPolicy::Pairing);
        assert_eq!(config.channels.discord.dm_policy, DmPolicy::Open);
        // Others default to Allowlist
        assert_eq!(config.channels.slack.dm_policy, DmPolicy::Allowlist);
    }

    #[test]
    fn test_credential_helper_config_default() {
        let config = Config::default();
        assert!(config.credential_helper.command.is_empty());
        assert!(config.credential_helper.args.is_empty());
        assert!(config.credential_helper.format.is_empty());
    }

    #[test]
    fn test_credential_helper_config_deserializes() {
        let json = r#"{
            "credentialHelper": {
                "command": "op",
                "args": ["--vault", "oxicrab"],
                "format": "1password"
            }
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.credential_helper.command, "op");
        assert_eq!(config.credential_helper.args, vec!["--vault", "oxicrab"]);
        assert_eq!(config.credential_helper.format, "1password");
    }

    #[test]
    fn test_credential_helper_config_missing_is_default() {
        let json = r"{}";
        let config: Config = serde_json::from_str(json).unwrap();
        assert!(config.credential_helper.command.is_empty());
    }
}
