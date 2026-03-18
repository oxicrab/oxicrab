//! Web tools for the oxicrab framework.
//!
//! This crate provides HTTP, Reddit, WebSearch, and WebFetch tools,
//! extracted from the main binary crate for modularity.

pub mod http;
pub mod reddit;
mod utils;
pub mod web;

use oxicrab_core::config::schema::WebSearchConfig;
use oxicrab_core::tools::base::Tool;
use std::sync::Arc;

/// Create the HTTP tool.
pub fn create_http_tool() -> Arc<dyn Tool> {
    Arc::new(http::HttpTool::new())
}

/// Create the Reddit tool.
pub fn create_reddit_tool() -> Arc<dyn Tool> {
    Arc::new(reddit::RedditTool::new())
}

/// Create web tools (WebSearch + WebFetch).
///
/// If `config` is provided, uses it for WebSearch configuration;
/// otherwise uses defaults (DuckDuckGo fallback with 5 results).
pub fn create_web_tools(config: Option<&WebSearchConfig>) -> Vec<Arc<dyn Tool>> {
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();

    let search = if let Some(ws_cfg) = config {
        web::WebSearchTool::from_config(ws_cfg)
    } else {
        web::WebSearchTool::new(None, 5)
    };
    tools.push(Arc::new(search));

    if let Ok(fetch) = web::WebFetchTool::new(50000) {
        tools.push(Arc::new(fetch));
    }

    tools
}
