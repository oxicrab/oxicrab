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
mod tests;
