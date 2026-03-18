use super::MemoryDB;
use anyhow::Result;
use oxicrab_core::credential_store::{OAuthTokenRow, OAuthTokenStore};
use rusqlite::params;

impl MemoryDB {
    /// Save (insert or replace) an OAuth token for a provider.
    pub fn save_oauth_token(
        &self,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: i64,
        extra_json: Option<&str>,
    ) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO oauth_tokens
             (provider, access_token, refresh_token, expires_at, extra_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
            params![
                provider,
                access_token,
                refresh_token,
                expires_at,
                extra_json
            ],
        )?;
        Ok(())
    }

    /// Load an OAuth token row by provider name. Returns `None` if not found.
    pub fn load_oauth_token(&self, provider: &str) -> Result<Option<OAuthTokenRow>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT provider, access_token, refresh_token, expires_at, extra_json
             FROM oauth_tokens WHERE provider = ?1",
        )?;
        let mut rows = stmt.query(params![provider])?;
        if let Some(row) = rows.next()? {
            Ok(Some(OAuthTokenRow {
                provider: row.get(0)?,
                access_token: row.get(1)?,
                refresh_token: row.get(2)?,
                expires_at: row.get(3)?,
                extra_json: row.get(4)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Delete an OAuth token row by provider name. Returns `true` if a row was deleted.
    pub fn delete_oauth_token(&self, provider: &str) -> Result<bool> {
        let conn = self.lock_conn()?;
        let deleted = conn.execute(
            "DELETE FROM oauth_tokens WHERE provider = ?1",
            params![provider],
        )?;
        Ok(deleted > 0)
    }
}

impl OAuthTokenStore for MemoryDB {
    fn load_token(&self, provider: &str) -> Result<Option<OAuthTokenRow>> {
        self.load_oauth_token(provider)
    }

    fn save_token(
        &self,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: i64,
        extra_json: Option<&str>,
    ) -> Result<()> {
        self.save_oauth_token(
            provider,
            access_token,
            refresh_token,
            expires_at,
            extra_json,
        )
    }

    fn delete_token(&self, provider: &str) -> Result<bool> {
        self.delete_oauth_token(provider)
    }
}

#[cfg(test)]
mod tests {
    use super::super::MemoryDB;

    #[test]
    fn test_save_and_load_oauth_token() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.save_oauth_token("anthropic", "access_tok", Some("refresh_tok"), 99999, None)
            .unwrap();

        let row = db.load_oauth_token("anthropic").unwrap().unwrap();
        assert_eq!(row.provider, "anthropic");
        assert_eq!(row.access_token, "access_tok");
        assert_eq!(row.refresh_token, Some("refresh_tok".to_string()));
        assert_eq!(row.expires_at, 99999);
        assert!(row.extra_json.is_none());
    }

    #[test]
    fn test_save_oauth_token_upsert() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.save_oauth_token("anthropic", "old_tok", Some("old_ref"), 1000, None)
            .unwrap();
        db.save_oauth_token("anthropic", "new_tok", Some("new_ref"), 2000, None)
            .unwrap();

        let row = db.load_oauth_token("anthropic").unwrap().unwrap();
        assert_eq!(row.access_token, "new_tok");
        assert_eq!(row.refresh_token, Some("new_ref".to_string()));
        assert_eq!(row.expires_at, 2000);
    }

    #[test]
    fn test_load_oauth_token_not_found() {
        let db = MemoryDB::new(":memory:").unwrap();
        assert!(db.load_oauth_token("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_delete_oauth_token() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.save_oauth_token("google", "tok", None, 5000, Some(r#"{"client_id":"cid"}"#))
            .unwrap();
        assert!(db.delete_oauth_token("google").unwrap());
        assert!(!db.delete_oauth_token("google").unwrap());
        assert!(db.load_oauth_token("google").unwrap().is_none());
    }

    #[test]
    fn test_save_oauth_token_with_extra_json() {
        let db = MemoryDB::new(":memory:").unwrap();
        let extra = r#"{"client_id":"cid","client_secret":"csec","token_uri":"https://example.com","scopes":["a","b"]}"#;

        db.save_oauth_token("google", "tok", Some("ref"), 9000, Some(extra))
            .unwrap();

        let row = db.load_oauth_token("google").unwrap().unwrap();
        assert_eq!(row.extra_json, Some(extra.to_string()));
    }

    #[test]
    fn test_save_oauth_token_no_refresh() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.save_oauth_token("test", "access", None, 1234, None)
            .unwrap();

        let row = db.load_oauth_token("test").unwrap().unwrap();
        assert!(row.refresh_token.is_none());
    }

    #[test]
    fn test_multiple_providers() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.save_oauth_token("anthropic", "ant_tok", Some("ant_ref"), 1000, None)
            .unwrap();
        db.save_oauth_token("google", "goo_tok", Some("goo_ref"), 2000, None)
            .unwrap();

        let ant = db.load_oauth_token("anthropic").unwrap().unwrap();
        let goo = db.load_oauth_token("google").unwrap().unwrap();
        assert_eq!(ant.access_token, "ant_tok");
        assert_eq!(goo.access_token, "goo_tok");
    }
}
