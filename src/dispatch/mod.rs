use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Structured tool call that bypasses the LLM.
#[derive(Debug, Clone)]
pub struct ActionDispatch {
    pub tool: String,
    pub params: serde_json::Value,
    pub source: ActionSource,
}

#[derive(Debug, Clone)]
pub enum ActionSource {
    Button { action_id: String },
    Webhook { webhook_name: String },
    Cron { job_id: String },
    Command { raw: String },
    ToolChain { parent_tool: String },
}

impl ActionSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Button { .. } => "button",
            Self::Webhook { .. } => "webhook",
            Self::Cron { .. } => "cron",
            Self::Command { .. } => "command",
            Self::ToolChain { .. } => "chain",
        }
    }
}

/// Serialized payload in ButtonSpec.context and webhook dispatch configs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionDispatchPayload {
    pub tool: String,
    pub params: serde_json::Value,
}

const DEFAULT_DISPATCH_TTL: Duration = Duration::from_mins(15);

struct DispatchStoreInner {
    entries: HashMap<String, (ActionDispatchPayload, Instant)>,
    order: VecDeque<String>,
}

/// In-memory LRU store for Discord button dispatch contexts.
/// Single mutex protects both map and insertion order for consistency.
pub struct DispatchContextStore {
    inner: Mutex<DispatchStoreInner>,
    capacity: usize,
    ttl: Duration,
}

impl DispatchContextStore {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "DispatchContextStore capacity must be > 0");
        Self {
            inner: Mutex::new(DispatchStoreInner {
                entries: HashMap::new(),
                order: VecDeque::new(),
            }),
            capacity,
            ttl: DEFAULT_DISPATCH_TTL,
        }
    }

    #[cfg(test)]
    pub fn with_ttl(capacity: usize, ttl: Duration) -> Self {
        assert!(capacity > 0, "DispatchContextStore capacity must be > 0");
        Self {
            inner: Mutex::new(DispatchStoreInner {
                entries: HashMap::new(),
                order: VecDeque::new(),
            }),
            capacity,
            ttl,
        }
    }

    pub fn insert(&self, key: String, payload: ActionDispatchPayload) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Handle duplicate key
        if inner.entries.contains_key(&key) {
            inner.order.retain(|k| k != &key);
        }
        // Evict oldest if at capacity
        while inner.entries.len() >= self.capacity {
            if let Some(oldest) = inner.order.pop_front() {
                inner.entries.remove(&oldest);
            } else {
                break;
            }
        }
        inner.entries.insert(key.clone(), (payload, Instant::now()));
        inner.order.push_back(key);
    }

    pub fn get(&self, key: &str) -> Option<ActionDispatchPayload> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((payload, inserted_at)) = inner.entries.get(key) {
            if inserted_at.elapsed() > self.ttl {
                inner.entries.remove(key);
                inner.order.retain(|k| k != key);
                return None;
            }
            Some(payload.clone())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payload_roundtrip() {
        let p = ActionDispatchPayload {
            tool: "rss".into(),
            params: serde_json::json!({"action": "accept", "article_ids": ["abc"]}),
        };
        let s = serde_json::to_string(&p).unwrap();
        let d: ActionDispatchPayload = serde_json::from_str(&s).unwrap();
        assert_eq!(d.tool, "rss");
        assert_eq!(d.params["action"], "accept");
    }

    #[test]
    fn test_payload_missing_params_fails() {
        let json = r#"{"tool": "rss"}"#;
        assert!(serde_json::from_str::<ActionDispatchPayload>(json).is_err());
    }

    #[test]
    fn test_source_label() {
        assert_eq!(
            ActionSource::Button {
                action_id: "x".into()
            }
            .label(),
            "button"
        );
        assert_eq!(
            ActionSource::Webhook {
                webhook_name: "x".into()
            }
            .label(),
            "webhook"
        );
        assert_eq!(ActionSource::Cron { job_id: "x".into() }.label(), "cron");
        assert_eq!(
            ActionSource::ToolChain {
                parent_tool: "x".into()
            }
            .label(),
            "chain"
        );
        assert_eq!(ActionSource::Command { raw: "x".into() }.label(), "command");
    }

    #[test]
    fn test_dispatch_context_store_insert_get() {
        let store = DispatchContextStore::new(100);
        let p = ActionDispatchPayload {
            tool: "rss".into(),
            params: serde_json::json!({}),
        };
        store.insert("btn-1".into(), p);
        assert!(store.get("btn-1").is_some());
        assert!(store.get("missing").is_none());
    }

    #[test]
    fn test_dispatch_context_store_eviction() {
        let store = DispatchContextStore::new(2);
        let p = |t: &str| ActionDispatchPayload {
            tool: t.into(),
            params: serde_json::json!({}),
        };
        store.insert("a".into(), p("a"));
        store.insert("b".into(), p("b"));
        store.insert("c".into(), p("c"));
        assert!(store.get("a").is_none());
        assert!(store.get("b").is_some());
        assert!(store.get("c").is_some());
    }

    #[test]
    fn test_dispatch_context_store_ttl() {
        let store = DispatchContextStore::with_ttl(100, std::time::Duration::from_millis(50));
        let p = ActionDispatchPayload {
            tool: "x".into(),
            params: serde_json::json!({}),
        };
        store.insert("btn".into(), p);
        assert!(store.get("btn").is_some());
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(store.get("btn").is_none());
    }

    #[test]
    #[should_panic(expected = "capacity must be > 0")]
    fn test_dispatch_context_store_zero_capacity_panics() {
        DispatchContextStore::new(0);
    }

    #[test]
    fn test_dispatch_context_store_ttl_cleans_order() {
        let store = DispatchContextStore::with_ttl(100, std::time::Duration::from_millis(50));
        let p = ActionDispatchPayload {
            tool: "x".into(),
            params: serde_json::json!({}),
        };
        store.insert("btn".into(), p);
        std::thread::sleep(std::time::Duration::from_millis(60));
        // TTL eviction in get() should also remove from order
        assert!(store.get("btn").is_none());
        let inner = store
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(!inner.order.iter().any(|k| k == "btn"));
    }

    #[test]
    fn test_dispatch_context_store_duplicate_key() {
        let store = DispatchContextStore::new(2);
        let p = |t: &str| ActionDispatchPayload {
            tool: t.into(),
            params: serde_json::json!({}),
        };
        store.insert("a".into(), p("v1"));
        store.insert("a".into(), p("v2"));
        assert_eq!(store.get("a").unwrap().tool, "v2");
    }
}
