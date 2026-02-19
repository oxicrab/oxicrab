use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, warn};

/// Alphabet for human-friendly pairing codes (no 0/O/1/I to avoid confusion)
const CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const CODE_LENGTH: usize = 8;
const CODE_TTL_SECS: u64 = 15 * 60; // 15 minutes
const MAX_PENDING_PER_CHANNEL: usize = 3;
const MAX_FAILED_ATTEMPTS: usize = 10;
const FAILED_ATTEMPT_WINDOW_SECS: u64 = 5 * 60; // 5 minutes
const MAX_LOCKOUT_CLIENTS: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingRequest {
    pub channel: String,
    pub sender_id: String,
    pub code: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AllowlistData {
    senders: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PendingData {
    requests: Vec<PendingRequest>,
}

/// Persisted failed approval attempts for brute-force lockout.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct FailedAttemptsData {
    /// Map from `client_id` → list of attempt timestamps (unix secs)
    clients: HashMap<String, Vec<u64>>,
}

pub struct PairingStore {
    base_dir: PathBuf,
    allowlists: HashMap<String, AllowlistData>,
    pending: PendingData,
    failed_attempts: HashMap<String, Vec<u64>>,
}

impl PairingStore {
    pub fn new() -> Result<Self> {
        let base_dir = crate::utils::get_oxicrab_home()?.join("pairing");
        std::fs::create_dir_all(&base_dir)
            .with_context(|| format!("failed to create pairing dir: {}", base_dir.display()))?;

        let mut store = Self {
            base_dir,
            allowlists: HashMap::new(),
            pending: PendingData::default(),
            failed_attempts: HashMap::new(),
        };
        store.load_all()?;
        Ok(store)
    }

    #[cfg(test)]
    fn with_dir(base_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&base_dir)?;
        let mut store = Self {
            base_dir,
            allowlists: HashMap::new(),
            pending: PendingData::default(),
            failed_attempts: HashMap::new(),
        };
        store.load_all()?;
        Ok(store)
    }

    fn load_all(&mut self) -> Result<()> {
        // Load pending requests
        let pending_path = self.base_dir.join("pending.json");
        if pending_path.exists() {
            let content = std::fs::read_to_string(&pending_path)?;
            self.pending = serde_json::from_str(&content).unwrap_or_default();
        }

        // Load persisted failed attempts for brute-force lockout
        let failed_path = self.base_dir.join("failed-attempts.json");
        if failed_path.exists()
            && let Ok(content) = std::fs::read_to_string(&failed_path)
            && let Ok(data) = serde_json::from_str::<FailedAttemptsData>(&content)
        {
            self.failed_attempts = data.clients;
            // Prune expired entries on load
            let now = Self::now_secs();
            self.failed_attempts.retain(|_, ts| {
                ts.retain(|&t| now.saturating_sub(t) < FAILED_ATTEMPT_WINDOW_SECS);
                !ts.is_empty()
            });
        }

        // Load per-channel allowlists
        for entry in std::fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(channel) = name.strip_suffix("-allowlist.json") {
                let content = std::fs::read_to_string(entry.path())?;
                let data: AllowlistData = serde_json::from_str(&content).unwrap_or_default();
                self.allowlists.insert(channel.to_string(), data);
            }
        }

        Ok(())
    }

    fn save_pending(&self) -> Result<()> {
        let path = self.base_dir.join("pending.json");
        let content = serde_json::to_string_pretty(&self.pending)?;
        crate::utils::atomic_write(&path, &content)?;
        Ok(())
    }

    fn save_failed_attempts(&self) -> Result<()> {
        let path = self.base_dir.join("failed-attempts.json");
        let data = FailedAttemptsData {
            clients: self.failed_attempts.clone(),
        };
        let content = serde_json::to_string(&data)?;
        crate::utils::atomic_write(&path, &content)?;
        Ok(())
    }

    fn save_allowlist(&self, channel: &str) -> Result<()> {
        let path = self.base_dir.join(format!("{}-allowlist.json", channel));
        let data = self.allowlists.get(channel).cloned().unwrap_or_default();
        let content = serde_json::to_string_pretty(&data)?;
        crate::utils::atomic_write(&path, &content)?;
        Ok(())
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn generate_code() -> String {
        let mut code = String::with_capacity(CODE_LENGTH);
        for _ in 0..CODE_LENGTH {
            let idx = fastrand::usize(0..CODE_ALPHABET.len());
            code.push(CODE_ALPHABET[idx] as char);
        }
        code
    }

    /// Request pairing for a sender on a channel.
    /// Returns `Some(code)` if a new code was issued, `None` if rate-limited.
    pub fn request_pairing(&mut self, channel: &str, sender_id: &str) -> Result<Option<String>> {
        self.cleanup_expired();

        // Check if already paired
        if self.is_paired(channel, sender_id) {
            return Ok(None);
        }

        // Check pending count per channel
        let pending_count = self
            .pending
            .requests
            .iter()
            .filter(|r| r.channel == channel)
            .count();
        if pending_count >= MAX_PENDING_PER_CHANNEL {
            debug!(
                "max pending pairing requests for channel {} reached",
                channel
            );
            return Ok(None);
        }

        // Check if this sender already has a pending request
        if let Some(existing) = self
            .pending
            .requests
            .iter()
            .find(|r| r.channel == channel && r.sender_id == sender_id)
        {
            return Ok(Some(existing.code.clone()));
        }

        let code = Self::generate_code();
        self.pending.requests.push(PendingRequest {
            channel: channel.to_string(),
            sender_id: sender_id.to_string(),
            code: code.clone(),
            created_at: Self::now_secs(),
        });
        self.save_pending()?;

        Ok(Some(code))
    }

    /// Approve a pairing request by code, with per-client lockout.
    /// `client_id` identifies the approver (e.g. CLI user, admin session).
    /// Returns `(channel, sender_id)` on success.
    pub fn approve_with_client(
        &mut self,
        code: &str,
        client_id: &str,
    ) -> Result<Option<(String, String)>> {
        let now = Self::now_secs();

        // Evict oldest client entries if map is too large (DoS protection)
        if self.failed_attempts.len() > MAX_LOCKOUT_CLIENTS {
            // Find the client with the oldest most-recent attempt and remove it
            if let Some(oldest_key) = self
                .failed_attempts
                .iter()
                .min_by_key(|(_, attempts)| attempts.last().copied().unwrap_or(0))
                .map(|(k, _)| k.clone())
            {
                self.failed_attempts.remove(&oldest_key);
            }
        }

        // Per-client rate limiting
        let attempts = self
            .failed_attempts
            .entry(client_id.to_string())
            .or_default();
        attempts.retain(|&t| now.saturating_sub(t) < FAILED_ATTEMPT_WINDOW_SECS);
        if attempts.len() >= MAX_FAILED_ATTEMPTS {
            anyhow::bail!("too many failed approval attempts, try again later");
        }

        let code_upper = code.to_uppercase();
        let idx =
            self.pending.requests.iter().position(|r| {
                r.code == code_upper && now.saturating_sub(r.created_at) < CODE_TTL_SECS
            });

        let Some(idx) = idx else {
            self.failed_attempts
                .entry(client_id.to_string())
                .or_default()
                .push(now);
            // Persist so lockout survives PairingStore recreation
            if let Err(e) = self.save_failed_attempts() {
                warn!("failed to persist failed attempts: {}", e);
            }
            return Ok(None);
        };

        let request = self.pending.requests.remove(idx);
        let channel = request.channel.clone();
        let sender_id = request.sender_id.clone();

        // Add to allowlist
        let allowlist = self.allowlists.entry(channel.clone()).or_default();
        if !allowlist.senders.contains(&sender_id) {
            allowlist.senders.push(sender_id.clone());
        }

        self.save_pending()?;
        self.save_allowlist(&channel)?;

        Ok(Some((channel, sender_id)))
    }

    /// Approve a pairing request by code. Returns `(channel, sender_id)` on success.
    /// Uses a default client ID for lockout tracking.
    pub fn approve(&mut self, code: &str) -> Result<Option<(String, String)>> {
        self.approve_with_client(code, "default")
    }

    /// Check if a sender is in the pairing store's allowlist for a channel.
    pub fn is_paired(&self, channel: &str, sender_id: &str) -> bool {
        self.allowlists
            .get(channel)
            .is_some_and(|data| data.senders.iter().any(|s| s == sender_id))
    }

    /// List all pending pairing requests (non-expired).
    pub fn list_pending(&self) -> Vec<&PendingRequest> {
        let now = Self::now_secs();
        self.pending
            .requests
            .iter()
            .filter(|r| now.saturating_sub(r.created_at) < CODE_TTL_SECS)
            .collect()
    }

    /// List all paired senders across all channels.
    pub fn paired_count(&self) -> usize {
        self.allowlists.values().map(|a| a.senders.len()).sum()
    }

    /// Revoke a sender's pairing for a channel.
    pub fn revoke(&mut self, channel: &str, sender_id: &str) -> Result<bool> {
        if let Some(allowlist) = self.allowlists.get_mut(channel) {
            let before = allowlist.senders.len();
            allowlist.senders.retain(|s| s != sender_id);
            if allowlist.senders.len() < before {
                self.save_allowlist(channel)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Remove expired pending requests.
    pub fn cleanup_expired(&mut self) {
        let now = Self::now_secs();
        let before = self.pending.requests.len();
        self.pending
            .requests
            .retain(|r| now.saturating_sub(r.created_at) < CODE_TTL_SECS);
        if self.pending.requests.len() < before
            && let Err(e) = self.save_pending()
        {
            warn!("failed to save pending after cleanup: {}", e);
        }
    }

    /// List all paired senders for a specific channel.
    pub fn list_channel_senders(&self, channel: &str) -> Option<Vec<String>> {
        self.allowlists
            .get(channel)
            .map(|data| data.senders.clone())
    }

    /// Check if the pairing store directory exists.
    pub fn store_exists() -> bool {
        crate::utils::get_oxicrab_home().is_ok_and(|h| h.join("pairing").exists())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_code_format() {
        let code = PairingStore::generate_code();
        assert_eq!(code.len(), CODE_LENGTH);
        for c in code.chars() {
            assert!(
                CODE_ALPHABET.contains(&(c as u8)),
                "invalid char in code: {}",
                c
            );
        }
    }

    #[test]
    fn test_request_and_approve() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

        assert!(!store.is_paired("telegram", "user123"));

        let code = store
            .request_pairing("telegram", "user123")
            .unwrap()
            .unwrap();
        assert_eq!(code.len(), CODE_LENGTH);

        // Approve with the code
        let result = store.approve(&code).unwrap();
        assert!(result.is_some());
        let (channel, sender) = result.unwrap();
        assert_eq!(channel, "telegram");
        assert_eq!(sender, "user123");

        // Now paired
        assert!(store.is_paired("telegram", "user123"));
    }

    #[test]
    fn test_approve_invalid_code() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

        store
            .request_pairing("telegram", "user123")
            .unwrap()
            .unwrap();
        let result = store.approve("BADCODE1").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_revoke() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

        let code = store
            .request_pairing("discord", "user456")
            .unwrap()
            .unwrap();
        store.approve(&code).unwrap();
        assert!(store.is_paired("discord", "user456"));

        let revoked = store.revoke("discord", "user456").unwrap();
        assert!(revoked);
        assert!(!store.is_paired("discord", "user456"));
    }

    #[test]
    fn test_duplicate_request_returns_same_code() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

        let code1 = store
            .request_pairing("telegram", "user123")
            .unwrap()
            .unwrap();
        let code2 = store
            .request_pairing("telegram", "user123")
            .unwrap()
            .unwrap();
        assert_eq!(code1, code2);
    }

    #[test]
    fn test_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        // Create store, add a pairing
        {
            let mut store = PairingStore::with_dir(path.clone()).unwrap();
            let code = store.request_pairing("slack", "user789").unwrap().unwrap();
            store.approve(&code).unwrap();
        }

        // Reload and check
        {
            let store = PairingStore::with_dir(path).unwrap();
            assert!(store.is_paired("slack", "user789"));
        }
    }

    #[test]
    fn test_list_pending() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

        store.request_pairing("telegram", "user1").unwrap();
        store.request_pairing("telegram", "user2").unwrap();

        let pending = store.list_pending();
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn test_max_pending_per_channel() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

        for i in 0..MAX_PENDING_PER_CHANNEL {
            let result = store
                .request_pairing("telegram", &format!("user{}", i))
                .unwrap();
            assert!(result.is_some());
        }

        // Next request should be rate-limited
        let result = store.request_pairing("telegram", "overflow_user").unwrap();
        assert!(result.is_none());

        // But a different channel should still work
        let result = store.request_pairing("discord", "overflow_user").unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_approve_with_client_success() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

        let code = store
            .request_pairing("telegram", "user123")
            .unwrap()
            .unwrap();
        let result = store.approve_with_client(&code, "admin1").unwrap();
        assert!(result.is_some());
        let (channel, sender) = result.unwrap();
        assert_eq!(channel, "telegram");
        assert_eq!(sender, "user123");
        assert!(store.is_paired("telegram", "user123"));
    }

    #[test]
    fn test_approve_with_client_records_failed_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

        store
            .request_pairing("telegram", "user123")
            .unwrap()
            .unwrap();

        // Try a bad code
        let result = store.approve_with_client("BADCODE1", "admin1").unwrap();
        assert!(result.is_none());

        // Should have recorded one failed attempt
        assert_eq!(store.failed_attempts["admin1"].len(), 1);
    }

    #[test]
    fn test_approve_with_client_locks_out_after_max_failures() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

        store
            .request_pairing("telegram", "user123")
            .unwrap()
            .unwrap();

        // Exhaust the failure budget
        for _ in 0..MAX_FAILED_ATTEMPTS {
            let _ = store.approve_with_client("BADCODE1", "admin1");
        }

        // Next attempt should bail
        let result = store.approve_with_client("BADCODE1", "admin1");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("too many failed approval attempts")
        );
    }

    #[test]
    fn test_approve_with_client_separate_limits_per_client() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

        let code = store
            .request_pairing("telegram", "user123")
            .unwrap()
            .unwrap();

        // Exhaust admin1's budget
        for _ in 0..MAX_FAILED_ATTEMPTS {
            let _ = store.approve_with_client("BADCODE1", "admin1");
        }

        // admin1 is locked out
        assert!(store.approve_with_client("BADCODE1", "admin1").is_err());

        // admin2 can still approve successfully
        let result = store.approve_with_client(&code, "admin2").unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_approve_default_uses_default_client() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

        store
            .request_pairing("telegram", "user123")
            .unwrap()
            .unwrap();

        // approve() delegates to approve_with_client("default")
        let _ = store.approve("BADCODE1");
        assert!(store.failed_attempts.contains_key("default"));
    }

    #[test]
    fn test_case_insensitive_code_matching() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

        let code = store
            .request_pairing("telegram", "user123")
            .unwrap()
            .unwrap();

        // Approve with lowercase version of the code
        let lower = code.to_lowercase();
        let result = store.approve(&lower).unwrap();
        assert!(result.is_some());
    }
}
