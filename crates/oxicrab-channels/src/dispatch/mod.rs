pub use oxicrab_core::dispatch::ActionDispatchPayload;
use std::time::Duration;

const DEFAULT_DISPATCH_TTL: Duration = Duration::from_mins(15);

/// In-memory LRU store for Discord button dispatch contexts.
/// Uses `moka` for bounded capacity and TTL eviction.
pub struct DispatchContextStore {
    inner: moka::sync::Cache<String, ActionDispatchPayload>,
}

impl DispatchContextStore {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "DispatchContextStore capacity must be > 0");
        Self::with_ttl(capacity, DEFAULT_DISPATCH_TTL)
    }

    pub fn with_ttl(capacity: usize, ttl: Duration) -> Self {
        assert!(capacity > 0, "DispatchContextStore capacity must be > 0");
        Self {
            inner: moka::sync::Cache::builder()
                .max_capacity(capacity as u64)
                .time_to_live(ttl)
                .build(),
        }
    }

    pub fn insert(&self, key: String, payload: ActionDispatchPayload) {
        self.inner.insert(key, payload);
    }

    pub fn get(&self, key: &str) -> Option<ActionDispatchPayload> {
        self.inner.get(key)
    }
}

#[cfg(test)]
mod tests;
