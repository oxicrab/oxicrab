pub mod loader;
pub mod schema;

pub use loader::{load_config, save_config, get_config_path};
pub use schema::{
    Config,
    TelegramConfig, DiscordConfig, SlackConfig, WhatsAppConfig, GoogleConfig,
    CompactionConfig,
};
