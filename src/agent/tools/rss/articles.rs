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
    // Resolve short feed ID prefix to full UUID
    let resolved_feed_id: Option<String> = match feed_id {
        Some(id) => match db.resolve_rss_feed_id(id) {
            Ok(full) => Some(full),
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "{e} — use list_feeds to see available feeds"
                )));
            }
        },
        None => None,
    };
    let feed_id = resolved_feed_id.as_deref();

    // When ranking is applied (status is None or "new"), fetch a larger candidate
    // pool so that relevant older articles can bubble up via LinTS ranking.
    // Without this, only the most recent `limit` articles are ever considered.
    let use_ranking = status.is_none() || status == Some("new");
    // Window must cover at least offset+limit so pagination doesn't hit a dead end
    let rank_window = if use_ranking {
        (limit * 3).max(offset + limit + 1)
    } else {
        0
    };

    let fetch_limit = if rank_window > 0 {
        rank_window.max(limit.saturating_add(1))
    } else {
        limit.saturating_add(1)
    };

    // When ranking, fetch from offset 0 so we rank the full candidate pool,
    // then apply pagination after ranking.
    let fetch_offset = if rank_window > 0 { 0 } else { offset };
    let articles = db.get_rss_articles(status, feed_id, fetch_limit, fetch_offset)?;

    if articles.is_empty() {
        return Ok(ToolResult::new("No articles found."));
    }

    // Build a feed-id → name map for display
    let feeds = db.list_rss_feeds().unwrap_or_default();
    let feed_name_map: HashMap<&str, &str> = feeds
        .iter()
        .map(|f| (f.id.as_str(), f.name.as_str()))
        .collect();

    if rank_window > 0 {
        // Rank the full candidate pool, then paginate
        let display_order = rank_articles(db, &articles);
        let page: Vec<usize> = display_order.into_iter().skip(offset).take(limit).collect();
        let total_matching = db.count_rss_articles(status, feed_id)?;
        let has_more = offset + limit < total_matching;

        if page.is_empty() {
            let msg = if has_more {
                "No articles on this page, but more exist. Try a smaller offset or use offset=0."
            } else {
                "No articles found."
            };
            return Ok(ToolResult::new(msg));
        }

        let mut out = format!("Articles ({}):\n\n", page.len());
        for &idx in &page {
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
    } else {
        // Non-ranked path: simple pagination
        let has_more = articles.len() > limit;
        let articles = &articles[..articles.len().min(limit)];
        let display_order: Vec<usize> = (0..articles.len()).collect();

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

/// Return the next unreviewed article with Accept/Reject buttons attached.
/// This enforces one-article-at-a-time presentation — each call returns
/// exactly one article, and the buttons trigger the next cycle.
pub fn handle_next(db: &MemoryDB) -> Result<ToolResult> {
    let articles = db.get_rss_articles(Some("new"), None, 1, 0)?;

    let Some(article) = articles.first() else {
        return Ok(ToolResult::new("No more articles to review."));
    };

    let short_id: String = article.id.chars().take(8).collect();

    let feeds = db.list_rss_feeds().unwrap_or_default();
    let feed_name = feeds
        .iter()
        .find(|f| f.id == article.feed_id)
        .map_or("Unknown", |f| f.name.as_str());

    let snippet = article
        .description
        .as_deref()
        .unwrap_or("")
        .chars()
        .take(300)
        .collect::<String>();

    let remaining = db.count_rss_articles(Some("new"), None)?;

    let mut out = format!(
        "**{}**\nSource: {} | ID: {short_id}",
        article.title, feed_name
    );
    if let Some(ref author) = article.author {
        let _ = write!(out, " | Author: {author}");
    }
    if !snippet.is_empty() {
        let _ = write!(out, "\n\n{snippet}");
        if article.description.as_deref().unwrap_or("").chars().count() > 300 {
            out.push('…');
        }
    }
    let _ = write!(out, "\n\n({remaining} articles remaining)");

    let mut metadata = HashMap::new();
    metadata.insert(
        "suggested_buttons".to_string(),
        serde_json::json!([
            {"id": format!("rss-accept-{short_id}"), "label": "Accept", "style": "primary", "context": short_id},
            {"id": format!("rss-reject-{short_id}"), "label": "Reject", "style": "danger", "context": short_id},
        ]),
    );
    Ok(ToolResult::new(out).with_metadata(metadata))
}

/// Rank articles using the persisted `LinTS` model in read-only mode.
/// Does NOT register new features or save the model — unknown features
/// contribute zero to scoring, which is correct (the model hasn't learned
/// about them yet). This avoids the mutation-without-save problem and
/// keeps covariance/pruning semantics consistent with the scan path.
fn rank_articles(
    db: &MemoryDB,
    articles: &[crate::agent::memory::memory_db::rss::RssArticle],
) -> Vec<usize> {
    let model = load_or_create_model(db);

    // If the model has no features yet, return natural order
    if model.dimension() == 0 {
        return (0..articles.len()).collect();
    }

    // Extract profile keywords so browse ranking uses the same features as scan/feedback
    let keywords = super::scanner::extract_profile_keywords(db);

    // Batch-fetch all tags in one query instead of N+1 per-article queries
    let article_ids: Vec<&str> = articles.iter().map(|a| a.id.as_str()).collect();
    let tags_map = db
        .get_rss_article_tags_batch(&article_ids)
        .unwrap_or_default();

    // Read-only encoding: build_feature_vector uses only features already in the model.
    // No ensure_feature calls → model dimension stays constant → all vectors are uniform.
    let feature_vecs: Vec<_> = articles
        .iter()
        .map(|a| {
            let empty = Vec::new();
            let tags = tags_map.get(&a.id).unwrap_or(&empty);
            let keyword_overlap: Vec<String> = keywords
                .iter()
                .filter(|kw| {
                    let kw_lower = kw.to_lowercase();
                    a.title.to_lowercase().contains(&kw_lower)
                        || a.description
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&kw_lower)
                })
                .cloned()
                .collect();
            model.build_feature_vector(&a.feed_id, tags, &keyword_overlap)
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
    if let Err(e) = db.save_rss_model(&feature_index, &mu, &sigma, super::now_ms()) {
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

    // Extract profile keywords so feedback encodes the same features as scanning
    let keywords = super::scanner::extract_profile_keywords(db);
    let keyword_overlap: Vec<String> = keywords
        .iter()
        .filter(|kw| {
            let kw_lower = kw.to_lowercase();
            article.title.to_lowercase().contains(&kw_lower)
                || article
                    .description
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&kw_lower)
        })
        .cloned()
        .collect();

    let mut model = load_or_create_model(db);
    let x = model.encode_article(&article.feed_id, &tags, &keyword_overlap);
    model.update(&x, accepted);
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
