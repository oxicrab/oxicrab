use serde::{Deserialize, Serialize};
use std::time::Duration;

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
        store.inner.run_pending_tasks();
        let present = ["a", "b", "c"]
            .iter()
            .filter(|k| store.get(k).is_some())
            .count();
        assert!(
            present <= 2,
            "cache capacity is 2, but {} entries remained",
            present
        );
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
        store.inner.run_pending_tasks();
        assert!(store.get("btn").is_none());
    }

    #[test]
    #[should_panic(expected = "capacity must be > 0")]
    fn test_dispatch_context_store_zero_capacity_panics() {
        DispatchContextStore::new(0);
    }

    #[test]
    fn test_dispatch_context_store_ttl_cleans_entry() {
        let store = DispatchContextStore::with_ttl(100, std::time::Duration::from_millis(50));
        let p = ActionDispatchPayload {
            tool: "x".into(),
            params: serde_json::json!({}),
        };
        store.insert("btn".into(), p);
        std::thread::sleep(std::time::Duration::from_millis(60));
        store.inner.run_pending_tasks();
        assert!(store.get("btn").is_none());
        assert!(store.inner.get("btn").is_none());
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
