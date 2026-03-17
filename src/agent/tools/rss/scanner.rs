use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use reqwest::Client;
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};

use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::memory::memory_db::rss::RssArticle;
use crate::agent::tools::ToolResult;
use crate::agent::tools::rss::model::LinTSModel;
use crate::config::RssConfig;

use super::now_ms;

/// Result of fetching and parsing a single feed.
struct FeedResult {
    feed_id: String,
    feed_name: String,
    fetched: usize,
}

struct ParsedEntry {
    url: String,
    title: String,
    author: Option<String>,
    description: Option<String>,
    published_at_ms: Option<i64>,
    tags: Vec<String>,
}

pub async fn handle_scan(db: &MemoryDB, client: &Client, config: &RssConfig) -> Result<ToolResult> {
    // ── Phase 1: Fetch all enabled feeds concurrently ─────────────────────

    let feeds = db.list_rss_feeds()?;
    let enabled_feeds: Vec<_> = feeds.into_iter().filter(|f| f.enabled).collect();

    if enabled_feeds.is_empty() {
        return Ok(ToolResult::error(
            "no enabled feeds. Use 'add_feed' to add one.",
        ));
    }

    info!("rss scan: fetching {} feed(s)", enabled_feeds.len());

    // Bound concurrency to 8
    let semaphore = Arc::new(Semaphore::new(8));
    let timeout_secs = config.scan_timeout;

    let mut handles = Vec::new();
    for feed in enabled_feeds {
        let client = client.clone();
        let sem = semaphore.clone();
        let max_per_feed = config.max_articles_per_feed;

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("rss scan semaphore closed");
            fetch_feed(
                &client,
                &feed.id,
                &feed.url,
                &feed.name,
                timeout_secs,
                max_per_feed,
            )
            .await
        }));
    }

    let join_results = futures_util::future::join_all(handles).await;

    // ── Phase 2: Pre-filter and insert new articles ───────────────────────

    let now = now_ms();
    // Use purge_days as the ingest lookback so fetch and purge windows are consistent.
    // Articles older than purge_days will be cleaned up by the hygiene phase anyway.
    let lookback_ms: i64 =
        i64::try_from(u128::from(config.purge_days) * 24 * 60 * 60 * 1000).unwrap_or(0);
    let cutoff_ms = now.saturating_sub(lookback_ms);

    let mut feed_results: Vec<FeedResult> = Vec::new();
    let mut all_new_articles: Vec<(String, RssArticle, Vec<String>)> = Vec::new(); // (feed_id, article, tags)

    for join_result in join_results {
        let result = match join_result {
            Ok(r) => r,
            Err(e) => {
                warn!("rss scan: task panicked: {e}");
                continue;
            }
        };

        let (feed_id, feed_name, fetch_result) = result;

        match fetch_result {
            Err(e) => {
                warn!("rss scan: feed '{feed_id}' fetch failed: {e}");
                if let Err(db_err) = db.increment_rss_feed_failures(&feed_id, &e.to_string()) {
                    warn!("rss scan: failed to record failure for '{feed_id}': {db_err}");
                }
            }
            Ok(entries) => {
                // Reset failure state on success
                if let Err(e) = db.update_rss_feed_fetch_state(&feed_id, now) {
                    warn!("rss scan: failed to update fetch state for '{feed_id}': {e}");
                }

                let fetched = entries.len();
                let mut passing = 0usize;

                for entry in entries {
                    // Skip entries with empty titles
                    if entry.title.trim().is_empty() {
                        continue;
                    }

                    // Skip entries older than purge_days (when published date is known)
                    if let Some(pub_ms) = entry.published_at_ms
                        && pub_ms < cutoff_ms
                    {
                        debug!(
                            "rss scan: skipping old article '{}' (pub_ms={pub_ms})",
                            entry.title
                        );
                        continue;
                    }

                    // Cap at max_articles_per_feed per feed
                    if passing >= config.max_articles_per_feed {
                        break;
                    }

                    let article_id = uuid::Uuid::new_v4().to_string();
                    let article = RssArticle {
                        id: article_id.clone(),
                        feed_id: feed_id.clone(),
                        url: entry.url.clone(),
                        title: entry.title.clone(),
                        author: entry.author.clone(),
                        published_at_ms: entry.published_at_ms,
                        fetched_at_ms: now,
                        description: entry.description.clone(),
                        full_content: None,
                        summary: None,
                        status: "new".to_string(),
                        read: false,
                        created_at_ms: now,
                    };

                    match db.insert_rss_article(&article) {
                        Ok(()) => {
                            // Insert tags
                            let tag_refs: Vec<&str> =
                                entry.tags.iter().map(String::as_str).collect();
                            if !tag_refs.is_empty()
                                && let Err(e) = db.insert_rss_article_tags(&article_id, &tag_refs)
                            {
                                warn!("rss scan: failed to insert tags for {article_id}: {e}");
                            }
                            all_new_articles.push((feed_id.clone(), article, entry.tags));
                            passing += 1;
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            if msg.contains("UNIQUE") || msg.contains("unique") {
                                // Dedup is global by article URL (not per-feed). When the same
                                // article appears in multiple feeds (e.g. HN and Lobsters both
                                // link the same blog post), only the first feed's insert wins.
                                // This is intentional — duplicate content shouldn't surface twice.
                                debug!("rss scan: skipping duplicate article '{}'", entry.url);
                            } else {
                                warn!("rss scan: insert failed for '{}': {e}", entry.url);
                            }
                        }
                    }
                }

                feed_results.push(FeedResult {
                    feed_id: feed_id.clone(),
                    feed_name,
                    fetched,
                });
            }
        }
    }

    // ── Phase 3: LinTS Ranking ─────────────────────────────────────────────

    // Load or create model
    let mut model = match db.load_rss_model() {
        Ok(Some((fi, mu, sigma))) => match LinTSModel::from_bytes(&fi, &mu, &sigma) {
            Ok(m) => m,
            Err(e) => {
                warn!("rss scan: model corrupted, creating fresh: {e}");
                LinTSModel::new()
            }
        },
        Ok(None) => LinTSModel::new(),
        Err(e) => {
            warn!("rss scan: failed to load model: {e}");
            LinTSModel::new()
        }
    };

    // Extract keywords from user profile
    let keywords = extract_profile_keywords(db);

    // Only inflate/encode/save if there are new articles to rank. Inflating on
    // empty scans would cause unbounded covariance growth (0.04/day at default
    // settings), making rankings near-random after ~250 quiet days.
    if !all_new_articles.is_empty() {
        model.inflate_covariance(config.covariance_inflation);
    }

    // Encode all new articles using a two-pass approach to avoid dimension
    // mismatch: pass 1 registers all features, pass 2 builds uniform-dimension
    // vectors. Without this, earlier vectors would be shorter than later ones,
    // causing a panic in sample_weights().dot(x).

    // Pass 1: Register all features and collect keyword overlaps
    let mut keyword_overlaps: Vec<Vec<String>> = Vec::new();
    for (feed_id, article, tags) in &all_new_articles {
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

        // Register features without building vectors
        let feed_key = format!("feed:{feed_id}");
        model.ensure_feature(&feed_key);
        for tag in tags {
            model.ensure_feature(&format!("tag:{tag}"));
        }
        for kw in &keyword_overlap {
            model.ensure_feature(&format!("kw:{kw}"));
        }

        keyword_overlaps.push(keyword_overlap);
    }

    // Pass 2: Build all vectors at the final (uniform) dimension
    let mut encoded: Vec<(usize, nalgebra::DVector<f64>)> = Vec::new();
    for (idx, (feed_id, _article, tags)) in all_new_articles.iter().enumerate() {
        let x = model.build_feature_vector(feed_id, tags, &keyword_overlaps[idx]);
        encoded.push((idx, x));
    }

    // Sample weights for Thompson Sampling ranking — single draw, reused for display scores
    let mut all_scores: HashMap<usize, f64> = HashMap::new();
    let ranked_indices: Vec<usize> = if encoded.is_empty() {
        vec![]
    } else {
        let feature_vecs: Vec<nalgebra::DVector<f64>> =
            encoded.iter().map(|(_, x)| x.clone()).collect();

        match model.sample_weights() {
            Ok(w) => {
                let mut scored: Vec<(usize, f64)> = feature_vecs
                    .iter()
                    .enumerate()
                    .map(|(i, x)| {
                        let score = LinTSModel::score(&w, x);
                        let orig_idx = encoded[i].0;
                        all_scores.insert(orig_idx, score);
                        (orig_idx, score)
                    })
                    .collect();
                scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                scored.into_iter().map(|(orig_idx, _)| orig_idx).collect()
            }
            Err(e) => {
                warn!("rss scan: thompson sampling failed, using natural order: {e}");
                (0..all_new_articles.len()).collect()
            }
        }
    };

    // Take top candidates_per_scan
    let top_indices: Vec<usize> = ranked_indices
        .into_iter()
        .take(config.candidates_per_scan)
        .collect();

    // Save model only if we encoded new articles (inflation or feature expansion occurred)
    if !all_new_articles.is_empty() {
        model.prune_if_needed();
        let (fi_json, mu_bytes, sigma_bytes) = model.to_bytes();
        if let Err(e) = db.save_rss_model(&fi_json, &mu_bytes, &sigma_bytes, now) {
            warn!("rss scan: failed to save model: {e}");
        }
    }

    // ── Phase 4: Format output ─────────────────────────────────────────────

    let profile = db.get_rss_profile().unwrap_or(None);
    let interests_summary = profile
        .as_ref()
        .map_or_else(|| "(no profile set)".to_string(), |p| p.interests.clone());

    let mut out = String::new();

    // Interest profile summary
    let _ = writeln!(
        out,
        "**RSS Scan Results**\n\nInterest profile: {interests_summary}\n"
    );

    if all_new_articles.is_empty() {
        let _ = writeln!(out, "No new articles found across all feeds.");
    } else {
        let shown = top_indices.len();
        let total_new = all_new_articles.len();
        let _ = writeln!(
            out,
            "Showing top {shown} of {total_new} new article(s), ranked by relevance:\n"
        );

        let _ = writeln!(
            out,
            "{} new articles ranked by relevance.",
            top_indices.len()
        );
    }

    // Per-source summary
    if !feed_results.is_empty() {
        let _ = writeln!(out, "\n**Per-feed summary:**");
        let feed_id_to_new: HashMap<&str, usize> = {
            let mut m: HashMap<&str, usize> = HashMap::new();
            for (feed_id, _, _) in &all_new_articles {
                *m.entry(feed_id.as_str()).or_insert(0) += 1;
            }
            m
        };

        for fr in &feed_results {
            let new_count = feed_id_to_new
                .get(fr.feed_id.as_str())
                .copied()
                .unwrap_or(0);
            let _ = writeln!(
                out,
                "  • {}: {} fetched, {} new",
                fr.feed_name, fr.fetched, new_count
            );
        }
    }

    if !all_new_articles.is_empty() {
        out.push_str(
            "\nCall rss { action: \"next\" } to review articles one at a time with Accept/Reject buttons.",
        );
    }

    // ── Phase 5: Hygiene ───────────────────────────────────────────────────

    match db.purge_stale_rss_articles(config.purge_days) {
        Ok(n) if n > 0 => debug!("rss scan: purged {n} stale article(s)"),
        Ok(_) => {}
        Err(e) => warn!("rss scan: purge failed: {e}"),
    }

    Ok(ToolResult::new(out.trim_end().to_string()))
}

/// Fetch a single feed, returning `(feed_id, feed_name, Result<Vec<ParsedEntry>>)`.
async fn fetch_feed(
    client: &Client,
    feed_id: &str,
    url: &str,
    name: &str,
    timeout_secs: u64,
    max_per_feed: usize,
) -> (String, String, Result<Vec<ParsedEntry>>) {
    let result = do_fetch_feed(client, url, timeout_secs, max_per_feed).await;
    (feed_id.to_string(), name.to_string(), result)
}

async fn do_fetch_feed(
    _client: &Client,
    url: &str,
    timeout_secs: u64,
    max_per_feed: usize,
) -> Result<Vec<ParsedEntry>> {
    // SSRF validation — also returns pinned addresses to prevent DNS rebinding
    let resolved = crate::utils::url_security::validate_and_resolve(url)
        .await
        .map_err(|e| anyhow::anyhow!("SSRF blocked: {e}"))?;

    // Build a per-request client pinned to the validated addresses
    let pinned = {
        let mut builder = Client::builder()
            .user_agent(format!("oxicrab/{}", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(timeout_secs))
            .redirect(reqwest::redirect::Policy::none());
        // Prefer IPv4 — many hosts advertise AAAA records but have broken IPv6
        // connectivity, causing reqwest to hang until timeout.
        let has_ipv4 = resolved.addrs.iter().any(std::net::SocketAddr::is_ipv4);
        for addr in &resolved.addrs {
            if has_ipv4 && addr.is_ipv6() {
                continue;
            }
            builder = builder.resolve(&resolved.host, *addr);
        }
        builder
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build pinned client: {e}"))?
    };

    // Fetch
    let resp = pinned
        .get(url)
        .timeout(Duration::from_secs(timeout_secs))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("fetch failed: {e}"))?;

    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status().as_u16());
    }

    // Read body with size limit (10 MB) to prevent OOM from malicious feeds
    let (body, _truncated) = crate::utils::http::limited_body(resp, 10 * 1024 * 1024)
        .await
        .map_err(|e| anyhow::anyhow!("body read failed: {e}"))?;

    // Parse
    let feed = feed_rs::parser::parse(body.as_slice())
        .map_err(|e| anyhow::anyhow!("parse failed: {e}"))?;

    let entries: Vec<ParsedEntry> = feed
        .entries
        .into_iter()
        .take(max_per_feed * 2) // fetch a bit more to allow filtering
        .filter_map(|entry| {
            // Extract URL: prefer links, fall back to entry.id
            let url = entry
                .links
                .first()
                .map(|l| l.href.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| entry.id.clone());

            // Skip entries with no useful URL
            if url.is_empty() {
                return None;
            }

            let title = entry.title.map(|t| t.content).unwrap_or_default();

            let description = entry
                .summary
                .map(|s| s.content)
                .or_else(|| entry.content.into_iter().find_map(|c| c.body));

            let author = entry.authors.into_iter().next().map(|a| a.name);

            let published_at_ms = entry.published.map(|dt| dt.timestamp_millis());

            let tags: Vec<String> = entry
                .categories
                .into_iter()
                .map(|c| c.term)
                .filter(|t| !t.is_empty())
                .collect();

            Some(ParsedEntry {
                url,
                title,
                author,
                description,
                published_at_ms,
                tags,
            })
        })
        .collect();

    Ok(entries)
}

/// Extract simple keywords from the user's interest profile for feature matching.
pub(super) fn extract_profile_keywords(db: &MemoryDB) -> Vec<String> {
    let Ok(Some(profile)) = db.get_rss_profile() else {
        return vec![];
    };

    // Split on common delimiters, filter noise words, take up to 20 keywords
    let stop_words = [
        "i",
        "am",
        "a",
        "an",
        "the",
        "and",
        "or",
        "but",
        "in",
        "on",
        "at",
        "to",
        "for",
        "of",
        "with",
        "my",
        "is",
        "are",
        "was",
        "be",
        "been",
        "have",
        "has",
        "had",
        "im",
        "interested",
        "interest",
        "topics",
        "topic",
        "things",
        "thing",
    ];

    profile
        .interests
        .split([',', ';', '\n', '/', '|'])
        .flat_map(|chunk| chunk.split_whitespace())
        .map(|w| {
            w.trim_matches(|c: char| c.is_ascii_punctuation())
                .to_lowercase()
        })
        .filter(|w| w.len() >= 3 && !stop_words.contains(&w.as_str()))
        .take(20)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::memory::memory_db::MemoryDB;
    use crate::agent::memory::memory_db::rss::{RssFeed, STATE_COMPLETE};
    use crate::config::RssConfig;

    fn test_db_with_profile() -> MemoryDB {
        let db = MemoryDB::new(":memory:").unwrap();
        let now = now_ms();
        db.set_rss_profile(
            "Rust programming, AI, distributed systems",
            STATE_COMPLETE,
            now,
        )
        .unwrap();
        db
    }

    #[tokio::test]
    async fn test_scan_no_feeds() {
        let db = test_db_with_profile();
        let client = Client::new();
        let config = RssConfig::default();

        let result = handle_scan(&db, &client, &config).await.unwrap();
        assert!(result.is_error, "expected error when no enabled feeds");
        assert!(
            result.content.contains("no enabled feeds"),
            "expected 'no enabled feeds' message, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_scan_no_enabled_feeds() {
        let db = test_db_with_profile();
        // Insert a disabled feed
        db.insert_rss_feed(&RssFeed {
            id: "f1".into(),
            url: "https://example.com/feed.xml".into(),
            name: "Test".into(),
            site_url: None,
            last_fetched_at_ms: None,
            last_error: None,
            consecutive_failures: 0,
            enabled: false, // disabled!
            created_at_ms: now_ms(),
        })
        .unwrap();

        let client = Client::new();
        let config = RssConfig::default();

        let result = handle_scan(&db, &client, &config).await.unwrap();
        assert!(result.is_error, "expected error when no enabled feeds");
    }

    #[test]
    fn test_extract_profile_keywords_basic() {
        let db = MemoryDB::new(":memory:").unwrap();
        db.set_rss_profile(
            "Rust programming, AI research, distributed systems",
            "complete",
            now_ms(),
        )
        .unwrap();

        let kws = extract_profile_keywords(&db);
        // Should include at least "rust", "programming", "research", "distributed", "systems"
        let lower: Vec<String> = kws.iter().map(|k| k.to_lowercase()).collect();
        assert!(
            lower.contains(&"rust".to_string()),
            "expected 'rust' keyword, got: {lower:?}"
        );
        assert!(
            lower.contains(&"programming".to_string()),
            "expected 'programming' keyword, got: {lower:?}"
        );
    }

    #[test]
    fn test_extract_profile_keywords_no_profile() {
        let db = MemoryDB::new(":memory:").unwrap();
        let kws = extract_profile_keywords(&db);
        assert!(kws.is_empty());
    }

    #[test]
    fn test_extract_profile_keywords_stop_words_filtered() {
        let db = MemoryDB::new(":memory:").unwrap();
        db.set_rss_profile("I am interested in the and or a an", "complete", now_ms())
            .unwrap();
        let kws = extract_profile_keywords(&db);
        // All stop words — should be empty (or very short)
        for kw in &kws {
            assert!(
                kw.len() >= 3,
                "short keyword '{kw}' should have been filtered"
            );
        }
    }
}
