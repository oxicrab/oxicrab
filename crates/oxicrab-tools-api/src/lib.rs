//! API tools for the oxicrab framework.
//!
//! This crate provides GitHub, Weather, Todoist, Media, and ImageGen tools,
//! extracted from the main binary crate for modularity.

pub mod github;
pub mod image_gen;
pub mod media;
pub mod todoist;
mod utils;
pub mod weather;

use oxicrab_core::config::schema::MediaConfig;
use oxicrab_core::tools::base::Tool;
use std::sync::Arc;

/// Create the GitHub tool.
pub fn create_github_tool(token: String) -> Arc<dyn Tool> {
    Arc::new(github::GitHubTool::new(token))
}

/// Create the Weather tool.
pub fn create_weather_tool(api_key: String) -> Arc<dyn Tool> {
    Arc::new(weather::WeatherTool::new(api_key))
}

/// Create the Todoist tool.
pub fn create_todoist_tool(token: String) -> Arc<dyn Tool> {
    Arc::new(todoist::TodoistTool::new(token))
}

/// Create the Media tool (Radarr/Sonarr).
pub fn create_media_tool(config: &MediaConfig) -> Arc<dyn Tool> {
    Arc::new(media::MediaTool::new(config))
}

/// Create the Image Generation tool.
pub fn create_image_gen_tool(
    openai_api_key: Option<String>,
    google_api_key: Option<String>,
    default_provider: String,
) -> Arc<dyn Tool> {
    Arc::new(image_gen::ImageGenTool::new(
        openai_api_key,
        google_api_key,
        default_provider,
    ))
}
