pub mod loader;
pub mod schema;

pub use loader::{get_config_path, load_config, save_config};
pub use schema::{
    BrowserConfig, ChannelsConfig, CompactionConfig, Config, DiscordConfig, GitHubConfig,
    GoogleConfig, ImageGenConfig, McpConfig, MediaConfig, MemoryConfig, ObsidianConfig,
    SlackConfig, TelegramConfig, TodoistConfig, TranscriptionConfig, TwilioConfig, VoiceConfig,
    WeatherConfig, WebSearchConfig, WhatsAppConfig,
};
