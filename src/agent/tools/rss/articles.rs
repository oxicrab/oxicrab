use std::collections::HashMap;
use std::fmt::Write as _;

use anyhow::Result;
use reqwest::Client;
use tracing::warn;

use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::tools::ToolResult;
use crate::utils::http::limited_text;

/// Format a millisecond timestamp as a human-readable date (UTC, no external deps).
fn format_date_ms(ms: i64) -> String {
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

pub fn handle_get_articles(
    db: &MemoryDB,
    status: Option<&str>,
    feed_id: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<ToolResult> {
    // Fetch one extra to detect whether there are more results
    let fetch_limit = limit.saturating_add(1);
    let articles = db.get_rss_articles(status, feed_id, fetch_limit, offset)?;
    let has_more = articles.len() > limit;
    let articles = &articles[..articles.len().min(limit)];

    if articles.is_empty() {
        return Ok(ToolResult::new("No articles found."));
    }

    // Build a feed-id → name map for display
    let feeds = db.list_rss_feeds().unwrap_or_default();
    let feed_name_map: HashMap<&str, &str> = feeds
        .iter()
        .map(|f| (f.id.as_str(), f.name.as_str()))
        .collect();

    let mut out = format!("Articles ({}):\n\n", articles.len());
    for article in articles {
        let short_id: String = article.id.chars().take(8).collect();
        let feed_label = feed_name_map
            .get(article.feed_id.as_str())
            .copied()
            .unwrap_or(&article.feed_id);
        let date_str = article
            .published_at_ms
            .map_or_else(|| "—".to_string(), format_date_ms);
        let _ = writeln!(
            out,
            "• [{}] {} ({})\n  Feed: {} | Status: {} | Published: {}",
            short_id, article.title, article.url, feed_label, article.status, date_str
        );
    }

    if has_more {
        let next_offset = offset + limit;
        let _ = write!(
            out,
            "\nMore articles available. Use offset={next_offset} to see the next page."
        );
    }

    Ok(ToolResult::new(out))
}

/// Shared logic for accept and reject. `accepted` = true → "accepted", false → "rejected".
fn handle_feedback(db: &MemoryDB, article_ids: &[&str], accepted: bool) -> ToolResult {
    if article_ids.is_empty() {
        return ToolResult::error(
            "no article IDs provided — use article_ids (array) or article_id (single)",
        );
    }

    let target_status = if accepted { "accepted" } else { "rejected" };
    let terminal_states = ["accepted", "rejected"];

    let mut successes: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for &raw_id in article_ids {
        // Resolve short → full ID
        let full_id = match db.resolve_rss_article_id(raw_id) {
            Ok(id) => id,
            Err(e) => {
                errors.push(format!("{raw_id}: {e}"));
                continue;
            }
        };

        // Check article exists and is not already in a terminal state
        match db.get_rss_article(&full_id) {
            Ok(Some(article)) => {
                if terminal_states.contains(&article.status.as_str()) {
                    errors.push(format!("{raw_id}: already {}", article.status));
                    continue;
                }
                if let Err(e) = db.update_rss_article_status(&full_id, target_status) {
                    errors.push(format!("{raw_id}: failed to update status — {e}"));
                    continue;
                }
                let short_id: String = full_id.chars().take(8).collect();
                successes.push(short_id);
            }
            Ok(None) => {
                errors.push(format!("{raw_id}: article not found after resolution"));
            }
            Err(e) => {
                errors.push(format!("{raw_id}: {e}"));
            }
        }
    }

    let mut msg = String::new();

    if !successes.is_empty() {
        let _ = writeln!(msg, "{}: {}", target_status, successes.join(", "));
    }
    if !errors.is_empty() {
        let _ = writeln!(msg, "Errors:");
        for err in &errors {
            let _ = writeln!(msg, "  • {err}");
        }
    }

    let msg = msg.trim_end().to_string();

    // Return error only when everything failed; mixed results count as success
    let is_all_error = successes.is_empty() && !errors.is_empty();
    if is_all_error {
        return ToolResult::error(msg);
    }

    let mut metadata = HashMap::new();
    metadata.insert(
        "suggested_buttons".to_string(),
        serde_json::json!([
            {"id": "rss-next", "label": "Next Article", "style": "primary"},
            {"id": "rss-done", "label": "Done Reviewing", "style": "default"}
        ]),
    );
    ToolResult::new(msg).with_metadata(metadata)
}

pub fn handle_accept(db: &MemoryDB, article_ids: &[&str]) -> ToolResult {
    handle_feedback(db, article_ids, true)
}

pub fn handle_reject(db: &MemoryDB, article_ids: &[&str]) -> ToolResult {
    handle_feedback(db, article_ids, false)
}

pub async fn handle_get_article_detail(
    db: &MemoryDB,
    client: &Client,
    article_id: &str,
) -> Result<ToolResult> {
    // Resolve short → full ID
    let full_id = match db.resolve_rss_article_id(article_id) {
        Ok(id) => id,
        Err(e) => return Ok(ToolResult::error(format!("article not found: {e}"))),
    };

    let Some(article) = db.get_rss_article(&full_id)? else {
        return Ok(ToolResult::error(format!(
            "article '{article_id}' not found"
        )));
    };

    // If full content already cached, return it
    if let Some(ref content) = article.full_content {
        let mut out = format!("**{}**\n{}\n\n{}", article.title, article.url, content);
        if let Some(ref author) = article.author {
            out = format!(
                "**{}** by {}\n{}\n\n{}",
                article.title, author, article.url, content
            );
        }
        return Ok(ToolResult::new(out));
    }

    // Validate URL via SSRF guard before fetching
    if let Err(e) = crate::utils::url_security::validate_and_resolve(&article.url).await {
        warn!(
            "rss article detail: URL validation failed for {}: {e}",
            article.url
        );
        // Fall back to snippet
        return Ok(fallback_to_snippet(&article));
    }

    // Fetch article content
    let fetch_result = client.get(&article.url).send().await;

    match fetch_result {
        Ok(resp) if resp.status().is_success() => {
            match limited_text(resp, 500 * 1024).await {
                Ok(content) => {
                    // Store in DB (also marks as read)
                    if let Err(e) = db.update_rss_article_full_content(&full_id, &content) {
                        warn!("rss article detail: failed to store content for {full_id}: {e}");
                    }
                    let header = if let Some(ref author) = article.author {
                        format!("**{}** by {}\n{}\n\n", article.title, author, article.url)
                    } else {
                        format!("**{}**\n{}\n\n", article.title, article.url)
                    };
                    Ok(ToolResult::new(format!("{header}{content}")))
                }
                Err(e) => {
                    warn!(
                        "rss article detail: failed to read body for {}: {e}",
                        article.url
                    );
                    Ok(fallback_to_snippet(&article))
                }
            }
        }
        Ok(resp) => {
            warn!(
                "rss article detail: HTTP {} for {}",
                resp.status(),
                article.url
            );
            Ok(fallback_to_snippet(&article))
        }
        Err(e) => {
            warn!("rss article detail: fetch failed for {}: {e}", article.url);
            Ok(fallback_to_snippet(&article))
        }
    }
}

fn fallback_to_snippet(article: &crate::agent::memory::memory_db::rss::RssArticle) -> ToolResult {
    let snippet = article
        .description
        .as_deref()
        .unwrap_or("(no description available)");
    let header = if let Some(ref author) = article.author {
        format!("**{}** by {}\n{}\n\n", article.title, author, article.url)
    } else {
        format!("**{}**\n{}\n\n", article.title, article.url)
    };
    ToolResult::new(format!(
        "{header}{snippet}\n\n(Note: full article content could not be fetched — showing snippet only.)"
    ))
}
