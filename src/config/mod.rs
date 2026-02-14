pub mod loader;
pub mod schema;

pub use loader::{get_config_path, load_config, save_config};
pub use schema::{
    ChannelsConfig, CompactionConfig, Config, DiscordConfig, GitHubConfig, GoogleConfig,
    MediaConfig, ObsidianConfig, SlackConfig, TelegramConfig, TodoistConfig, TranscriptionConfig,
    TwilioConfig, VoiceConfig, WeatherConfig, WebSearchConfig, WhatsAppConfig,
};
