pub mod credentials;
pub mod loader;
pub mod schema;
pub mod watcher;

pub use loader::{get_config_path, load_config, save_config};
pub use schema::{
    AgentDefaults, AgentsConfig, AnthropicOAuthConfig, BrowserConfig, ChannelsConfig,
    CheckpointConfig, CircuitBreakerConfig, CognitiveConfig, CompactionConfig, Config,
    CostGuardConfig, CredentialHelperConfig, DiscordCommand, DiscordCommandOption, DiscordConfig,
    DmPolicy, ExecToolConfig, ExfiltrationGuardConfig, FusionStrategy, GatewayConfig, GitHubConfig,
    GoogleConfig, ImageGenConfig, McpConfig, MediaConfig, MemoryConfig, ModelCost, ObsidianConfig,
    PromptGuardAction, PromptGuardConfig, ProviderConfig, ProvidersConfig, SandboxConfig,
    SlackConfig, TelegramConfig, TodoistConfig, ToolsConfig, TranscriptionConfig, TwilioConfig,
    VoiceConfig, WeatherConfig, WebSearchConfig, WebhookConfig, WebhookTarget, WhatsAppConfig,
    normalize_provider,
};
