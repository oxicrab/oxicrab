//! Browser tool for the oxicrab framework.

pub mod browser;
mod utils;

use oxicrab_core::config::schema::BrowserConfig;
use oxicrab_core::tools::base::Tool;
use std::sync::Arc;

/// Create the browser tool.
pub fn create_browser_tool(config: &BrowserConfig) -> Arc<dyn Tool> {
    Arc::new(browser::BrowserTool::new(config))
}
