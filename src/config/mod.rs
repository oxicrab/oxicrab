pub mod loader;
pub mod schema;

pub use loader::{load_config, save_config, get_config_path};
pub use schema::{
    Config, AgentsConfig, ChannelsConfig, ProvidersConfig, GatewayConfig, ToolsConfig,
    TelegramConfig, DiscordConfig, SlackConfig, WhatsAppConfig, GoogleConfig,
    CompactionConfig, DaemonConfig, AgentDefaults,
};
