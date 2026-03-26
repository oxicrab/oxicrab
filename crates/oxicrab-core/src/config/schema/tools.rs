use serde::{Deserialize, Serialize};

use super::channels::DenyByDefaultList;
use super::default_true;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExfiltrationGuardConfig {
    /// Disabled by default. When enabled with an empty `allow_tools` list,
    /// ALL network-outbound tools are hidden from the LLM — this includes
    /// cron, calendar, mail, tasks, github, todoist, weather, web search,
    /// rss, and most other useful tools. Only enable this when you have
    /// explicitly configured `allow_tools`.
    #[serde(default)]
    pub enabled: bool,
    /// Force-allow specific network-outbound tools when guard is enabled.
    /// Empty = deny all network-outbound tools (secure default).
    /// Use `["*"]` to allow all, or list specific tool names.
    #[serde(default, rename = "allowTools")]
    pub allow_tools: DenyByDefaultList,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GoogleConfig {
    #[serde(default, rename = "clientId")]
    pub client_id: String,
    #[serde(default, rename = "clientSecret")]
    pub client_secret: String,
    #[serde(default = "default_true")]
    pub gmail: bool,
    #[serde(default = "default_true")]
    pub calendar: bool,
    #[serde(default = "default_true")]
    pub tasks: bool,
}

redact_debug!(
    GoogleConfig,
    client_id,
    redact(client_secret),
    gmail,
    calendar,
    tasks,
);

impl Default for GoogleConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: String::new(),
            gmail: true,
            calendar: true,
            tasks: true,
        }
    }
}

impl GoogleConfig {
    pub fn is_configured(&self) -> bool {
        !self.client_id.is_empty() && !self.client_secret.is_empty()
    }

    pub fn any_tool_enabled(&self) -> bool {
        self.gmail || self.calendar || self.tasks
    }

    pub fn required_scopes(&self) -> Vec<String> {
        let mut scopes = Vec::new();
        if self.gmail {
            scopes.push("https://www.googleapis.com/auth/gmail.modify".to_string());
            scopes.push("https://www.googleapis.com/auth/gmail.send".to_string());
        }
        if self.calendar {
            scopes.push("https://www.googleapis.com/auth/calendar.events".to_string());
            scopes.push("https://www.googleapis.com/auth/calendar.readonly".to_string());
        }
        if self.tasks {
            scopes.push("https://www.googleapis.com/auth/tasks".to_string());
        }
        scopes
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchProvider {
    #[default]
    Brave,
    #[serde(alias = "ddg")]
    Duckduckgo,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct WebSearchConfig {
    #[serde(default)]
    pub provider: SearchProvider,
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
    #[serde(default = "default_max_results", rename = "maxResults")]
    pub max_results: usize,
}

redact_debug!(WebSearchConfig, provider, redact(api_key), max_results,);

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            provider: SearchProvider::default(),
            api_key: String::new(),
            max_results: default_max_results(),
        }
    }
}

fn default_max_results() -> usize {
    5
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

/// Shell command allowlist. Empty = no restrictions (all commands allowed).
/// Non-empty = only listed commands may execute.
/// Default is a comprehensive list of safe commands.
///
/// **Polarity note**: this is the opposite of [`DenyByDefaultList`] where
/// empty means *deny all*. Here empty means *unrestricted* because the
/// default is a populated list of ~70 safe commands — clearing it is an
/// explicit operator choice to remove restrictions. Use [`is_restricted()`]
/// to check whether the list is enforced, and [`is_allowed()`] to check
/// individual commands.
///
/// [`DenyByDefaultList`]: super::channels::DenyByDefaultList
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AllowedCommands(Vec<String>);

impl AllowedCommands {
    pub fn new(commands: Vec<String>) -> Self {
        Self(commands)
    }

    /// Check if a command is allowed. Empty list = unrestricted.
    pub fn is_allowed(&self, command: &str) -> bool {
        self.0.is_empty() || self.0.iter().any(|a| a == command)
    }

    /// Returns true if restrictions are in effect.
    pub fn is_restricted(&self) -> bool {
        !self.0.is_empty()
    }

    pub fn entries(&self) -> &[String] {
        &self.0
    }

    /// Merge additional commands into this list, deduplicating.
    pub fn merge(&self, additional: &[String]) -> Self {
        let mut cmds = self.0.clone();
        for cmd in additional {
            if !cmds.contains(cmd) {
                cmds.push(cmd.clone());
            }
        }
        Self(cmds)
    }
}

impl Default for AllowedCommands {
    fn default() -> Self {
        Self(default_allowed_commands())
    }
}

fn default_allowed_commands_wrapped() -> AllowedCommands {
    AllowedCommands::default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecToolConfig {
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(
        default = "default_allowed_commands_wrapped",
        rename = "allowedCommands"
    )]
    pub allowed_commands: AllowedCommands,
    #[serde(
        default,
        rename = "additionalAllowedCommands",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub additional_allowed_commands: Vec<String>,
    #[serde(default)]
    pub sandbox: SandboxConfig,
}

impl ExecToolConfig {
    /// Returns the effective allowed commands list: defaults (or overrides) merged
    /// with any additional commands, deduplicated.
    pub fn effective_allowed_commands(&self) -> AllowedCommands {
        self.allowed_commands
            .merge(&self.additional_allowed_commands)
    }
}

impl Default for ExecToolConfig {
    fn default() -> Self {
        Self {
            timeout: default_timeout(),
            allowed_commands: AllowedCommands::default(),
            additional_allowed_commands: Vec::new(),
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
        "journalctl",
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
    pub api_url: Option<super::providers::HttpUrl>,
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
            api_url: None,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageGenProvider {
    #[default]
    Openai,
    Google,
    #[serde(alias = "gemini")]
    Gemini,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ImageGenConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "defaultProvider")]
    pub default_provider: ImageGenProvider,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: std::collections::HashMap<String, McpServerConfig>,
}

/// Trust level for MCP servers.
///
/// - `Local`: full access, no approval required
/// - `Verified`: requires approval for each tool call
/// - `Community`: read-only safe tools only (filtered by keyword)
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTrust {
    Local,
    #[default]
    Verified,
    Community,
}

impl std::fmt::Display for McpTrust {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local => write!(f, "local"),
            Self::Verified => write!(f, "verified"),
            Self::Community => write!(f, "community"),
        }
    }
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
    /// (requires approval, default), or "community" (read-only safe tools only).
    #[serde(default)]
    pub trust: McpTrust,
    /// Landlock sandbox config for the MCP server child process.
    /// Defaults to enabled with network blocked (same as shell tool).
    #[serde(default)]
    pub sandbox: SandboxConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_rss_scan_timeout", rename = "scanTimeout")]
    pub scan_timeout: u64,
    #[serde(default = "default_rss_max_articles", rename = "maxArticlesPerFeed")]
    pub max_articles_per_feed: usize,
    #[serde(default = "default_rss_purge_days", rename = "purgeDays")]
    pub purge_days: u64,
    #[serde(default = "default_rss_candidates", rename = "candidatesPerScan")]
    pub candidates_per_scan: usize,
    #[serde(
        default = "default_rss_covariance_inflation",
        rename = "covarianceInflation"
    )]
    pub covariance_inflation: f64,
}

impl Default for RssConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scan_timeout: 15,
            max_articles_per_feed: 50,
            purge_days: 90,
            candidates_per_scan: 20,
            covariance_inflation: 0.01,
        }
    }
}

fn default_rss_scan_timeout() -> u64 {
    15
}

fn default_rss_max_articles() -> usize {
    50
}

fn default_rss_purge_days() -> u64 {
    90
}

fn default_rss_candidates() -> usize {
    20
}

fn default_rss_covariance_inflation() -> f64 {
    0.01
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsConfig {
    #[serde(default, rename = "webSearch")]
    pub web_search: WebSearchConfig,
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
    #[serde(default)]
    pub rss: RssConfig,
}
