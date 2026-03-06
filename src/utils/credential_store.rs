use crate::agent::memory::memory_db::{MemoryDB, OAuthTokenRow};
use anyhow::Result;
use std::path::Path;

/// Storage abstraction for OAuth token state.
pub trait OAuthTokenStore {
    fn load_token(&self, provider: &str) -> Result<Option<OAuthTokenRow>>;
    fn save_token(
        &self,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: i64,
        extra_json: Option<&str>,
    ) -> Result<()>;
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
}

/// Load an OAuth token row from a state store, if available.
pub fn load_oauth_token(
    store: Option<&dyn OAuthTokenStore>,
    provider: &str,
) -> Result<Option<OAuthTokenRow>> {
    let Some(store) = store else {
        return Ok(None);
    };
    store.load_token(provider)
}

/// Save an OAuth token row into a state store, if available.
///
/// Returns `Ok(true)` if saved to state store, `Ok(false)` if no store was provided.
pub fn save_oauth_token(
    store: Option<&dyn OAuthTokenStore>,
    provider: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: i64,
    extra_json: Option<&str>,
) -> Result<bool> {
    let Some(store) = store else {
        return Ok(false);
    };
    store.save_token(
        provider,
        access_token,
        refresh_token,
        expires_at,
        extra_json,
    )?;
    Ok(true)
}

/// Read JSON from disk with lock + parse validation.
pub fn read_json_locked<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    crate::utils::io_safe::read_json_locked(path)
}

/// Write JSON to disk with lock + atomic replace + owner-only perms.
pub fn write_json_locked<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    crate::utils::io_safe::write_json_locked(path, value)
}
