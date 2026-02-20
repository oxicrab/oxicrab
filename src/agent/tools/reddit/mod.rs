use crate::agent::tools::base::ExecutionContext;
use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const REDDIT_BASE: &str = "https://www.reddit.com";
const MAX_LIMIT: u64 = 25;

pub struct RedditTool {
    base_url: String,
    client: Client,
}

impl Default for RedditTool {
    fn default() -> Self {
        Self::new()
    }
}

impl RedditTool {
    pub fn new() -> Self {
        Self {
            base_url: REDDIT_BASE.to_string(),
            client: Client::builder()
                .user_agent("oxicrab/1.0")
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }

    #[cfg(test)]
    fn with_base_url(base_url: String) -> Self {
        Self {
            base_url,
            client: Client::builder()
                .user_agent("oxicrab/1.0")
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }

    async fn fetch_listing(
        &self,
        subreddit: &str,
        endpoint: &str,
        limit: u64,
        query_params: &[(&str, &str)],
    ) -> Result<String> {
        let url = format!("{}/r/{}/{}.json", self.base_url, subreddit, endpoint);
        let limit_str = limit.to_string();
        let mut params: Vec<(&str, &str)> = vec![("limit", &limit_str), ("raw_json", "1")];
        params.extend_from_slice(query_params);

        let resp = self.client.get(&url).query(&params).send().await?;

        let status = resp.status();
        if status.as_u16() == 404 {
            anyhow::bail!("Subreddit r/{} not found", subreddit);
        }
        if status.as_u16() == 403 {
            anyhow::bail!("Subreddit r/{} is private or quarantined", subreddit);
        }
        if !status.is_success() {
            anyhow::bail!("Reddit API error: HTTP {}", status);
        }

        let json: Value = resp.json().await?;
        let posts = json["data"]["children"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();

        if posts.is_empty() {
            return Ok(format!("No posts found in r/{}.", subreddit));
        }

        let lines: Vec<String> = posts
            .iter()
            .enumerate()
            .map(|(i, post)| {
                let d = &post["data"];
                let title = d["title"].as_str().unwrap_or("[no title]");
                let score = d["score"].as_i64().unwrap_or(0);
                let comments = d["num_comments"].as_i64().unwrap_or(0);
                let author = d["author"].as_str().unwrap_or("[deleted]");
                let url = d["url"].as_str().unwrap_or("");
                let selftext = d["selftext"].as_str().unwrap_or("");
                let preview: String = if selftext.is_empty() {
                    String::new()
                } else {
                    let truncated: String = selftext.chars().take(150).collect();
                    if selftext.chars().count() > 150 {
                        format!("\n   {}...", truncated)
                    } else {
                        format!("\n   {}", truncated)
                    }
                };

                format!(
                    "{}. {} (score: {}, comments: {}, by u/{})\n   {}{}",
                    i + 1,
                    title,
                    score,
                    comments,
                    author,
                    url,
                    preview
                )
            })
            .collect();

        Ok(lines.join("\n\n"))
    }

    async fn search(&self, subreddit: &str, query: &str, limit: u64) -> Result<String> {
        let url = format!("{}/r/{}/search.json", self.base_url, subreddit);
        let limit_str = limit.to_string();
        let params = [
            ("q", query),
            ("restrict_sr", "on"),
            ("limit", &limit_str),
            ("raw_json", "1"),
        ];

        let resp = self.client.get(&url).query(&params).send().await?;

        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("Reddit search error: HTTP {}", status);
        }

        let json: Value = resp.json().await?;
        let posts = json["data"]["children"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();

        if posts.is_empty() {
            return Ok(format!("No results for '{}' in r/{}.", query, subreddit));
        }

        let lines: Vec<String> = posts
            .iter()
            .enumerate()
            .map(|(i, post)| {
                let d = &post["data"];
                let title = d["title"].as_str().unwrap_or("[no title]");
                let score = d["score"].as_i64().unwrap_or(0);
                let comments = d["num_comments"].as_i64().unwrap_or(0);
                let author = d["author"].as_str().unwrap_or("[deleted]");
                let url = d["url"].as_str().unwrap_or("");

                format!(
                    "{}. {} (score: {}, comments: {}, by u/{})\n   {}",
                    i + 1,
                    title,
                    score,
                    comments,
                    author,
                    url
                )
            })
            .collect();

        Ok(lines.join("\n\n"))
    }
}

#[async_trait]
impl Tool for RedditTool {
    fn name(&self) -> &'static str {
        "reddit"
    }

    fn description(&self) -> &'static str {
        "Browse Reddit. Get hot, new, or top posts from a subreddit, or search within a subreddit. Read-only, no authentication required."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
    }

    fn cacheable(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subreddit": {
                    "type": "string",
                    "description": "Subreddit name without the r/ prefix (e.g. 'rust', 'programming')"
                },
                "action": {
                    "type": "string",
                    "enum": ["hot", "new", "top", "search"],
                    "default": "hot",
                    "description": "What to fetch: hot (trending), new (latest), top (highest scored), or search"
                },
                "limit": {
                    "type": "integer",
                    "default": 10,
                    "description": "Number of posts to return (max 25)"
                },
                "time": {
                    "type": "string",
                    "enum": ["hour", "day", "week", "month", "year", "all"],
                    "default": "day",
                    "description": "Time filter for 'top' action"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (required for 'search' action)"
                }
            },
            "required": ["subreddit"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let subreddit = match params["subreddit"].as_str() {
            Some(s) => s.trim_start_matches("r/"),
            None => {
                return Ok(ToolResult::error(
                    "missing 'subreddit' parameter".to_string(),
                ));
            }
        };

        let action = params["action"].as_str().unwrap_or("hot");
        let limit = params["limit"].as_u64().unwrap_or(10).min(MAX_LIMIT);

        let result = match action {
            "hot" => self.fetch_listing(subreddit, "hot", limit, &[]).await,
            "new" => self.fetch_listing(subreddit, "new", limit, &[]).await,
            "top" => {
                let time = params["time"].as_str().unwrap_or("day");
                self.fetch_listing(subreddit, "top", limit, &[("t", time)])
                    .await
            }
            "search" => {
                let Some(query) = params["query"].as_str() else {
                    return Ok(ToolResult::error(
                        "missing 'query' parameter for search action".to_string(),
                    ));
                };
                self.search(subreddit, query, limit).await
            }
            _ => return Ok(ToolResult::error(format!("unknown action: {}", action))),
        };

        match result {
            Ok(content) => Ok(ToolResult::new(content)),
            Err(e) => Ok(ToolResult::error(format!("Reddit error: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests;
