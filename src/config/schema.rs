use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WhatsAppConfig {
    pub enabled: bool,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct TelegramConfig {
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: Vec<String>,
    pub proxy: Option<String>,
}

impl std::fmt::Debug for TelegramConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramConfig")
            .field("enabled", &self.enabled)
            .field(
                "token",
                &if self.token.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field("allow_from", &self.allow_from)
            .field("proxy", &self.proxy)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct DiscordConfig {
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: Vec<String>,
}

impl std::fmt::Debug for DiscordConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordConfig")
            .field("enabled", &self.enabled)
            .field(
                "token",
                &if self.token.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field("allow_from", &self.allow_from)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct SlackConfig {
    pub enabled: bool,
    #[serde(default, rename = "botToken")]
    pub bot_token: String,
    #[serde(default, rename = "appToken")]
    pub app_token: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: Vec<String>,
}

impl std::fmt::Debug for SlackConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackConfig")
            .field("enabled", &self.enabled)
            .field(
                "bot_token",
                &if self.bot_token.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field(
                "app_token",
                &if self.app_token.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field("allow_from", &self.allow_from)
            .finish()
    }
}

fn default_webhook_port() -> u16 {
    8080
}

fn default_webhook_path() -> String {
    "/twilio/webhook".to_string()
}

#[derive(Clone, Serialize, Deserialize, Default)]
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
}

impl std::fmt::Debug for TwilioConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TwilioConfig")
            .field("enabled", &self.enabled)
            .field(
                "account_sid",
                &if self.account_sid.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field(
                "auth_token",
                &if self.auth_token.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field("phone_number", &self.phone_number)
            .field("webhook_port", &self.webhook_port)
            .field("webhook_path", &self.webhook_path)
            .field("webhook_url", &self.webhook_url)
            .field("allow_from", &self.allow_from)
            .finish()
    }
}

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
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_tokens: default_threshold_tokens(),
            keep_recent: default_keep_recent(),
            extraction_enabled: true,
            model: None,
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
pub struct AgentDefaults {
    #[serde(default = "default_workspace")]
    pub workspace: String,
    #[serde(default = "default_model")]
    pub model: String,
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
}

impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            workspace: default_workspace(),
            model: default_model(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            max_tool_iterations: default_max_tool_iterations(),
            compaction: CompactionConfig::default(),
            daemon: DaemonConfig::default(),
            session_ttl_days: default_session_ttl_days(),
            memory_indexer_interval: default_memory_indexer_interval(),
            media_ttl_days: default_media_ttl_days(),
            max_concurrent_subagents: default_max_concurrent_subagents(),
        }
    }
}

fn default_memory_indexer_interval() -> u64 {
    300
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
    "~/.nanobot/workspace".to_string()
}

fn default_model() -> String {
    "claude-sonnet-4-5-20250929".to_string()
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
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

impl std::fmt::Debug for AnthropicOAuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicOAuthConfig")
            .field("enabled", &self.enabled)
            .field(
                "access_token",
                &if self.access_token.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field(
                "refresh_token",
                &if self.refresh_token.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field("expires_at", &self.expires_at)
            .field("credentials_path", &self.credentials_path)
            .field("auto_detect", &self.auto_detect)
            .finish()
    }
}

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
}

impl ProvidersConfig {
    /// Get the API key for a given model name by matching provider keywords,
    /// falling back to the first available key.
    pub fn get_api_key(&self, model: &str) -> Option<&str> {
        let model_lower = model.to_lowercase();

        // Match provider by model name
        if model_lower.contains("openrouter") && !self.openrouter.api_key.is_empty() {
            return Some(&self.openrouter.api_key);
        }
        if model_lower.contains("deepseek") && !self.deepseek.api_key.is_empty() {
            return Some(&self.deepseek.api_key);
        }
        if (model_lower.contains("anthropic") || model_lower.contains("claude"))
            && !self.anthropic.api_key.is_empty()
        {
            return Some(&self.anthropic.api_key);
        }
        if (model_lower.contains("openai") || model_lower.contains("gpt"))
            && !self.openai.api_key.is_empty()
        {
            return Some(&self.openai.api_key);
        }
        if model_lower.contains("gemini") && !self.gemini.api_key.is_empty() {
            return Some(&self.gemini.api_key);
        }
        if model_lower.contains("groq") && !self.groq.api_key.is_empty() {
            return Some(&self.groq.api_key);
        }
        if model_lower.contains("moonshot") && !self.moonshot.api_key.is_empty() {
            return Some(&self.moonshot.api_key);
        }
        if model_lower.contains("zhipu") && !self.zhipu.api_key.is_empty() {
            return Some(&self.zhipu.api_key);
        }
        if model_lower.contains("dashscope") && !self.dashscope.api_key.is_empty() {
            return Some(&self.dashscope.api_key);
        }
        if model_lower.contains("vllm") && !self.vllm.api_key.is_empty() {
            return Some(&self.vllm.api_key);
        }

        // Fallback: first available key
        if !self.openrouter.api_key.is_empty() {
            return Some(&self.openrouter.api_key);
        }
        if !self.anthropic.api_key.is_empty() {
            return Some(&self.anthropic.api_key);
        }
        if !self.openai.api_key.is_empty() {
            return Some(&self.openai.api_key);
        }
        if !self.gemini.api_key.is_empty() {
            return Some(&self.gemini.api_key);
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
    "0.0.0.0".to_string()
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

impl std::fmt::Debug for GoogleConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleConfig")
            .field("enabled", &self.enabled)
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &if self.client_secret.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field("scopes", &self.scopes)
            .finish()
    }
}

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

impl std::fmt::Debug for WebSearchConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSearchConfig")
            .field("provider", &self.provider)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field("max_results", &self.max_results)
            .finish()
    }
}

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
pub struct ExecToolConfig {
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default = "default_allowed_commands", rename = "allowedCommands")]
    pub allowed_commands: Vec<String>,
}

impl Default for ExecToolConfig {
    fn default() -> Self {
        Self {
            timeout: default_timeout(),
            allowed_commands: default_allowed_commands(),
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

impl std::fmt::Debug for GitHubConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubConfig")
            .field("enabled", &self.enabled)
            .field(
                "token",
                &if self.token.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct WeatherConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
}

impl std::fmt::Debug for WeatherConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WeatherConfig")
            .field("enabled", &self.enabled)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct TodoistConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
}

impl std::fmt::Debug for TodoistConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TodoistConfig")
            .field("enabled", &self.enabled)
            .field(
                "token",
                &if self.token.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct MediaServiceConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
}

impl std::fmt::Debug for MediaServiceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaServiceConfig")
            .field("url", &self.url)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .finish()
    }
}

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

impl std::fmt::Debug for ObsidianConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObsidianConfig")
            .field("enabled", &self.enabled)
            .field("api_url", &self.api_url)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field("vault_name", &self.vault_name)
            .field("sync_interval", &self.sync_interval)
            .field("timeout", &self.timeout)
            .finish()
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

impl std::fmt::Debug for TranscriptionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranscriptionConfig")
            .field("enabled", &self.enabled)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field("api_base", &self.api_base)
            .field("model", &self.model)
            .field("local_model_path", &self.local_model_path)
            .field("prefer_local", &self.prefer_local)
            .field("threads", &self.threads)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VoiceConfig {
    #[serde(default)]
    pub transcription: TranscriptionConfig,
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
}

impl Config {
    pub fn workspace_path(&self) -> PathBuf {
        crate::utils::get_workspace_path(&self.agents.defaults.workspace)
    }

    /// Validate configuration values
    pub fn validate(&self) -> Result<(), crate::errors::NanobotError> {
        use crate::errors::NanobotError;

        // Validate agent defaults
        if self.agents.defaults.max_tokens == 0 {
            return Err(NanobotError::Config(
                "agents.defaults.maxTokens must be > 0".into(),
            ));
        }
        if self.agents.defaults.max_tokens > 1_000_000 {
            return Err(NanobotError::Config(
                "agents.defaults.maxTokens is unreasonably large (> 1,000,000)".into(),
            ));
        }
        if self.agents.defaults.temperature < 0.0 || self.agents.defaults.temperature > 2.0 {
            return Err(NanobotError::Config(
                "agents.defaults.temperature must be between 0.0 and 2.0".into(),
            ));
        }
        if self.agents.defaults.max_tool_iterations == 0 {
            return Err(NanobotError::Config(
                "agents.defaults.maxToolIterations must be > 0".into(),
            ));
        }
        if self.agents.defaults.max_tool_iterations > 1000 {
            return Err(NanobotError::Config(
                "agents.defaults.maxToolIterations is unreasonably large (> 1000)".into(),
            ));
        }

        // Validate compaction config
        if self.agents.defaults.compaction.enabled {
            if self.agents.defaults.compaction.threshold_tokens == 0 {
                return Err(NanobotError::Config(
                    "agents.defaults.compaction.thresholdTokens must be > 0 when enabled".into(),
                ));
            }
            if self.agents.defaults.compaction.keep_recent == 0 {
                return Err(NanobotError::Config(
                    "agents.defaults.compaction.keepRecent must be > 0 when enabled".into(),
                ));
            }
        }

        // Validate daemon config
        if self.agents.defaults.daemon.enabled {
            if self.agents.defaults.daemon.interval == 0 {
                return Err(NanobotError::Config(
                    "agents.defaults.daemon.interval must be > 0 when enabled".into(),
                ));
            }
            if self.agents.defaults.daemon.interval < 60 {
                warn!("Daemon interval is very short (< 60s), this may cause high resource usage");
            }
        }

        // Validate gateway config
        if self.gateway.port == 0 {
            return Err(NanobotError::Config("gateway.port must be > 0".into()));
        }

        // Validate tools config
        if self.tools.exec.timeout == 0 {
            return Err(NanobotError::Config(
                "tools.exec.timeout must be > 0".into(),
            ));
        }
        if self.tools.exec.timeout > 3600 {
            warn!("tools.exec.timeout is very long (> 3600s), this may cause timeouts");
        }

        // Validate obsidian config
        if self.tools.obsidian.enabled {
            if self.tools.obsidian.api_url.is_empty() {
                return Err(NanobotError::Config(
                    "tools.obsidian.apiUrl is required when obsidian is enabled".into(),
                ));
            }
            if self.tools.obsidian.api_key.is_empty() {
                return Err(NanobotError::Config(
                    "tools.obsidian.apiKey is required when obsidian is enabled".into(),
                ));
            }
            if self.tools.obsidian.vault_name.is_empty() {
                return Err(NanobotError::Config(
                    "tools.obsidian.vaultName is required when obsidian is enabled".into(),
                ));
            }
        }

        // Validate Twilio config
        if self.channels.twilio.enabled {
            if self.channels.twilio.account_sid.is_empty() {
                return Err(NanobotError::Config(
                    "channels.twilio.accountSid is required when twilio is enabled".into(),
                ));
            }
            if self.channels.twilio.auth_token.is_empty() {
                return Err(NanobotError::Config(
                    "channels.twilio.authToken is required when twilio is enabled".into(),
                ));
            }
            if self.channels.twilio.webhook_url.is_empty() {
                return Err(NanobotError::Config(
                    "channels.twilio.webhookUrl is required when twilio is enabled".into(),
                ));
            }
            if self.channels.twilio.webhook_port == 0 {
                return Err(NanobotError::Config(
                    "channels.twilio.webhookPort must be > 0 when twilio is enabled".into(),
                ));
            }
        }

        // Validate web search config
        if self.tools.web.search.max_results == 0 {
            return Err(NanobotError::Config(
                "tools.web.search.maxResults must be > 0".into(),
            ));
        }
        if self.tools.web.search.max_results > 100 {
            warn!("tools.web.search.maxResults is very large (> 100), this may be slow");
        }

        Ok(())
    }

    pub fn get_api_key(&self, model: Option<&str>) -> Option<&str> {
        let model = model.unwrap_or(&self.agents.defaults.model);
        self.providers.get_api_key(model)
    }

    /// Create an LLM provider instance based on configuration.
    ///
    /// Uses a strategy pattern to select the appropriate provider based on model name.
    pub async fn create_provider(
        &self,
        model: Option<&str>,
    ) -> anyhow::Result<std::sync::Arc<dyn crate::providers::base::LLMProvider>> {
        use crate::providers::strategy::ProviderFactory;

        let model = model.unwrap_or(&self.agents.defaults.model);
        let factory = ProviderFactory::new(self);
        factory.create_provider(model).await
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
}
