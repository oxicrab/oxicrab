pub mod credentials;
pub mod loader;
pub mod routing;
pub mod schema;

pub use loader::{get_config_path, load_config, save_config};
pub use schema::{
    A2aConfig, AgentDefaults, AgentsConfig, AllowedCommands, AnthropicOAuthConfig, ApprovalConfig,
    ApprovalScope, BrowserConfig, ChannelTarget, ChannelsConfig, ChatModels, ChatRoutingConfig,
    ChatThresholds, CircuitBreakerConfig, CognitiveConfig, CompactionConfig, Config,
    ContextProviderConfig, CredentialHelperConfig, DenyByDefaultList, DiscordCommand,
    DiscordCommandOption, DiscordConfig, DmPolicy, ExecToolConfig, ExfiltrationGuardConfig,
    FusionStrategy, GatewayConfig, GitHubConfig, GoogleConfig, HttpUrl, ImageGenConfig, McpConfig,
    McpTrust, MediaConfig, MemoryConfig, ModelRoutingConfig, ObsidianConfig, PromptGuardAction,
    PromptGuardConfig, ProviderConfig, ProvidersConfig, RouterConfig, RssConfig, SandboxConfig,
    SlackConfig, TaskRouting, TelegramConfig, TodoistConfig, ToolsConfig, TranscriptionConfig,
    TwilioConfig, VoiceConfig, WeatherConfig, WebSearchConfig, WebhookConfig, WebhookTarget,
    WhatsAppConfig, WorkspaceTtlConfig, infer_provider_from_model, normalize_provider,
    parse_model_ref,
};
