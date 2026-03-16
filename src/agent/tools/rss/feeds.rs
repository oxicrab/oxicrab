use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use reqwest::Client;
use std::time::Duration;
use tracing::{info, warn};

use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::memory::memory_db::rss::{RssFeed, STATE_NEEDS_CALIBRATION, STATE_NEEDS_FEEDS};
use crate::agent::tools::ToolResult;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

pub async fn handle_add_feed(
    db: &MemoryDB,
    client: &Client,
    url: &str,
    name: Option<&str>,
    timeout: u64,
) -> Result<ToolResult> {
    // 1. Validate URL via SSRF guard
    if let Err(e) = crate::utils::url_security::validate_and_resolve(url).await {
        return Ok(ToolResult::error(format!("invalid feed URL: {e}")));
    }

    // 2. Fetch the feed
    let response = match client
        .get(url)
        .timeout(Duration::from_secs(timeout))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Ok(ToolResult::error(format!("failed to fetch feed: {e}")));
        }
    };

    // 3. Check HTTP status
    if !response.status().is_success() {
        return Ok(ToolResult::error(format!(
            "feed URL returned HTTP {}: {}",
            response.status().as_u16(),
            response.status().canonical_reason().unwrap_or("error")
        )));
    }

    // 4. Read body as text
    let body = match response.text().await {
        Ok(b) => b,
        Err(e) => {
            return Ok(ToolResult::error(format!("failed to read feed body: {e}")));
        }
    };

    // 5. Parse with feed_rs
    let Ok(parsed) = feed_rs::parser::parse(body.as_bytes()) else {
        return Ok(ToolResult::error(
            "URL returned valid HTTP but content is not RSS or Atom",
        ));
    };

    let entry_count = parsed.entries.len();

    // 6. Extract feed name
    let feed_name = name
        .map(str::to_owned)
        .or_else(|| parsed.title.map(|t| t.content))
        .unwrap_or_else(|| url.to_owned());

    // 7. Extract site_url from feed links
    let site_url = parsed.links.first().map(|l| l.href.clone());

    // 8. Create RssFeed with a UUID
    let feed = RssFeed {
        id: uuid::Uuid::new_v4().to_string(),
        url: url.to_owned(),
        name: feed_name.clone(),
        site_url,
        last_fetched_at_ms: None,
        last_error: None,
        consecutive_failures: 0,
        enabled: true,
        created_at_ms: now_ms(),
    };

    // 9. Insert into DB — catch UNIQUE constraint violations
    if let Err(e) = db.insert_rss_feed(&feed) {
        let msg = e.to_string();
        if msg.contains("UNIQUE") || msg.contains("unique") {
            return Ok(ToolResult::error("a feed with this URL already exists"));
        }
        return Err(e);
    }

    info!("rss feed added: {} ({})", feed_name, url);

    // 10. If profile state is "needs_feeds", transition to "needs_calibration"
    if let Ok(Some(profile)) = db.get_rss_profile()
        && profile.onboarding_state == STATE_NEEDS_FEEDS
        && let Err(e) = db.set_rss_onboarding_state(STATE_NEEDS_CALIBRATION, now_ms())
    {
        warn!("failed to advance onboarding state: {e}");
    }

    // 11. Return success
    Ok(ToolResult::new(format!(
        "Feed added successfully.\n\nName: {feed_name}\nURL: {url}\nEntries in feed: {entry_count}\n\n\
         Use 'scan' to fetch new articles, or 'get_articles' to browse existing ones."
    )))
}

pub fn handle_remove_feed(db: &MemoryDB, feed_id: &str) -> Result<ToolResult> {
    // 1. Find the feed matching feed_id
    let feeds = db.list_rss_feeds()?;
    let Some(feed) = feeds.iter().find(|f| f.id == feed_id) else {
        return Ok(ToolResult::error(format!(
            "no feed found with id '{feed_id}' — use list_feeds to see available feeds"
        )));
    };

    // 2. During calibration, enforce >= 1 feed remains
    if let Ok(Some(profile)) = db.get_rss_profile()
        && profile.onboarding_state == STATE_NEEDS_CALIBRATION
        && feeds.len() <= 1
    {
        return Ok(ToolResult::error(
            "cannot remove the last feed during calibration — at least one feed is required",
        ));
    }

    // 3. Count accepted articles for warning
    let accepted_articles = db.get_rss_articles(Some("accepted"), Some(feed_id), usize::MAX, 0)?;
    let accepted_count = accepted_articles.len();

    // 4. Delete the feed (cascades to articles)
    db.delete_rss_feed(feed_id)?;
    info!("rss feed removed: {} ({})", feed.name, feed.url);

    // 5. Return success with warning if accepted articles were lost
    let mut msg = format!("Feed '{}' removed.", feed.name);
    if accepted_count > 0 {
        let _ = write!(
            msg,
            "\n\nNote: {accepted_count} accepted article(s) from this feed were also deleted."
        );
    }

    Ok(ToolResult::new(msg))
}

pub fn handle_list_feeds(db: &MemoryDB) -> Result<ToolResult> {
    let feeds = db.list_rss_feeds()?;

    if feeds.is_empty() {
        return Ok(ToolResult::new(
            "No feeds configured. Use 'add_feed' to add one.",
        ));
    }

    let mut out = format!("Feeds ({}):\n\n", feeds.len());
    for feed in &feeds {
        let short_id: String = feed.id.chars().take(8).collect();
        let status = if feed.enabled { "enabled" } else { "disabled" };
        let _ = write!(
            out,
            "• {} [id: {}]\n  URL: {}\n  Status: {}",
            feed.name, short_id, feed.url, status
        );
        if feed.consecutive_failures > 0 {
            let _ = write!(out, ", failures: {}", feed.consecutive_failures);
        }
        if let Some(ref err) = feed.last_error {
            let _ = write!(out, "\n  Last error: {err}");
        }
        out.push('\n');
    }

    Ok(ToolResult::new(out))
}
