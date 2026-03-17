use std::fmt::Write as _;
use std::sync::Arc;

use anyhow::Result;
use tracing::warn;

use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::memory::memory_db::rss::{
    STATE_COMPLETE, STATE_NEEDS_CALIBRATION, STATE_NEEDS_FEEDS, STATE_NEEDS_PROFILE,
};
use crate::agent::tools::ToolResult;
use crate::agent::tools::base::ExecutionContext;
use crate::cron::service::CronService;
use crate::cron::types::{CronJob, CronJobState, CronPayload, CronSchedule, CronTarget};

type FeedList = &'static [(&'static str, &'static str)];

const RUST_FEEDS: FeedList = &[
    ("This Week in Rust", "https://this-week-in-rust.org/rss.xml"),
    ("Rust Blog", "https://blog.rust-lang.org/feed.xml"),
];
const AI_FEEDS: FeedList = &[
    ("arXiv CS.AI", "https://arxiv.org/rss/cs.AI"),
    ("The Gradient", "https://thegradient.pub/rss/"),
];
const WEB_FEEDS: FeedList = &[
    ("CSS Tricks", "https://css-tricks.com/feed/"),
    (
        "Smashing Magazine",
        "https://www.smashingmagazine.com/feed/",
    ),
];
const SECURITY_FEEDS: FeedList = &[
    ("Krebs on Security", "https://krebsonsecurity.com/feed/"),
    ("Schneier on Security", "https://www.schneier.com/feed/"),
];
const DEVOPS_FEEDS: FeedList = &[
    (
        "DevOps Weekly Archive",
        "https://devopsweeklyarchive.com/feed/",
    ),
    ("The New Stack", "https://thenewstack.io/feed/"),
];
const GENERAL_FEEDS: FeedList = &[
    ("Hacker News", "https://hnrss.org/frontpage"),
    ("Lobsters", "https://lobste.rs/rss"),
];

use super::now_ms;

/// Check whether the given action is permitted in the current onboarding state.
/// Returns `Ok(Some(ToolResult))` with an error response if the action is gated,
/// or `Ok(None)` if the action is allowed.
pub fn check_gate(db: &MemoryDB, action: &str) -> Result<Option<ToolResult>> {
    // onboard and set_profile are always allowed
    if matches!(action, "onboard" | "set_profile") {
        return Ok(None);
    }

    let profile = db.get_rss_profile()?;
    let state = profile
        .as_ref()
        .map_or(STATE_NEEDS_PROFILE, |p| p.onboarding_state.as_str());

    let allowed = match action {
        "add_feed" | "remove_feed" | "enable_feed" | "list_feeds" => matches!(
            state,
            STATE_NEEDS_FEEDS | STATE_NEEDS_CALIBRATION | STATE_COMPLETE
        ),
        "get_articles" | "accept" | "reject" | "get_article_detail" | "review" | "next"
        | "done" => {
            matches!(state, STATE_NEEDS_CALIBRATION | STATE_COMPLETE)
        }
        "scan" => matches!(state, STATE_NEEDS_CALIBRATION | STATE_COMPLETE),
        "feed_stats" => state == STATE_COMPLETE,
        _ => true, // unknown actions fall through to execute()'s own error
    };

    if allowed {
        return Ok(None);
    }

    let progress = progress_description(state);
    let next_action = next_action_hint(state);

    let msg = serde_json::json!({
        "error": "action not available in current onboarding state",
        "onboarding_state": state,
        "progress": progress,
        "next_action": next_action
    });

    Ok(Some(ToolResult::error(msg.to_string())))
}

/// Return the appropriate onboarding response for the current state.
pub fn handle_onboard(
    db: &MemoryDB,
    ctx: &ExecutionContext,
    cron_service: Option<&Arc<CronService>>,
) -> Result<ToolResult> {
    let profile = db.get_rss_profile()?;
    let state = profile
        .as_ref()
        .map_or(STATE_NEEDS_PROFILE, |p| p.onboarding_state.as_str());

    match state {
        STATE_NEEDS_PROFILE => Ok(ToolResult::new(
            "Welcome to your personalised RSS reader!\n\n\
             To get started, please describe your interests so I can suggest relevant feeds. \
             For example: \"I'm interested in Rust programming, AI research, and cloud infrastructure.\"\n\n\
             Use the set_profile action with your interests to continue.",
        )),

        STATE_NEEDS_FEEDS => {
            let interests = profile.as_ref().map_or("", |p| p.interests.as_str());
            let suggestions = suggest_feeds(interests);
            Ok(ToolResult::new(format!(
                "Great! Your interests are recorded: \"{interests}\"\n\n\
                 Now let's add some feeds. Here are some suggestions based on your interests:\n\n\
                 {suggestions}\n\
                 Use the add_feed action with a URL (and optional name) to add feeds. \
                 Add at least one feed to continue."
            )))
        }

        STATE_NEEDS_CALIBRATION => {
            let review_count = db.count_rss_reviews()?;
            if review_count >= 5 {
                // Check if we're running inside a cron job — can't create cron from cron
                let in_cron = ctx
                    .metadata
                    .get(crate::bus::meta::IS_CRON_JOB)
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);

                if in_cron {
                    return Ok(ToolResult::new(
                        "Calibration is complete, but a scheduled scan job cannot be created \
                         from within a cron execution. Please run onboard again via direct chat \
                         to finish setup.",
                    ));
                }

                let now = now_ms();

                // Only create a cron job if one doesn't already exist
                let existing_cron_id = db.get_rss_profile()?.and_then(|p| p.cron_job_id);

                if existing_cron_id.is_none() {
                    // Create a recurring cron job to scan every 6 hours
                    let job_id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
                    let job = CronJob {
                        id: job_id.clone(),
                        name: "rss-scan".to_string(),
                        enabled: true,
                        schedule: CronSchedule::Cron {
                            expr: Some("0 */6 * * *".to_string()),
                            tz: Some("UTC".to_string()),
                        },
                        payload: CronPayload {
                            kind: "agent_turn".to_string(),
                            message: "Scan RSS feeds using the rss tool scan action, then call \
                                      rss { action: \"review\" } to present the first article for review. \
                                      After the user accepts or rejects, call review again for the next article. \
                                      Keep going until the user says done or there are no more articles."
                                .to_string(),
                            agent_echo: true,
                            targets: vec![CronTarget {
                                channel: ctx.channel.clone(),
                                to: ctx.chat_id.clone(),
                            }],
                        },
                        state: CronJobState::default(),
                        created_at_ms: now,
                        updated_at_ms: now,
                        delete_after_run: false,
                        expires_at_ms: None,
                        max_runs: None,
                        cooldown_secs: None,
                        max_concurrent: None,
                    };

                    if let Some(svc) = cron_service {
                        // Re-check inside the critical window to prevent TOCTOU
                        // race from concurrent onboard calls creating duplicates
                        let still_none =
                            db.get_rss_profile()?.and_then(|p| p.cron_job_id).is_none();
                        if !still_none {
                            // Another call already created the job
                            db.set_rss_onboarding_state(STATE_COMPLETE, now)?;
                            let feed_count = db.count_rss_feeds()?;
                            return Ok(ToolResult::new(format!(
                                "Setup complete! {feed_count} feeds configured, \
                                 {review_count} articles reviewed. Scanning is already scheduled."
                            )));
                        }
                        svc.add_job(job)?;
                        if let Err(e) = db.set_rss_cron_job_id(&job_id, now) {
                            // Roll back the cron job to avoid orphaned duplicates
                            let _ = svc.remove_job(&job_id);
                            return Err(e);
                        }
                    } else {
                        warn!(
                            "rss onboard: cron service unavailable, staying in needs_calibration"
                        );
                        return Ok(ToolResult::new(
                            "Calibration complete, but a scheduled scan job could not be created \
                             because the cron service is unavailable. Please run onboard again to \
                             finish setup once the service is available.",
                        ));
                    }
                }

                db.set_rss_onboarding_state(STATE_COMPLETE, now)?;

                let feed_count = db.count_rss_feeds()?;
                Ok(ToolResult::new(format!(
                    "Calibration complete! You've reviewed {review_count} articles.\n\n\
                     Your personalised RSS reader is fully set up.\n\
                     Feeds: {feed_count}\n\
                     Scheduled scanning: every 6 hours (cron: 0 */6 * * *, UTC)\n\n\
                     Available actions: scan (fetch now), get_articles (browse), \
                     feed_stats (metrics), add_feed / remove_feed (manage feeds)."
                )))
            } else {
                let remaining = 5 - review_count;
                let articles = db.get_rss_articles(Some("new"), None, 5, 0)?;
                if articles.is_empty() {
                    Ok(ToolResult::new(format!(
                        "Calibration in progress ({review_count}/5 reviews done).\n\n\
                         No pending articles yet — use scan to fetch articles from your feeds, \
                         then come back to review them."
                    )))
                } else {
                    let mut out = format!(
                        "Calibration in progress ({review_count}/5 reviews done, \
                         {remaining} more needed).\n\n\
                         Here are articles to review:\n\n"
                    );
                    for a in &articles {
                        let short_id: String = a.id.chars().take(8).collect();
                        let _ = writeln!(out, "- [{}] {} — {}", short_id, a.title, a.url);
                    }
                    out.push_str(
                        "\nUse accept or reject with article IDs to calibrate your preferences.",
                    );
                    Ok(ToolResult::new(out))
                }
            }
        }

        // STATE_COMPLETE and any unexpected states
        _ => {
            let feed_count = db.count_rss_feeds()?;
            let review_count = db.count_rss_reviews()?;
            Ok(ToolResult::new(format!(
                "Your RSS reader is fully set up.\n\n\
                 Feeds: {feed_count}\n\
                 Articles reviewed: {review_count}\n\n\
                 Available actions: scan (fetch new articles), get_articles (browse), \
                 feed_stats (per-feed metrics), add_feed / remove_feed (manage feeds)."
            )))
        }
    }
}

/// Set or update the user profile interests and advance the onboarding state if needed.
pub fn handle_set_profile(db: &MemoryDB, interests: &str) -> Result<ToolResult> {
    if interests.chars().count() < 20 {
        return Ok(ToolResult::error(
            "interests must be at least 20 characters — please describe your interests in more detail",
        ));
    }

    let profile = db.get_rss_profile()?;
    let now = now_ms();

    match profile {
        None => {
            // First time — create profile and transition to needs_feeds
            db.set_rss_profile(interests, STATE_NEEDS_FEEDS, now)?;
            let suggestions = suggest_feeds(interests);
            Ok(ToolResult::new(format!(
                "Profile saved. Here are some feed suggestions based on your interests:\n\n\
                 {suggestions}\n\
                 Use add_feed to subscribe to feeds."
            )))
        }
        Some(ref p) if p.onboarding_state == STATE_NEEDS_PROFILE => {
            // Still in initial state — update profile and advance
            db.set_rss_profile(interests, STATE_NEEDS_FEEDS, now)?;
            let suggestions = suggest_feeds(interests);
            Ok(ToolResult::new(format!(
                "Profile updated. Here are some feed suggestions based on your interests:\n\n\
                 {suggestions}\n\
                 Use add_feed to subscribe to feeds."
            )))
        }
        Some(ref p) => {
            // Already past profile step — just update interests, keep current state
            db.set_rss_profile(interests, &p.onboarding_state.clone(), now)?;
            Ok(ToolResult::new("interests updated successfully"))
        }
    }
}

/// Return feed suggestions based on keyword matching against interests.
/// Always includes general tech feeds.
fn suggest_feeds(interests: &str) -> String {
    let lower = interests.to_lowercase();
    let mut sections: Vec<(&str, FeedList)> = Vec::new();

    if lower.contains(" rust ")
        || lower.starts_with("rust ")
        || lower.ends_with(" rust")
        || lower == "rust"
    {
        sections.push(("Rust", RUST_FEEDS));
    }
    if lower.contains(" ai ")
        || lower.starts_with("ai ")
        || lower.ends_with(" ai")
        || lower == "ai"
        || lower.contains("machine learning")
        || lower.contains(" ml ")
        || lower.ends_with(" ml")
        || lower.starts_with("ml ")
    {
        sections.push(("AI / Machine Learning", AI_FEEDS));
    }
    if lower.contains("web") || lower.contains("frontend") || lower.contains("javascript") {
        sections.push(("Web / Frontend", WEB_FEEDS));
    }
    if lower.contains("security") || lower.contains("infosec") {
        sections.push(("Security", SECURITY_FEEDS));
    }
    if lower.contains("devops") || lower.contains("infrastructure") || lower.contains("cloud") {
        sections.push(("DevOps / Infrastructure", DEVOPS_FEEDS));
    }

    // Always include general tech
    sections.push(("General Tech", GENERAL_FEEDS));

    let mut out = String::new();
    for (category, feeds) in &sections {
        let _ = writeln!(out, "**{category}**");
        for (name, url) in *feeds {
            let _ = writeln!(out, "  - {name}: {url}");
        }
        out.push('\n');
    }
    out
}

fn progress_description(state: &str) -> &'static str {
    match state {
        STATE_NEEDS_PROFILE => "step 1/4: set your interests with set_profile",
        STATE_NEEDS_FEEDS => "step 2/4: add feeds with add_feed",
        STATE_NEEDS_CALIBRATION => "step 3/4: review 5 articles to calibrate recommendations",
        STATE_COMPLETE => "step 4/4: complete",
        _ => "unknown state",
    }
}

fn next_action_hint(state: &str) -> &'static str {
    match state {
        STATE_NEEDS_PROFILE => "set_profile",
        STATE_NEEDS_FEEDS => "add_feed",
        STATE_NEEDS_CALIBRATION => "accept",
        STATE_COMPLETE => "scan",
        _ => "onboard",
    }
}
