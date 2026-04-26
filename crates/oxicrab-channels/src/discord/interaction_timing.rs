use std::time::{Duration, Instant};

/// Discord interaction tokens are valid for 15 minutes after Discord delivered
/// the interaction. Use a 14-minute safety margin so a request that takes a
/// long time to round-trip through the agent loop still has time to be
/// followed-up before Discord drops the token.
const INTERACTION_VALID_WINDOW: Duration = Duration::from_secs(14 * 60);

/// Keep entries in the cache slightly longer than the validity window so a
/// borderline check still finds the entry and can correctly classify it as
/// expired (rather than missing).
const STORE_TTL: Duration = Duration::from_secs(16 * 60);

const STORE_CAPACITY: u64 = 4096;

/// Per-process store mapping Discord interaction tokens to the monotonic
/// `Instant` at which the interaction arrived.
///
/// Discord interaction tokens expire 15 minutes after the gateway delivered
/// the interaction event. Previously the channel tracked the wall-clock
/// `SystemTime` at receipt and recomputed `now - then`. That math broke under
/// NTP correction, VM suspend/resume, or any wall-clock jump: a fresh
/// interaction could appear hours stale, and a stale interaction could
/// silently miss its expiry. This store uses `Instant`, which is monotonic
/// and unaffected by clock changes.
///
/// Entries auto-evict via moka TTL so dormant tokens do not accumulate.
pub struct InteractionTimingStore {
    inner: moka::sync::Cache<String, Instant>,
}

impl InteractionTimingStore {
    pub fn new() -> Self {
        Self {
            inner: moka::sync::Cache::builder()
                .max_capacity(STORE_CAPACITY)
                .time_to_live(STORE_TTL)
                .build(),
        }
    }

    /// Record the time an interaction was received.
    pub fn record(&self, token: &str) {
        self.inner.insert(token.to_string(), Instant::now());
    }

    /// Return true if the token is still within Discord's 15-minute followup
    /// window. Tokens that were never recorded (or were evicted) are treated
    /// as expired.
    pub fn is_valid(&self, token: &str) -> bool {
        self.inner
            .get(token)
            .is_some_and(|t| t.elapsed() < INTERACTION_VALID_WINDOW)
    }
}

impl Default for InteractionTimingStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_token_is_invalid() {
        let store = InteractionTimingStore::new();
        assert!(!store.is_valid("never-recorded"));
    }

    #[test]
    fn recorded_token_is_valid() {
        let store = InteractionTimingStore::new();
        store.record("tok-1");
        assert!(store.is_valid("tok-1"));
    }
}
