// mod feeds;
// mod articles;
// mod scanner;
mod onboard;
// mod model;

#[cfg(test)]
mod tests;

use crate::actions;
use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory};
use crate::agent::tools::{Tool, ToolResult};
use crate::config::RssConfig;
use crate::cron::service::CronService;
use crate::require_param;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub struct RssTool {
    pub db: Arc<MemoryDB>,
    pub client: Client,
    pub config: RssConfig,
    pub cron_service: Option<Arc<CronService>>,
}

impl RssTool {
    pub fn new(
        db: Arc<MemoryDB>,
        config: RssConfig,
        cron_service: Option<Arc<CronService>>,
    ) -> Self {
        let timeout = config.scan_timeout;
        Self {
            db,
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(timeout))
                .build()
                .unwrap_or_else(|_| Client::new()),
            config,
            cron_service,
        }
    }

    #[cfg(test)]
    fn new_for_test() -> Self {
        let db = Arc::new(MemoryDB::new(":memory:").unwrap());
        Self::new(db, RssConfig::default(), None)
    }
}

#[async_trait]
impl Tool for RssTool {
    fn name(&self) -> &'static str {
        "rss"
    }

    fn description(&self) -> &'static str {
        "Manage RSS feeds and personalised article recommendations. Actions: onboard, set_profile, \
         add_feed, remove_feed, list_feeds, scan, get_articles, accept, reject, \
         get_article_detail, feed_stats."
    }

    fn cacheable(&self) -> bool {
        false
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_mins(5)
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: actions![
                onboard,
                set_profile,
                add_feed,
                remove_feed,
                list_feeds: ro,
                scan,
                get_articles: ro,
                accept,
                reject,
                get_article_detail: ro,
                feed_stats: ro,
            ],
            category: ToolCategory::Web,
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "onboard", "set_profile", "add_feed", "remove_feed", "list_feeds",
                        "scan", "get_articles", "accept", "reject",
                        "get_article_detail", "feed_stats"
                    ],
                    "description": "Action to perform"
                },
                "url": {
                    "type": "string",
                    "description": "Feed URL (for add_feed)"
                },
                "name": {
                    "type": "string",
                    "description": "Human-readable feed name (for add_feed)"
                },
                "interests": {
                    "type": "string",
                    "description": "Free-text description of the user's interests (for onboard, set_profile)"
                },
                "feed_id": {
                    "type": "integer",
                    "description": "Feed ID (for remove_feed, feed_stats)"
                },
                "article_ids": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "List of article IDs (for accept, reject)"
                },
                "article_id": {
                    "type": "integer",
                    "description": "Single article ID (for get_article_detail)"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "accepted", "rejected"],
                    "description": "Filter articles by status (for get_articles)"
                },
                "limit": {
                    "type": "integer",
                    "default": 20,
                    "description": "Maximum number of results to return"
                },
                "offset": {
                    "type": "integer",
                    "default": 0,
                    "description": "Pagination offset"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = require_param!(params, "action");

        if let Some(err) = onboard::check_gate(&self.db, action)? {
            return Ok(err);
        }

        match action {
            "onboard" => onboard::handle_onboard(&self.db),
            "set_profile" => {
                let interests = require_param!(params, "interests");
                onboard::handle_set_profile(&self.db, interests)
            }
            "add_feed" | "remove_feed" | "list_feeds" | "scan" | "get_articles" | "accept"
            | "reject" | "get_article_detail" | "feed_stats" => {
                Ok(ToolResult::error("not yet implemented".to_string()))
            }
            other => Ok(ToolResult::error(format!("unknown action: {other}"))),
        }
    }
}
