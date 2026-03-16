use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::memory::memory_db::rss::{
    STATE_COMPLETE, STATE_NEEDS_CALIBRATION, STATE_NEEDS_FEEDS, STATE_NEEDS_PROFILE,
};
use crate::agent::tools::ToolResult;

type FeedList = &'static [(&'static str, &'static str)];

const RUST_FEEDS: FeedList = &[
    ("This Week in Rust", "https://this-week-in-rust.org/rss.xml"),
    ("Rust Blog", "https://blog.rust-lang.org/feed.xml"),
];
const AI_FEEDS: FeedList = &[
    ("arXiv CS.AI", "http://arxiv.org/rss/cs.AI"),
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

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

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
        "add_feed" | "remove_feed" | "list_feeds" => matches!(
            state,
            STATE_NEEDS_FEEDS | STATE_NEEDS_CALIBRATION | STATE_COMPLETE
        ),
        "get_articles" | "accept" | "reject" | "get_article_detail" => {
            matches!(state, STATE_NEEDS_CALIBRATION | STATE_COMPLETE)
        }
        "scan" | "feed_stats" => state == STATE_COMPLETE,
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
pub fn handle_onboard(db: &MemoryDB) -> Result<ToolResult> {
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
                Ok(ToolResult::new(format!(
                    "Calibration complete! You've reviewed {review_count} articles.\n\n\
                     Your personalised scanner is being set up. \
                     Use get_articles to browse your feed, or scan to fetch new articles."
                )))
            } else {
                let remaining = 5 - review_count;
                let articles = db.get_rss_articles(Some("pending"), None, 5, 0)?;
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
                        let _ = writeln!(out, "- [{}] {} — {}", a.id, a.title, a.url);
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
    if interests.len() < 20 {
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

    if lower.contains("rust") {
        sections.push(("Rust", RUST_FEEDS));
    }
    if lower.contains("ai")
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
        STATE_COMPLETE => "scan",
        _ => "onboard",
    }
}
