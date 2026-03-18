use std::path::Path;

// Re-export OAuth types from core
pub use oxicrab_core::credential_store::{OAuthTokenStore, load_oauth_token, save_oauth_token};

/// Read JSON from disk with lock + parse validation.
pub fn read_json_locked<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<Option<T>> {
    crate::utils::io_safe::read_json_locked(path)
}

/// Write JSON to disk with lock + atomic replace + owner-only perms.
pub fn write_json_locked<T: serde::Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    crate::utils::io_safe::write_json_locked(path, value)
}
