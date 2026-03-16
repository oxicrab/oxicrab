use std::collections::HashMap;
use std::fmt::Write as _;

use anyhow::Result;
use reqwest::Client;
use tracing::warn;

use super::model::LinTSModel;
use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::tools::ToolResult;
use crate::utils::http::limited_text;

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

    // Rank pending/new articles using the model when no specific status filter is applied
    let display_order: Vec<usize> = if status.is_none() || status == Some("new") {
        rank_articles(db, articles)
    } else {
        (0..articles.len()).collect()
    };

    // Build a feed-id → name map for display
    let feeds = db.list_rss_feeds().unwrap_or_default();
    let feed_name_map: HashMap<&str, &str> = feeds
        .iter()
        .map(|f| (f.id.as_str(), f.name.as_str()))
        .collect();

    let mut out = format!("Articles ({}):\n\n", articles.len());
    for &idx in &display_order {
        let article = &articles[idx];
        let short_id: String = article.id.chars().take(8).collect();
        let feed_label = feed_name_map
            .get(article.feed_id.as_str())
            .copied()
            .unwrap_or(&article.feed_id);
        let date_str = article
            .published_at_ms
            .map_or_else(|| "—".to_string(), super::format_date_ms);
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
                update_model_for_article(db, &full_id, accepted);
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

/// Rank articles using the `LinTS` model. Returns indices in display order.
fn rank_articles(
    db: &MemoryDB,
    articles: &[crate::agent::memory::memory_db::rss::RssArticle],
) -> Vec<usize> {
    let mut model = load_or_create_model(db);

    // If the model has no features yet, return natural order
    if model.dimension() == 0 && articles.len() <= 1 {
        return (0..articles.len()).collect();
    }

    let feature_vecs: Vec<_> = articles
        .iter()
        .map(|a| {
            let tags = db.get_rss_article_tags(&a.id).unwrap_or_default();
            model.encode_article(&a.feed_id, &tags, &[])
        })
        .collect();

    model.rank(&feature_vecs)
}

/// Load the `LinTS` model from DB, or create a fresh one.
fn load_or_create_model(db: &MemoryDB) -> LinTSModel {
    match db.load_rss_model() {
        Ok(Some((feature_index, mu, sigma))) => {
            match LinTSModel::from_bytes(&feature_index, &mu, &sigma) {
                Ok(model) => model,
                Err(e) => {
                    warn!("rss model corrupted, creating fresh: {e}");
                    LinTSModel::new()
                }
            }
        }
        Ok(None) => LinTSModel::new(),
        Err(e) => {
            warn!("failed to load rss model: {e}");
            LinTSModel::new()
        }
    }
}

/// Save the `LinTS` model to DB.
fn save_model(db: &MemoryDB, model: &LinTSModel) {
    let (feature_index, mu, sigma) = model.to_bytes();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    if let Err(e) = db.save_rss_model(&feature_index, &mu, &sigma, now_ms) {
        warn!("failed to save rss model: {e}");
    }
}

/// Update the model after article feedback.
///
/// Loads the model, encodes the article features, runs a Bayesian update,
/// and persists the updated model back to DB.
pub(crate) fn update_model_for_article(db: &MemoryDB, article_id: &str, accepted: bool) {
    let Ok(Some(article)) = db.get_rss_article(article_id) else {
        return;
    };

    let tags = db.get_rss_article_tags(article_id).unwrap_or_default();

    let mut model = load_or_create_model(db);
    let x = model.encode_article(&article.feed_id, &tags, &[]);
    model.update(&x, accepted);
    // Small covariance inflation to prevent over-confidence and handle taste drift
    model.inflate_covariance(0.01);
    model.prune_if_needed();
    save_model(db, &model);
}

pub async fn handle_get_article_detail(
    db: &MemoryDB,
    _client: &Client,
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

    // Validate URL via SSRF guard and get pinned addresses to prevent DNS rebinding
    let resolved = match crate::utils::url_security::validate_and_resolve(&article.url).await {
        Ok(r) => r,
        Err(e) => {
            warn!(
                "rss article detail: URL validation failed for {}: {e}",
                article.url
            );
            // Fall back to snippet
            return Ok(fallback_to_snippet(&article));
        }
    };

    // Build a per-request client pinned to the validated addresses
    let pinned = {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30));
        for addr in &resolved.addrs {
            builder = builder.resolve(&resolved.host, *addr);
        }
        match builder.build() {
            Ok(c) => c,
            Err(e) => {
                warn!("rss article detail: failed to build pinned client: {e}");
                return Ok(fallback_to_snippet(&article));
            }
        }
    };

    // Fetch article content
    let fetch_result = pinned.get(&article.url).send().await;

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
