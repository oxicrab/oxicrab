use serde::{Deserialize, Serialize};

use super::default_true;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExfiltrationGuardConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Force-allow specific `network_outbound` tools when guard is enabled.
    #[serde(default, rename = "allowTools")]
    pub allow_tools: Vec<String>,
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
