use crate::agent::memory::memory_db::MemoryDB;
use anyhow::Result;
use std::sync::Arc;
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

#[derive(Debug, Clone)]
pub struct PendingRequest {
    pub channel: String,
    pub sender_id: String,
    pub code: String,
    pub created_at: u64,
}

pub struct PairingStore {
    db: Arc<MemoryDB>,
}

impl PairingStore {
    pub fn new(db: Arc<MemoryDB>) -> Self {
        Self { db }
    }

    /// Open a `PairingStore` backed by the default workspace `MemoryDB`.
    /// Convenience for CLI commands and places that don't have a shared DB reference.
    pub fn open_default() -> Result<Self> {
        let db_path = crate::utils::get_memory_db_path()?;
        let db = Arc::new(MemoryDB::new(&db_path)?);
        Ok(Self { db })
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Generate a pairing code using CSPRNG-backed randomness.
    ///
    /// Uses `uuid::Uuid::new_v4()` as the entropy source (backed by the OS
    /// CSPRNG via `getrandom`), rather than `fastrand` which uses a
    /// non-cryptographic PRNG. Pairing codes are security-sensitive since
    /// they grant message access to new senders.
    fn generate_code() -> String {
        let mut code = String::with_capacity(CODE_LENGTH);
        // Each UUID provides 16 random bytes (122 bits of entropy).
        // We need CODE_LENGTH (8) indices into a 31-element alphabet.
        // Use rejection sampling to avoid modulo bias.
        let alphabet_len = CODE_ALPHABET.len() as u8; // 31
        let mut bytes_used = 16; // force initial UUID generation
        let mut random_bytes = [0u8; 16];
        while code.len() < CODE_LENGTH {
            if bytes_used >= 16 {
                random_bytes = *uuid::Uuid::new_v4().as_bytes();
                bytes_used = 0;
            }
            let b = random_bytes[bytes_used];
            bytes_used += 1;
            // Rejection sampling: discard values that would cause modulo bias.
            // 256 / 31 = 8 remainder 8, so accept values < 31 * 8 = 248.
            if b < alphabet_len.wrapping_mul(8) {
                let idx = (b % alphabet_len) as usize;
                code.push(CODE_ALPHABET[idx] as char);
            }
        }
        code
    }

    /// Request pairing for a sender on a channel.
    /// Returns `Some(code)` if a new code was issued, `None` if rate-limited.
    pub fn request_pairing(&self, channel: &str, sender_id: &str) -> Result<Option<String>> {
        self.cleanup_expired();

        // Check if already paired
        if self.is_paired(channel, sender_id) {
            return Ok(None);
        }

        // Check pending count per channel
        let pending_count = self.db.count_pending_for_channel(channel, CODE_TTL_SECS)?;
        if pending_count >= MAX_PENDING_PER_CHANNEL {
            debug!(
                "max pending pairing requests for channel {} reached",
                channel
            );
            return Ok(None);
        }

        // Check if this sender already has a pending request
        if let Some(existing) = self
            .db
            .get_pending_for_sender(channel, sender_id, CODE_TTL_SECS)?
        {
            return Ok(Some(existing.code));
        }

        let code = Self::generate_code();
        self.db
            .add_pending_request(channel, sender_id, &code, Self::now_secs())?;

        Ok(Some(code))
    }

    /// Approve a pairing request by code, with per-client lockout.
    /// `client_id` identifies the approver (e.g. CLI user, admin session).
    /// Returns `(channel, sender_id)` on success.
    ///
    /// SECURITY: Code comparison uses `subtle::ConstantTimeEq` in Rust,
    /// not SQL matching, to prevent timing side-channels.
    pub fn approve_with_client(
        &self,
        code: &str,
        client_id: &str,
    ) -> Result<Option<(String, String)>> {
        let now = Self::now_secs();

        // Evict oldest client entries if map is too large (DoS protection)
        self.db.evict_oldest_lockout_client(MAX_LOCKOUT_CLIENTS)?;

        // Per-client rate limiting
        let attempts = self
            .db
            .count_recent_failed_attempts(client_id, FAILED_ATTEMPT_WINDOW_SECS)?;
        if attempts >= MAX_FAILED_ATTEMPTS {
            anyhow::bail!("too many failed approval attempts, try again later");
        }

        // Fetch ALL non-expired pending requests and do constant-time compare in Rust
        let all_pending = self.db.get_all_pending(CODE_TTL_SECS)?;

        let code_upper = code.to_uppercase();

        // Also check expired codes for user-friendly feedback
        let all_including_expired = self.db.get_all_pending(u64::MAX)?;
        let has_expired_match = all_including_expired.iter().any(|r| {
            use subtle::ConstantTimeEq;
            let code_match: bool = r.code.as_bytes().ct_eq(code_upper.as_bytes()).into();
            code_match && now.saturating_sub(r.created_at) >= CODE_TTL_SECS
        });

        let matched = all_pending.iter().find(|r| {
            use subtle::ConstantTimeEq;
            r.code.as_bytes().ct_eq(code_upper.as_bytes()).into()
        });

        let Some(request) = matched else {
            if has_expired_match {
                warn!("pairing code matched but expired (TTL: {}s)", CODE_TTL_SECS);
            }
            self.db.record_failed_attempt(client_id, now)?;
            return Ok(None);
        };

        let channel = request.channel.clone();
        let sender_id = request.sender_id.clone();
        let matched_code = request.code.clone();

        // Remove the pending request and add to allowlist
        self.db.remove_pending(&matched_code)?;
        self.db.add_paired_sender(&channel, &sender_id)?;

        Ok(Some((channel, sender_id)))
    }

    /// Approve a pairing request by code. Returns `(channel, sender_id)` on success.
    /// Uses a default client ID for lockout tracking.
    pub fn approve(&self, code: &str) -> Result<Option<(String, String)>> {
        self.approve_with_client(code, "default")
    }

    /// Check if a sender is in the pairing store's allowlist for a channel.
    pub fn is_paired(&self, channel: &str, sender_id: &str) -> bool {
        self.db
            .is_sender_paired(channel, sender_id)
            .unwrap_or(false)
    }

    /// List all pending pairing requests (non-expired).
    pub fn list_pending(&self) -> Vec<PendingRequest> {
        self.db
            .get_all_pending(CODE_TTL_SECS)
            .unwrap_or_default()
            .into_iter()
            .map(|r| PendingRequest {
                channel: r.channel,
                sender_id: r.sender_id,
                code: r.code,
                created_at: r.created_at,
            })
            .collect()
    }

    /// List all paired senders across all channels.
    pub fn paired_count(&self) -> usize {
        self.db.count_paired_senders().unwrap_or(0)
    }

    /// Revoke a sender's pairing for a channel.
    pub fn revoke(&self, channel: &str, sender_id: &str) -> Result<bool> {
        self.db.remove_paired_sender(channel, sender_id)
    }

    /// Remove expired pending requests.
    pub fn cleanup_expired(&self) {
        if let Err(e) = self.db.cleanup_expired_pending(CODE_TTL_SECS) {
            warn!("failed to cleanup expired pending: {}", e);
        }
    }

    /// List all paired senders for a specific channel.
    pub fn list_channel_senders(&self, channel: &str) -> Option<Vec<String>> {
        match self.db.list_paired_senders(channel) {
            Ok(senders) if senders.is_empty() => None,
            Ok(senders) => Some(senders),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests;
