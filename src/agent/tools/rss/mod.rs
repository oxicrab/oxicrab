mod articles;
mod feeds;
pub(crate) mod model;
mod onboard;
mod scanner;
mod stats;

#[cfg(test)]
mod tests;

/// Format a millisecond timestamp as a human-readable date (UTC, no external deps).
pub(super) fn format_date_ms(ms: i64) -> String {
    // Convert ms → seconds, then days since Unix epoch
    let days = ms / 1000 / 86400;
    // Gregorian calendar algorithm (Richards, 2013 — via Astronomical Algorithms)
    let jdn = days + 2_440_588; // JDN for Unix epoch day 0
    let century = (4 * jdn + 3) / 146_097;
    let day_in_century = jdn - 146_097 * century / 4;
    let year_in_century = (4 * day_in_century + 3) / 1_461;
    let day_in_year = day_in_century - 1_461 * year_in_century / 4;
    let month_index = (5 * day_in_year + 2) / 153;
    let day = day_in_year - (153 * month_index + 2) / 5 + 1;
    let month = month_index + 3 - 12 * (month_index / 10);
    let year = 100 * century + year_in_century - 4_800 + month_index / 10;
    format!("{year:04}-{month:02}-{day:02}")
}

use std::time::{SystemTime, UNIX_EPOCH};

/// Shared timestamp helper — single source of truth for all RSS submodules.
pub(super) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

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
         add_feed, remove_feed, enable_feed, list_feeds, scan, next (review one article with buttons), \
         get_articles, accept, reject, get_article_detail, feed_stats."
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
                enable_feed,
                list_feeds: ro,
                scan,
                next: ro,
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
                        "onboard", "set_profile", "add_feed", "remove_feed", "enable_feed",
                        "list_feeds", "scan", "next", "get_articles", "accept", "reject",
                        "get_article_detail", "feed_stats"
                    ],
                    "description": "Action to perform. To review/accept/reject articles, \
                     use 'next' — it shows one article at a time with Accept/Reject buttons. \
                     Use 'get_articles' only to browse or list articles without reviewing."
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
                    "type": "string",
                    "description": "Feed ID (for remove_feed, enable_feed, feed_stats; optional filter for get_articles)"
                },
                "article_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of article IDs (for accept, reject)"
                },
                "article_id": {
                    "type": "string",
                    "description": "Single article ID (for get_article_detail)"
                },
                "status": {
                    "type": "string",
                    "enum": ["new", "accepted", "rejected"],
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

    async fn execute(&self, params: Value, ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = require_param!(params, "action");

        if let Some(err) = onboard::check_gate(&self.db, action)? {
            return Ok(err);
        }

        match action {
            "onboard" => onboard::handle_onboard(&self.db, ctx, self.cron_service.as_ref()),
            "set_profile" => {
                let interests = require_param!(params, "interests");
                onboard::handle_set_profile(&self.db, interests)
            }
            "add_feed" => {
                let url = require_param!(params, "url");
                let name = params["name"].as_str();
                feeds::handle_add_feed(&self.db, &self.client, url, name, self.config.scan_timeout)
                    .await
            }
            "remove_feed" => {
                let feed_id = require_param!(params, "feed_id");
                feeds::handle_remove_feed(&self.db, feed_id)
            }
            "enable_feed" => {
                let feed_id = require_param!(params, "feed_id");
                feeds::handle_enable_feed(&self.db, feed_id)
            }
            "list_feeds" => feeds::handle_list_feeds(&self.db),
            "get_articles" => {
                let status = params["status"].as_str();
                let feed_id = params["feed_id"].as_str();
                let limit = (params["limit"].as_u64().unwrap_or(20) as usize).min(500);
                let offset = (params["offset"].as_u64().unwrap_or(0) as usize).min(10_000);
                articles::handle_get_articles(&self.db, status, feed_id, limit, offset)
            }
            "accept" => {
                let ids = extract_article_ids(&params);
                Ok(articles::handle_accept(&self.db, &ids))
            }
            "reject" => {
                let ids = extract_article_ids(&params);
                Ok(articles::handle_reject(&self.db, &ids))
            }
            "get_article_detail" => {
                let id = require_param!(params, "article_id");
                articles::handle_get_article_detail(&self.db, &self.client, id).await
            }
            "scan" => scanner::handle_scan(&self.db, &self.client, &self.config).await,
            "next" => articles::handle_next(&self.db),
            "feed_stats" => stats::handle_feed_stats(&self.db),
            other => Ok(ToolResult::error(format!("unknown action: {other}"))),
        }
    }
}

fn extract_article_ids(params: &Value) -> Vec<&str> {
    if let Some(arr) = params["article_ids"].as_array() {
        arr.iter().filter_map(|v| v.as_str()).collect()
    } else if let Some(id) = params["article_id"].as_str() {
        vec![id]
    } else {
        vec![]
    }
}
