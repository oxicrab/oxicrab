use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WhatsAppConfig {
    pub enabled: bool,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramConfig {
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: Vec<String>,
    pub proxy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiscordConfig {
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SlackConfig {
    pub enabled: bool,
    #[serde(default, rename = "botToken")]
    pub bot_token: String,
    #[serde(default, rename = "appToken")]
    pub app_token: String,
    #[serde(default, rename = "allowFrom")]
    pub allow_from: Vec<String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompactionConfig {
    #[serde(default)]
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

fn default_threshold_tokens() -> u32 {
    40000
}

fn default_keep_recent() -> usize {
    10
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default, rename = "triageModel")]
    pub triage_model: Option<String>,
    #[serde(default, rename = "triageProvider")]
    pub triage_provider: Option<String>,
    #[serde(default, rename = "executionModel")]
    pub execution_model: Option<String>,
    #[serde(default, rename = "executionProvider")]
    pub execution_provider: Option<String>,
    #[serde(default = "default_strategy_file", rename = "strategyFile")]
    pub strategy_file: String,
    #[serde(default = "default_max_iterations", rename = "maxIterations")]
    pub max_iterations: usize,
    #[serde(default = "default_cooldown", rename = "cooldownAfterAction")]
    pub cooldown_after_action: u64,
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

fn default_cooldown() -> u64 {
    600
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
    #[serde(default, rename = "apiBase")]
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    18790
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

fn default_google_scopes() -> Vec<String> {
    vec![
        "https://www.googleapis.com/auth/gmail.modify".to_string(),
        "https://www.googleapis.com/auth/gmail.send".to_string(),
        "https://www.googleapis.com/auth/calendar.events".to_string(),
        "https://www.googleapis.com/auth/calendar.readonly".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebSearchConfig {
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
    #[serde(default = "default_max_results", rename = "maxResults")]
    pub max_results: usize,
}

fn default_max_results() -> usize {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebToolsConfig {
    #[serde(default)]
    pub search: WebSearchConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecToolConfig {
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_timeout() -> u64 {
    60
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agents: AgentsConfig {
                defaults: AgentDefaults {
                    workspace: default_workspace(),
                    model: default_model(),
                    max_tokens: default_max_tokens(),
                    temperature: default_temperature(),
                    max_tool_iterations: default_max_tool_iterations(),
                    compaction: CompactionConfig {
                        enabled: true,
                        threshold_tokens: default_threshold_tokens(),
                        keep_recent: default_keep_recent(),
                        extraction_enabled: true,
                        model: None,
                    },
                    daemon: DaemonConfig {
                        enabled: true,
                        interval: default_interval(),
                        triage_model: None,
                        triage_provider: None,
                        execution_model: None,
                        execution_provider: None,
                        strategy_file: default_strategy_file(),
                        max_iterations: default_max_iterations(),
                        cooldown_after_action: default_cooldown(),
                    },
                },
            },
            channels: ChannelsConfig {
                whatsapp: WhatsAppConfig {
                    enabled: false,
                    allow_from: vec![],
                },
                telegram: TelegramConfig {
                    enabled: false,
                    token: String::new(),
                    allow_from: vec![],
                    proxy: None,
                },
                discord: DiscordConfig {
                    enabled: false,
                    token: String::new(),
                    allow_from: vec![],
                },
                slack: SlackConfig {
                    enabled: false,
                    bot_token: String::new(),
                    app_token: String::new(),
                    allow_from: vec![],
                },
            },
            providers: ProvidersConfig {
                anthropic: ProviderConfig {
                    api_key: String::new(),
                    api_base: None,
                },
                anthropic_oauth: AnthropicOAuthConfig {
                    enabled: false,
                    access_token: String::new(),
                    refresh_token: String::new(),
                    expires_at: 0,
                    credentials_path: None,
                    auto_detect: true,
                },
                openai: ProviderConfig {
                    api_key: String::new(),
                    api_base: None,
                },
                openrouter: ProviderConfig {
                    api_key: String::new(),
                    api_base: None,
                },
                deepseek: ProviderConfig {
                    api_key: String::new(),
                    api_base: None,
                },
                groq: ProviderConfig {
                    api_key: String::new(),
                    api_base: None,
                },
                zhipu: ProviderConfig {
                    api_key: String::new(),
                    api_base: None,
                },
                dashscope: ProviderConfig {
                    api_key: String::new(),
                    api_base: None,
                },
                vllm: ProviderConfig {
                    api_key: String::new(),
                    api_base: None,
                },
                gemini: ProviderConfig {
                    api_key: String::new(),
                    api_base: None,
                },
                moonshot: ProviderConfig {
                    api_key: String::new(),
                    api_base: None,
                },
            },
            gateway: GatewayConfig {
                host: default_host(),
                port: default_port(),
            },
            tools: ToolsConfig {
                web: WebToolsConfig {
                    search: WebSearchConfig {
                        api_key: String::new(),
                        max_results: default_max_results(),
                    },
                },
                exec: ExecToolConfig {
                    timeout: default_timeout(),
                },
                restrict_to_workspace: false,
                google: GoogleConfig {
                    enabled: false,
                    client_id: String::new(),
                    client_secret: String::new(),
                    scopes: default_google_scopes(),
                },
            },
        }
    }
}

impl Config {
    pub fn workspace_path(&self) -> PathBuf {
        crate::utils::get_workspace_path(&self.agents.defaults.workspace)
    }

    pub fn get_api_key(&self, model: Option<&str>) -> Option<String> {
        let model = model.unwrap_or(&self.agents.defaults.model).to_lowercase();

        // Match provider by model name
        if model.contains("openrouter") && !self.providers.openrouter.api_key.is_empty() {
            return Some(self.providers.openrouter.api_key.clone());
        }
        if model.contains("deepseek") && !self.providers.deepseek.api_key.is_empty() {
            return Some(self.providers.deepseek.api_key.clone());
        }
        if (model.contains("anthropic") || model.contains("claude"))
            && !self.providers.anthropic.api_key.is_empty()
        {
            return Some(self.providers.anthropic.api_key.clone());
        }
        if (model.contains("openai") || model.contains("gpt"))
            && !self.providers.openai.api_key.is_empty()
        {
            return Some(self.providers.openai.api_key.clone());
        }
        if model.contains("gemini") && !self.providers.gemini.api_key.is_empty() {
            return Some(self.providers.gemini.api_key.clone());
        }

        // Fallback: first available key
        if !self.providers.openrouter.api_key.is_empty() {
            return Some(self.providers.openrouter.api_key.clone());
        }
        if !self.providers.anthropic.api_key.is_empty() {
            return Some(self.providers.anthropic.api_key.clone());
        }
        if !self.providers.openai.api_key.is_empty() {
            return Some(self.providers.openai.api_key.clone());
        }
        if !self.providers.gemini.api_key.is_empty() {
            return Some(self.providers.gemini.api_key.clone());
        }

        None
    }

    pub fn get_api_base(&self, model: Option<&str>) -> Option<String> {
        let model = model.unwrap_or(&self.agents.defaults.model).to_lowercase();

        if model.contains("openrouter") {
            return Some(
                self.providers
                    .openrouter
                    .api_base
                    .clone()
                    .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string()),
            );
        }
        if model.contains("zhipu") && self.providers.zhipu.api_base.is_some() {
            return self.providers.zhipu.api_base.clone();
        }
        if model.contains("vllm") && self.providers.vllm.api_base.is_some() {
            return self.providers.vllm.api_base.clone();
        }

        None
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
