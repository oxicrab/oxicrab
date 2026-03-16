use std::fmt::Write as _;

use anyhow::Result;

use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::tools::ToolResult;

pub fn handle_feed_stats(db: &MemoryDB) -> Result<ToolResult> {
    let mut out = String::new();

    // ── Overall totals ──────────────────────────────────────────────────────
    let all_articles = db.get_rss_articles(None, None, usize::MAX, 0)?;
    let total_scanned = all_articles.len();
    let total_accepted = all_articles
        .iter()
        .filter(|a| a.status == "accepted")
        .count();
    let total_rejected = all_articles
        .iter()
        .filter(|a| a.status == "rejected")
        .count();
    let overall_rate = acceptance_rate(total_accepted, total_accepted + total_rejected);

    let _ = writeln!(
        out,
        "RSS Feed Statistics\n\
         ══════════════════\n\
         Total scanned: {total_scanned} | Accepted: {total_accepted} | Rejected: {total_rejected} | \
         Acceptance rate: {overall_rate:.0}%"
    );

    // ── Per-feed stats ──────────────────────────────────────────────────────
    let feeds = db.list_rss_feeds()?;
    let enabled_count = feeds.iter().filter(|f| f.enabled).count();
    let disabled_count = feeds.len() - enabled_count;

    let _ = writeln!(
        out,
        "\nFeeds: {} total ({enabled_count} enabled, {disabled_count} disabled)",
        feeds.len()
    );

    if !feeds.is_empty() {
        out.push('\n');
        for feed in &feeds {
            let feed_articles = db.get_rss_articles(None, Some(&feed.id), usize::MAX, 0)?;
            let feed_total = feed_articles.len();
            let feed_accepted = feed_articles
                .iter()
                .filter(|a| a.status == "accepted")
                .count();
            let feed_rejected = feed_articles
                .iter()
                .filter(|a| a.status == "rejected")
                .count();
            let feed_rate = acceptance_rate(feed_accepted, feed_accepted + feed_rejected);

            let short_id: String = feed.id.chars().take(8).collect();
            let status_label = if feed.enabled { "enabled" } else { "disabled" };
            let last_fetched = feed
                .last_fetched_at_ms
                .map_or_else(|| "never".to_string(), format_date_ms);

            let _ = writeln!(
                out,
                "• {} [{}] ({status_label})\n  \
                 Articles: {feed_total} | Accepted: {feed_accepted} | Rejected: {feed_rejected} | \
                 Rate: {feed_rate:.0}% | Last fetched: {last_fetched}",
                feed.name, short_id,
            );
        }
    }

    // ── Model info ──────────────────────────────────────────────────────────
    out.push('\n');
    match db.load_rss_model() {
        Ok(Some((feature_index_json, mu_bytes, sigma_bytes))) => {
            match super::model::LinTSModel::from_bytes(&feature_index_json, &mu_bytes, &sigma_bytes)
            {
                Ok(model) => {
                    let dim = model.dimension();
                    let _ = writeln!(out, "Model: {dim} features");
                    if dim > 0 {
                        // Collect top-5 features by absolute weight
                        let mut features: Vec<(&String, f64)> = model
                            .feature_index
                            .iter()
                            .filter_map(|(name, &idx)| {
                                if idx < model.mu.len() {
                                    Some((name, model.mu[idx]))
                                } else {
                                    None
                                }
                            })
                            .collect();
                        features.sort_by(|a, b| {
                            b.1.abs()
                                .partial_cmp(&a.1.abs())
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                        let top: Vec<_> = features.iter().take(5).collect();
                        if !top.is_empty() {
                            out.push_str("Top features by weight:\n");
                            for (name, weight) in top {
                                let dir = if *weight >= 0.0 { "+" } else { "" };
                                let _ = writeln!(out, "  {dir}{weight:.3}  {name}");
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = writeln!(out, "Model: present but could not be loaded ({e})");
                }
            }
        }
        Ok(None) => {
            out.push_str("Model: not yet trained (accept/reject articles to build it)\n");
        }
        Err(e) => {
            let _ = writeln!(out, "Model: error loading — {e}");
        }
    }

    // ── Cron job status ─────────────────────────────────────────────────────
    out.push('\n');
    let profile = db.get_rss_profile()?;
    let cron_job_id = profile.as_ref().and_then(|p| p.cron_job_id.as_deref());

    if let Some(job_id) = cron_job_id {
        match db.get_cron_job(job_id) {
            Ok(Some(job)) => {
                let status = if job.enabled { "active" } else { "disabled" };
                let next = job
                    .state
                    .next_run_at_ms
                    .map_or_else(|| "unknown".to_string(), format_date_ms);
                let _ = writeln!(
                    out,
                    "Scheduled scanning: {status} (job id: {job_id}, \
                     schedule: {}, next run: {next})",
                    job.schedule.describe()
                );
            }
            Ok(None) => {
                out.push_str(
                    "Warning: scheduled scanning job not found — \
                     re-run onboard to recreate it\n",
                );
            }
            Err(e) => {
                let _ = writeln!(out, "Warning: could not check cron job status: {e}");
            }
        }
    } else {
        out.push_str(
            "Warning: scheduled scanning is not active — \
             run onboard to complete setup\n",
        );
    }

    Ok(ToolResult::new(out.trim_end().to_string()))
}

fn acceptance_rate(accepted: usize, reviewed: usize) -> f64 {
    if reviewed == 0 {
        0.0
    } else {
        (accepted as f64 / reviewed as f64) * 100.0
    }
}

/// Format a millisecond timestamp as a human-readable date (UTC).
fn format_date_ms(ms: i64) -> String {
    let days = ms / 1000 / 86400;
    let jdn = days + 2_440_588;
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
