use super::super::MemoryDB;
use super::*;

fn test_db() -> MemoryDB {
    MemoryDB::new(":memory:").unwrap()
}

fn make_feed(id: &str, url: &str) -> RssFeed {
    RssFeed {
        id: id.to_string(),
        url: url.to_string(),
        name: format!("Feed {id}"),
        site_url: None,
        last_fetched_at_ms: None,
        last_error: None,
        consecutive_failures: 0,
        enabled: true,
        created_at_ms: 1000,
    }
}

fn make_article(id: &str, feed_id: &str) -> RssArticle {
    RssArticle {
        id: id.to_string(),
        feed_id: feed_id.to_string(),
        url: format!("https://example.com/{id}"),
        title: format!("Article {id}"),
        author: None,
        published_at_ms: None,
        fetched_at_ms: 1000,
        description: None,
        full_content: None,
        summary: None,
        status: "new".to_string(),
        read: false,
        created_at_ms: 1000,
    }
}

#[test]
fn insert_and_list_feeds() {
    let db = test_db();
    let feed = make_feed("f1", "https://example.com/feed.xml");
    db.insert_rss_feed(&feed).unwrap();

    let feeds = db.list_rss_feeds().unwrap();
    assert_eq!(feeds.len(), 1);
    assert_eq!(feeds[0].id, "f1");
    assert_eq!(feeds[0].url, "https://example.com/feed.xml");
    assert!(feeds[0].enabled);
    assert_eq!(feeds[0].consecutive_failures, 0);
}

#[test]
fn duplicate_feed_url_rejected() {
    let db = test_db();
    let feed = make_feed("f1", "https://example.com/feed.xml");
    db.insert_rss_feed(&feed).unwrap();

    let feed2 = RssFeed {
        id: "f2".to_string(),
        url: "https://example.com/feed.xml".to_string(), // same URL
        ..make_feed("f2", "https://example.com/feed.xml")
    };
    let result = db.insert_rss_feed(&feed2);
    assert!(result.is_err(), "duplicate URL should be rejected");
}

#[test]
fn delete_feed_cascades_articles() {
    let db = test_db();
    let feed = make_feed("f1", "https://example.com/feed.xml");
    db.insert_rss_feed(&feed).unwrap();

    let article = make_article("a1", "f1");
    db.insert_rss_article(&article).unwrap();

    // Verify article exists before deletion
    let articles = db.get_rss_articles(None, None, 10, 0).unwrap();
    assert_eq!(articles.len(), 1);

    let deleted = db.delete_rss_feed("f1").unwrap();
    assert_eq!(deleted, 1);

    // Article should be gone due to ON DELETE CASCADE
    let articles = db.get_rss_articles(None, None, 10, 0).unwrap();
    assert!(articles.is_empty());
}

#[test]
fn profile_crud() {
    let db = test_db();

    // Initially no profile
    assert!(db.get_rss_profile().unwrap().is_none());

    // Insert profile
    db.set_rss_profile("rust, ai, databases", STATE_NEEDS_FEEDS, 1000)
        .unwrap();
    let profile = db.get_rss_profile().unwrap().unwrap();
    assert_eq!(profile.interests, "rust, ai, databases");
    assert_eq!(profile.onboarding_state, STATE_NEEDS_FEEDS);
    assert_eq!(profile.created_at_ms, 1000);
    assert_eq!(profile.updated_at_ms, 1000);
    assert!(profile.cron_job_id.is_none());

    // Update via upsert
    db.set_rss_profile(
        "rust, ai, databases, security",
        STATE_NEEDS_CALIBRATION,
        2000,
    )
    .unwrap();
    let profile = db.get_rss_profile().unwrap().unwrap();
    assert_eq!(profile.interests, "rust, ai, databases, security");
    assert_eq!(profile.onboarding_state, STATE_NEEDS_CALIBRATION);
    assert_eq!(profile.updated_at_ms, 2000);
    // created_at_ms should not change on update
    assert_eq!(profile.created_at_ms, 1000);

    // Update onboarding state
    db.set_rss_onboarding_state(STATE_COMPLETE, 3000).unwrap();
    let profile = db.get_rss_profile().unwrap().unwrap();
    assert_eq!(profile.onboarding_state, STATE_COMPLETE);
    assert_eq!(profile.updated_at_ms, 3000);

    // Set cron job ID
    db.set_rss_cron_job_id("cron-abc", 4000).unwrap();
    let profile = db.get_rss_profile().unwrap().unwrap();
    assert_eq!(profile.cron_job_id.as_deref(), Some("cron-abc"));
    assert_eq!(profile.updated_at_ms, 4000);
}

#[test]
fn update_article_status() {
    let db = test_db();
    let feed = make_feed("f1", "https://example.com/feed.xml");
    db.insert_rss_feed(&feed).unwrap();

    let article = make_article("a1", "f1");
    db.insert_rss_article(&article).unwrap();

    db.update_rss_article_status("a1", "accepted").unwrap();
    let got = db.get_rss_article("a1").unwrap().unwrap();
    assert_eq!(got.status, "accepted");
    assert!(!got.read);

    // update_rss_article_full_content should also set read=true
    db.update_rss_article_full_content("a1", "full body text")
        .unwrap();
    let got = db.get_rss_article("a1").unwrap().unwrap();
    assert_eq!(got.full_content.as_deref(), Some("full body text"));
    assert!(got.read);
}

#[test]
fn article_tags() {
    let db = test_db();
    let feed = make_feed("f1", "https://example.com/feed.xml");
    db.insert_rss_feed(&feed).unwrap();
    let article = make_article("a1", "f1");
    db.insert_rss_article(&article).unwrap();

    db.insert_rss_article_tags("a1", &["rust", "programming", "ai"])
        .unwrap();

    // Inserting duplicate tags should not error (INSERT OR IGNORE)
    db.insert_rss_article_tags("a1", &["rust", "new-tag"])
        .unwrap();

    let tags = db.get_rss_article_tags("a1").unwrap();
    assert_eq!(tags.len(), 4);
    assert!(tags.contains(&"rust".to_string()));
    assert!(tags.contains(&"programming".to_string()));
    assert!(tags.contains(&"ai".to_string()));
    assert!(tags.contains(&"new-tag".to_string()));

    let all_tags = db.get_all_rss_tags().unwrap();
    assert_eq!(all_tags.len(), 4);
}

#[test]
fn feed_failure_tracking() {
    let db = test_db();
    let feed = make_feed("f1", "https://example.com/feed.xml");
    db.insert_rss_feed(&feed).unwrap();

    // Four failures — feed should still be enabled
    for i in 0..4 {
        db.increment_rss_feed_failures("f1", &format!("error {i}"))
            .unwrap();
    }
    let feeds = db.list_rss_feeds().unwrap();
    assert!(
        feeds[0].enabled,
        "feed should still be enabled after 4 failures"
    );
    assert_eq!(feeds[0].consecutive_failures, 4);

    // Fifth failure triggers auto-disable
    db.increment_rss_feed_failures("f1", "fatal error").unwrap();
    let feeds = db.list_rss_feeds().unwrap();
    assert!(
        !feeds[0].enabled,
        "feed should be disabled after 5 failures"
    );
    assert_eq!(feeds[0].consecutive_failures, 5);
    assert_eq!(feeds[0].last_error.as_deref(), Some("fatal error"));

    // Successful fetch resets failure state
    let other_feed = make_feed("f2", "https://example.com/other.xml");
    db.insert_rss_feed(&other_feed).unwrap();
    db.increment_rss_feed_failures("f2", "temp error").unwrap();
    db.update_rss_feed_fetch_state("f2", 9999).unwrap();
    let feeds = db.list_rss_feeds().unwrap();
    let f2 = feeds.iter().find(|f| f.id == "f2").unwrap();
    assert_eq!(f2.consecutive_failures, 0);
    assert!(f2.last_error.is_none());
    assert_eq!(f2.last_fetched_at_ms, Some(9999));
}

#[test]
fn purge_stale_articles() {
    let db = test_db();
    let feed = make_feed("f1", "https://example.com/feed.xml");
    db.insert_rss_feed(&feed).unwrap();

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let old_ms = now_ms - (10 * 24 * 60 * 60 * 1000); // 10 days ago

    // Old article with status 'new' — should be purged
    let mut old_new = make_article("a-old-new", "f1");
    old_new.created_at_ms = old_ms;
    old_new.url = "https://example.com/a-old-new".to_string();
    db.insert_rss_article(&old_new).unwrap();

    // Old article with status 'accepted' — should also be purged (model already learned)
    let mut old_accepted = make_article("a-old-accepted", "f1");
    old_accepted.created_at_ms = old_ms;
    old_accepted.url = "https://example.com/a-old-accepted".to_string();
    old_accepted.status = "accepted".to_string();
    db.insert_rss_article(&old_accepted).unwrap();

    // Recent article with status 'new' — should survive
    let mut recent_new = make_article("a-recent-new", "f1");
    recent_new.created_at_ms = now_ms;
    recent_new.url = "https://example.com/a-recent-new".to_string();
    db.insert_rss_article(&recent_new).unwrap();

    let purged = db.purge_stale_rss_articles(7).unwrap();
    assert_eq!(purged, 2, "both old articles should be purged");

    let remaining = db.get_rss_articles(None, None, 10, 0).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, "a-recent-new");
}

#[test]
fn resolve_article_id() {
    let db = test_db();
    let feed = make_feed("f1", "https://example.com/feed.xml");
    db.insert_rss_feed(&feed).unwrap();

    let article = make_article("abcdef123456", "f1");
    db.insert_rss_article(&article).unwrap();

    // Exact match
    let resolved = db.resolve_rss_article_id("abcdef123456").unwrap();
    assert_eq!(resolved, "abcdef123456");

    // Short prefix match
    let resolved = db.resolve_rss_article_id("abcdef").unwrap();
    assert_eq!(resolved, "abcdef123456");

    // Ambiguous — two articles share same prefix
    let mut second = make_article("abcdef999999", "f1");
    second.url = "https://example.com/article2".to_string();
    db.insert_rss_article(&second).unwrap();
    let err = db.resolve_rss_article_id("abcdef").unwrap_err();
    assert!(
        err.to_string().contains("ambiguous"),
        "expected ambiguous error, got: {err}"
    );

    // Not found
    let err = db.resolve_rss_article_id("zzzzz").unwrap_err();
    assert!(
        err.to_string().contains("no article found"),
        "expected not found error, got: {err}"
    );
}
