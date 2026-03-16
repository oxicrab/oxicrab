use anyhow::Result;
use rusqlite::params;

use super::MemoryDB;

#[derive(Debug, Clone)]
pub struct RssFeed {
    pub id: String,
    pub url: String,
    pub name: String,
    pub site_url: Option<String>,
    pub last_fetched_at_ms: Option<i64>,
    pub last_error: Option<String>,
    pub consecutive_failures: i32,
    pub enabled: bool,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct RssArticle {
    pub id: String,
    pub feed_id: String,
    pub url: String,
    pub title: String,
    pub author: Option<String>,
    pub published_at_ms: Option<i64>,
    pub fetched_at_ms: i64,
    pub description: Option<String>,
    pub full_content: Option<String>,
    pub summary: Option<String>,
    pub status: String,
    pub read: bool,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct RssProfile {
    pub interests: String,
    pub onboarding_state: String,
    pub cron_job_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

pub const STATE_NEEDS_PROFILE: &str = "needs_profile";
pub const STATE_NEEDS_FEEDS: &str = "needs_feeds";
pub const STATE_NEEDS_CALIBRATION: &str = "needs_calibration";
pub const STATE_COMPLETE: &str = "complete";

/// `(feature_index, mu, sigma)` returned by `load_rss_model`.
pub type RssModelRow = (String, Vec<u8>, Vec<u8>);

impl MemoryDB {
    pub fn insert_rss_feed(&self, feed: &RssFeed) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "INSERT INTO rss_feeds (id, url, name, site_url, last_fetched_at_ms, last_error,
                                    consecutive_failures, enabled, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                feed.id,
                feed.url,
                feed.name,
                feed.site_url,
                feed.last_fetched_at_ms,
                feed.last_error,
                feed.consecutive_failures,
                i32::from(feed.enabled),
                feed.created_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn list_rss_feeds(&self) -> Result<Vec<RssFeed>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, url, name, site_url, last_fetched_at_ms, last_error,
                    consecutive_failures, enabled, created_at_ms
             FROM rss_feeds ORDER BY created_at_ms",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let enabled: i32 = row.get(7)?;
                Ok(RssFeed {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    name: row.get(2)?,
                    site_url: row.get(3)?,
                    last_fetched_at_ms: row.get(4)?,
                    last_error: row.get(5)?,
                    consecutive_failures: row.get(6)?,
                    enabled: enabled != 0,
                    created_at_ms: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_rss_feed(&self, id: &str) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let deleted = conn.execute("DELETE FROM rss_feeds WHERE id = ?1", params![id])?;
        Ok(deleted)
    }

    pub fn insert_rss_article(&self, article: &RssArticle) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "INSERT INTO rss_articles
                (id, feed_id, url, title, author, published_at_ms, fetched_at_ms,
                 description, full_content, summary, status, read, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                article.id,
                article.feed_id,
                article.url,
                article.title,
                article.author,
                article.published_at_ms,
                article.fetched_at_ms,
                article.description,
                article.full_content,
                article.summary,
                article.status,
                i32::from(article.read),
                article.created_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn get_rss_articles(
        &self,
        status: Option<&str>,
        feed_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RssArticle>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

        let mut conditions: Vec<String> = Vec::new();
        let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(s) = status {
            bind_values.push(Box::new(s.to_string()));
            conditions.push(format!("status = ?{}", bind_values.len()));
        }
        if let Some(f) = feed_id {
            bind_values.push(Box::new(f.to_string()));
            conditions.push(format!("feed_id = ?{}", bind_values.len()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        bind_values.push(Box::new(limit as i64));
        let limit_idx = bind_values.len();
        bind_values.push(Box::new(offset as i64));
        let offset_idx = bind_values.len();

        let sql = format!(
            "SELECT id, feed_id, url, title, author, published_at_ms, fetched_at_ms,
                    description, full_content, summary, status, read, created_at_ms
             FROM rss_articles {where_clause}
             ORDER BY created_at_ms DESC
             LIMIT ?{limit_idx} OFFSET ?{offset_idx}"
        );

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            bind_values.iter().map(AsRef::as_ref).collect();

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params_ref.as_slice(), |row| {
                let read: i32 = row.get(11)?;
                Ok(RssArticle {
                    id: row.get(0)?,
                    feed_id: row.get(1)?,
                    url: row.get(2)?,
                    title: row.get(3)?,
                    author: row.get(4)?,
                    published_at_ms: row.get(5)?,
                    fetched_at_ms: row.get(6)?,
                    description: row.get(7)?,
                    full_content: row.get(8)?,
                    summary: row.get(9)?,
                    status: row.get(10)?,
                    read: read != 0,
                    created_at_ms: row.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_rss_article(&self, id: &str) -> Result<Option<RssArticle>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, feed_id, url, title, author, published_at_ms, fetched_at_ms,
                    description, full_content, summary, status, read, created_at_ms
             FROM rss_articles WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            let read: i32 = row.get(11)?;
            Ok(Some(RssArticle {
                id: row.get(0)?,
                feed_id: row.get(1)?,
                url: row.get(2)?,
                title: row.get(3)?,
                author: row.get(4)?,
                published_at_ms: row.get(5)?,
                fetched_at_ms: row.get(6)?,
                description: row.get(7)?,
                full_content: row.get(8)?,
                summary: row.get(9)?,
                status: row.get(10)?,
                read: read != 0,
                created_at_ms: row.get(12)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Resolve a short article ID prefix to a full ID.
    /// Returns an error if zero or more than one article matches.
    pub fn resolve_rss_article_id(&self, short_id: &str) -> Result<String> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let pattern = format!("{short_id}%");
        let mut stmt = conn.prepare("SELECT id FROM rss_articles WHERE id LIKE ?1")?;
        let ids: Vec<String> = stmt
            .query_map(params![pattern], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        match ids.len() {
            0 => anyhow::bail!("no article found matching id prefix '{short_id}'"),
            1 => Ok(ids.into_iter().next().unwrap()),
            n => anyhow::bail!("ambiguous id prefix '{short_id}' matched {n} articles"),
        }
    }

    pub fn update_rss_article_status(&self, id: &str, status: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "UPDATE rss_articles SET status = ?1 WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    pub fn update_rss_article_full_content(&self, id: &str, content: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "UPDATE rss_articles SET full_content = ?1, read = 1 WHERE id = ?2",
            params![content, id],
        )?;
        Ok(())
    }

    pub fn insert_rss_article_tags(&self, article_id: &str, tags: &[&str]) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        for tag in tags {
            conn.execute(
                "INSERT OR IGNORE INTO rss_article_tags (article_id, tag) VALUES (?1, ?2)",
                params![article_id, tag],
            )?;
        }
        Ok(())
    }

    pub fn get_rss_article_tags(&self, article_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let mut stmt =
            conn.prepare("SELECT tag FROM rss_article_tags WHERE article_id = ?1 ORDER BY tag")?;
        let tags = stmt
            .query_map(params![article_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tags)
    }

    pub fn get_all_rss_tags(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let mut stmt = conn.prepare("SELECT DISTINCT tag FROM rss_article_tags ORDER BY tag")?;
        let tags = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tags)
    }

    pub fn get_rss_profile(&self) -> Result<Option<RssProfile>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT interests, onboarding_state, cron_job_id, created_at_ms, updated_at_ms
             FROM rss_profile WHERE id = 1",
        )?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            Ok(Some(RssProfile {
                interests: row.get(0)?,
                onboarding_state: row.get(1)?,
                cron_job_id: row.get(2)?,
                created_at_ms: row.get(3)?,
                updated_at_ms: row.get(4)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn set_rss_profile(&self, interests: &str, state: &str, now_ms: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "INSERT INTO rss_profile (id, interests, onboarding_state, created_at_ms, updated_at_ms)
             VALUES (1, ?1, ?2, ?3, ?3)
             ON CONFLICT(id) DO UPDATE SET interests = excluded.interests,
                                           onboarding_state = excluded.onboarding_state,
                                           updated_at_ms = excluded.updated_at_ms",
            params![interests, state, now_ms],
        )?;
        Ok(())
    }

    pub fn set_rss_onboarding_state(&self, state: &str, now_ms: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "UPDATE rss_profile SET onboarding_state = ?1, updated_at_ms = ?2 WHERE id = 1",
            params![state, now_ms],
        )?;
        Ok(())
    }

    pub fn set_rss_cron_job_id(&self, job_id: &str, now_ms: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "UPDATE rss_profile SET cron_job_id = ?1, updated_at_ms = ?2 WHERE id = 1",
            params![job_id, now_ms],
        )?;
        Ok(())
    }

    /// Reset the feed's failure state after a successful fetch.
    pub fn update_rss_feed_fetch_state(&self, id: &str, now_ms: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "UPDATE rss_feeds SET last_fetched_at_ms = ?1, consecutive_failures = 0,
                                  last_error = NULL
             WHERE id = ?2",
            params![now_ms, id],
        )?;
        Ok(())
    }

    /// Increment the consecutive failure counter. Disables the feed at >=5 failures.
    pub fn increment_rss_feed_failures(&self, id: &str, error: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "UPDATE rss_feeds
             SET consecutive_failures = consecutive_failures + 1,
                 last_error = ?1,
                 enabled = CASE WHEN consecutive_failures + 1 >= 5 THEN 0 ELSE enabled END
             WHERE id = ?2",
            params![error, id],
        )?;
        Ok(())
    }

    pub fn disable_rss_feed(&self, id: &str, reason: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "UPDATE rss_feeds SET enabled = 0, last_error = ?1 WHERE id = ?2",
            params![reason, id],
        )?;
        Ok(())
    }

    pub fn count_rss_feeds(&self) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM rss_feeds", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    pub fn count_rss_reviews(&self) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM rss_articles WHERE status IN ('accepted', 'rejected')",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Delete stale articles with status 'new' or 'triaged' older than `days` days.
    /// Returns the number of rows deleted.
    pub fn purge_stale_rss_articles(&self, days: u64) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let cutoff_ms = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .saturating_sub(u128::from(days) * 24 * 60 * 60 * 1000),
        )
        .unwrap_or(i64::MAX);
        let deleted = conn.execute(
            "DELETE FROM rss_articles
             WHERE status IN ('new', 'triaged') AND created_at_ms < ?1",
            params![cutoff_ms],
        )?;
        Ok(deleted)
    }

    pub fn save_rss_model(
        &self,
        feature_index: &str,
        mu: &[u8],
        sigma: &[u8],
        now_ms: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "INSERT INTO rss_model (id, feature_index, mu, sigma, updated_at_ms)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET feature_index = excluded.feature_index,
                                           mu = excluded.mu,
                                           sigma = excluded.sigma,
                                           updated_at_ms = excluded.updated_at_ms",
            params![feature_index, mu, sigma, now_ms],
        )?;
        Ok(())
    }

    pub fn load_rss_model(&self) -> Result<Option<RssModelRow>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let mut stmt =
            conn.prepare("SELECT feature_index, mu, sigma FROM rss_model WHERE id = 1")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let feature_index: String = row.get(0)?;
            let mu: Vec<u8> = row.get(1)?;
            let sigma: Vec<u8> = row.get(2)?;
            Ok(Some((feature_index, mu, sigma)))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
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

        // Old article with status 'accepted' — should survive
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
        assert_eq!(purged, 1, "only the old new article should be purged");

        let remaining = db.get_rss_articles(None, None, 10, 0).unwrap();
        let ids: Vec<&str> = remaining.iter().map(|a| a.id.as_str()).collect();
        assert!(
            !ids.contains(&"a-old-new"),
            "old new article should be gone"
        );
        assert!(
            ids.contains(&"a-old-accepted"),
            "old accepted article should survive"
        );
        assert!(
            ids.contains(&"a-recent-new"),
            "recent new article should survive"
        );
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
}
