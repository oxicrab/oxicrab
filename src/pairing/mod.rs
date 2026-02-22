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
    /// Acquire an exclusive lock on the pairing directory for cross-process safety.
    /// Lock released when the returned file is dropped.
    fn lock_exclusive(&self) -> Result<std::fs::File> {
        use fs2::FileExt;
        let lock_path = self.base_dir.join(".lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&lock_path)
            .with_context(|| "failed to open pairing lock file")?;
        lock_file
            .lock_exclusive()
            .with_context(|| "failed to acquire pairing lock")?;
        Ok(lock_file)
    }

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

    /// Acquire a shared (read) lock on the pairing directory for cross-process safety.
    /// Lock released when the returned file is dropped.
    fn lock_shared(&self) -> Result<std::fs::File> {
        let lock_path = self.base_dir.join(".lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&lock_path)
            .with_context(|| "failed to open pairing lock file")?;
        fs2::FileExt::lock_shared(&lock_file)
            .with_context(|| "failed to acquire pairing shared lock")?;
        Ok(lock_file)
    }

    fn load_all(&mut self) -> Result<()> {
        let _lock = self.lock_shared()?;

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
        let _lock = self.lock_exclusive()?;
        let path = self.base_dir.join("pending.json");
        let content = serde_json::to_string_pretty(&self.pending)?;
        crate::utils::atomic_write(&path, &content)?;
        Ok(())
    }

    fn save_failed_attempts(&self) -> Result<()> {
        let _lock = self.lock_exclusive()?;
        let path = self.base_dir.join("failed-attempts.json");
        let data = FailedAttemptsData {
            clients: self.failed_attempts.clone(),
        };
        let content = serde_json::to_string(&data)?;
        crate::utils::atomic_write(&path, &content)?;
        Ok(())
    }

    fn save_allowlist(&self, channel: &str) -> Result<()> {
        let _lock = self.lock_exclusive()?;
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
        let idx = self.pending.requests.iter().position(|r| {
            use subtle::ConstantTimeEq;
            let code_match = r.code.as_bytes().ct_eq(code_upper.as_bytes()).into();
            code_match && now.saturating_sub(r.created_at) < CODE_TTL_SECS
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
mod tests;
