use std::collections::HashMap;

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
        let conn = self.lock_conn()?;
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
        let conn = self.lock_conn()?;
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
        let conn = self.lock_conn()?;
        let deleted = conn.execute("DELETE FROM rss_feeds WHERE id = ?1", params![id])?;
        Ok(deleted)
    }

    pub fn insert_rss_article(&self, article: &RssArticle) -> Result<()> {
        let conn = self.lock_conn()?;
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
        let conn = self.lock_conn()?;

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
        let conn = self.lock_conn()?;
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
        let conn = self.lock_conn()?;
        // Escape LIKE wildcards to prevent injection via short_id containing % or _
        let escaped = short_id
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("{escaped}%");
        let mut stmt = conn.prepare("SELECT id FROM rss_articles WHERE id LIKE ?1 ESCAPE '\\'")?;
        let ids: Vec<String> = stmt
            .query_map(params![pattern], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        match ids.len() {
            0 => anyhow::bail!("no article found matching id prefix '{short_id}'"),
            1 => Ok(ids.into_iter().next().unwrap()),
            n => anyhow::bail!("ambiguous id prefix '{short_id}' matched {n} articles"),
        }
    }

    /// Resolve a short feed ID prefix to a full ID.
    /// Returns an error if zero or more than one feed matches.
    pub fn resolve_rss_feed_id(&self, short_id: &str) -> Result<String> {
        let conn = self.lock_conn()?;
        let escaped = short_id
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("{escaped}%");
        let mut stmt = conn.prepare("SELECT id FROM rss_feeds WHERE id LIKE ?1 ESCAPE '\\'")?;
        let ids: Vec<String> = stmt
            .query_map(params![pattern], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        match ids.len() {
            0 => anyhow::bail!("no feed found matching id prefix '{short_id}'"),
            1 => Ok(ids.into_iter().next().unwrap()),
            n => anyhow::bail!("ambiguous id prefix '{short_id}' matched {n} feeds"),
        }
    }

    /// Re-enable a previously disabled feed and reset its failure counter.
    pub fn enable_rss_feed(&self, id: &str) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE rss_feeds SET enabled = 1, consecutive_failures = 0, last_error = NULL WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn update_rss_article_status(&self, id: &str, status: &str) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE rss_articles SET status = ?1 WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    pub fn update_rss_article_full_content(&self, id: &str, content: &str) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE rss_articles SET full_content = ?1, read = 1 WHERE id = ?2",
            params![content, id],
        )?;
        Ok(())
    }

    pub fn insert_rss_article_tags(&self, article_id: &str, tags: &[&str]) -> Result<()> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        for tag in tags {
            tx.execute(
                "INSERT OR IGNORE INTO rss_article_tags (article_id, tag) VALUES (?1, ?2)",
                params![article_id, tag],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_rss_article_tags(&self, article_id: &str) -> Result<Vec<String>> {
        let conn = self.lock_conn()?;
        let mut stmt =
            conn.prepare("SELECT tag FROM rss_article_tags WHERE article_id = ?1 ORDER BY tag")?;
        let tags = stmt
            .query_map(params![article_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tags)
    }

    /// Batch-fetch tags for multiple articles.
    /// Returns a map from article ID to its tag list.
    /// Chunks queries into batches of 500 to stay within bind-variable limits.
    pub fn get_rss_article_tags_batch(
        &self,
        article_ids: &[&str],
    ) -> Result<HashMap<String, Vec<String>>> {
        if article_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.lock_conn()?;
        let mut map: HashMap<String, Vec<String>> = HashMap::new();

        // SQLite default SQLITE_MAX_VARIABLE_NUMBER is 999; chunk to stay safe
        for chunk in article_ids.chunks(500) {
            let placeholders: Vec<String> = (1..=chunk.len()).map(|i| format!("?{i}")).collect();
            let sql = format!(
                "SELECT article_id, tag FROM rss_article_tags WHERE article_id IN ({}) ORDER BY article_id, tag",
                placeholders.join(", ")
            );
            let mut stmt = conn.prepare(&sql)?;
            let params: Vec<&dyn rusqlite::types::ToSql> = chunk
                .iter()
                .map(|id| id as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = stmt.query_map(params.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (article_id, tag) = row?;
                map.entry(article_id).or_default().push(tag);
            }
        }

        Ok(map)
    }

    pub fn get_all_rss_tags(&self) -> Result<Vec<String>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare("SELECT DISTINCT tag FROM rss_article_tags ORDER BY tag")?;
        let tags = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tags)
    }

    pub fn get_rss_profile(&self) -> Result<Option<RssProfile>> {
        let conn = self.lock_conn()?;
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
        let conn = self.lock_conn()?;
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
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE rss_profile SET onboarding_state = ?1, updated_at_ms = ?2 WHERE id = 1",
            params![state, now_ms],
        )?;
        Ok(())
    }

    pub fn set_rss_cron_job_id(&self, job_id: &str, now_ms: i64) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE rss_profile SET cron_job_id = ?1, updated_at_ms = ?2 WHERE id = 1",
            params![job_id, now_ms],
        )?;
        Ok(())
    }

    /// Reset the feed's failure state after a successful fetch.
    pub fn update_rss_feed_fetch_state(&self, id: &str, now_ms: i64) -> Result<()> {
        let conn = self.lock_conn()?;
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
        let conn = self.lock_conn()?;
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

    /// Count articles optionally filtered by status and/or `feed_id`.
    pub fn count_rss_articles(&self, status: Option<&str>, feed_id: Option<&str>) -> Result<usize> {
        let conn = self.lock_conn()?;

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

        let sql = format!("SELECT COUNT(*) FROM rss_articles {where_clause}");
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            bind_values.iter().map(AsRef::as_ref).collect();

        let count: i64 = conn.query_row(&sql, params_ref.as_slice(), |row| row.get(0))?;
        Ok(count as usize)
    }

    pub fn count_rss_feeds(&self) -> Result<usize> {
        let conn = self.lock_conn()?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM rss_feeds", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    pub fn count_rss_reviews(&self) -> Result<usize> {
        let conn = self.lock_conn()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM rss_articles WHERE status IN ('accepted', 'rejected')",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Delete stale articles older than `days` days.
    /// Unreviewed (`new`) articles are purged first. Terminal-state articles
    /// (`accepted`/`rejected`) older than `days` are also purged — the `LinTS`
    /// model has already learned from them, so the feedback signal is preserved
    /// in the model weights, not the article rows.
    /// Returns the number of rows deleted.
    pub fn purge_stale_rss_articles(&self, days: u64) -> Result<usize> {
        let conn = self.lock_conn()?;
        let cutoff_ms = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .saturating_sub(u128::from(days) * 24 * 60 * 60 * 1000),
        )
        .unwrap_or(0);
        let deleted = conn.execute(
            "DELETE FROM rss_articles WHERE created_at_ms < ?1",
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
        let conn = self.lock_conn()?;
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
        let conn = self.lock_conn()?;
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
mod tests;
