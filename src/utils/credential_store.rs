use anyhow::Result;
use std::path::Path;

/// A row from the `oauth_tokens` table.
#[derive(Debug, Clone)]
pub struct OAuthTokenRow {
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
    pub extra_json: Option<String>,
}

/// Storage abstraction for OAuth token state.
pub trait OAuthTokenStore: Send + Sync {
    fn load_token(&self, provider: &str) -> Result<Option<OAuthTokenRow>>;
    fn save_token(
        &self,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: i64,
        extra_json: Option<&str>,
    ) -> Result<()>;
    fn delete_token(&self, provider: &str) -> Result<bool>;
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
