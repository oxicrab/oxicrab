pub mod loader;
pub mod schema;

pub use loader::{get_config_path, load_config, save_config};
pub use schema::{
    ChannelsConfig, CompactionConfig, Config, DiscordConfig, GitHubConfig, GoogleConfig,
    SlackConfig, TelegramConfig, TodoistConfig, WeatherConfig, WhatsAppConfig,
};
