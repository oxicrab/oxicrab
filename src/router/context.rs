use serde::{Deserialize, Serialize};

pub const MAX_DIRECTIVES: usize = 20;
pub const DEFAULT_DIRECTIVE_TTL_MS: i64 = 300_000; // 5 minutes
const MAX_PATTERN_LEN: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouterContext {
    pub active_tool: Option<String>,
    pub action_directives: Vec<ActionDirective>,
    pub updated_at_ms: i64,
}

impl RouterContext {
    /// Load from session metadata. Returns default if missing or malformed.
    pub fn from_session_metadata(
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Self {
        metadata
            .get("router_context")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    /// Save to session metadata.
    pub fn to_session_metadata(
        &self,
        metadata: &mut std::collections::HashMap<String, serde_json::Value>,
    ) {
        if let Ok(v) = serde_json::to_value(self) {
            metadata.insert("router_context".to_string(), v);
        }
    }

    /// Set active tool. Clears directives on context switch (different tool).
    pub fn set_active_tool(&mut self, tool: Option<String>) {
        if tool != self.active_tool {
            self.action_directives.clear();
        }
        self.active_tool = tool;
    }

    /// Replace all directives. Caps at `MAX_DIRECTIVES`.
    /// Triggers are normalized (lowercased) at install time so `matches()` is cheaper.
    pub fn install_directives(&mut self, directives: Vec<ActionDirective>) {
        self.action_directives = directives
            .into_iter()
            .map(|mut d| {
                d.trigger = d.trigger.normalized();
                d
            })
            .collect();
        self.action_directives.truncate(MAX_DIRECTIVES);
    }

    /// Remove expired directives.
    pub fn prune_expired(&mut self, now_ms: i64) {
        self.action_directives.retain(|d| !d.is_expired(now_ms));
    }

    /// Remove directive at index (for single-use consumption).
    pub fn remove_directive_at(&mut self, index: usize) {
        if index < self.action_directives.len() {
            self.action_directives.remove(index);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionDirective {
    pub trigger: DirectiveTrigger,
    pub tool: String,
    pub params: serde_json::Value,
    pub single_use: bool,
    pub ttl_ms: i64,
    pub created_at_ms: i64,
}

impl ActionDirective {
    pub fn is_expired(&self, now_ms: i64) -> bool {
        now_ms > self.created_at_ms + self.ttl_ms
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DirectiveTrigger {
    /// Single literal — "next", "done". Hash lookup.
    Exact(String),
    /// Alternative literals — "yes|accept|ok". Linear scan over the Vec.
    OneOf(Vec<String>),
    /// Regex with captures. Compiled lazily. Rare.
    Pattern(String),
}

impl DirectiveTrigger {
    /// Pre-lowercase `Exact` and `OneOf` variants at construction time so
    /// `matches()` only needs to lowercase the incoming message, not the trigger.
    #[must_use]
    pub fn normalized(self) -> Self {
        match self {
            Self::Exact(s) => Self::Exact(s.to_lowercase()),
            Self::OneOf(options) => {
                Self::OneOf(options.into_iter().map(|o| o.to_lowercase()).collect())
            }
            // Patterns are applied to already-lowercased input; no change needed.
            Self::Pattern(_) => self,
        }
    }

    /// Case-insensitive whole-message match (trimmed).
    /// `Exact` and `OneOf` triggers are expected to already be lowercased
    /// (call `.normalized()` at construction time).
    pub fn matches(&self, message: &str) -> bool {
        let normalized = message.trim().to_lowercase();
        self.matches_normalized(&normalized)
    }

    /// Match against a pre-lowercased, pre-trimmed message.
    /// Use this when checking multiple triggers against the same message to
    /// avoid redundant `to_lowercase()` allocations.
    pub fn matches_normalized(&self, normalized: &str) -> bool {
        match self {
            Self::Exact(s) => normalized == *s,
            Self::OneOf(options) => options.iter().any(|o| o == normalized),
            Self::Pattern(pat) => {
                if pat.len() > MAX_PATTERN_LEN {
                    return false;
                }
                // Pattern compiled per-match (not cached). Acceptable because:
                // - Directives are short-lived (5 min TTL)
                // - Pattern triggers are rare (most use Exact/OneOf)
                // - 256-char length limit bounds compilation cost
                regex::Regex::new(pat).is_ok_and(|re| re.is_match(normalized))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::time::now_ms;

    #[test]
    fn test_router_context_default() {
        let ctx = RouterContext::default();
        assert!(ctx.active_tool.is_none());
        assert!(ctx.action_directives.is_empty());
    }

    #[test]
    fn test_router_context_serde_roundtrip() {
        let ctx = RouterContext {
            active_tool: Some("rss".into()),
            action_directives: vec![ActionDirective {
                trigger: DirectiveTrigger::Exact("next".into()),
                tool: "rss".into(),
                params: serde_json::json!({"action": "next"}),
                single_use: false,
                ttl_ms: 300_000,
                created_at_ms: now_ms(),
            }],
            updated_at_ms: now_ms(),
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let restored: RouterContext = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.active_tool, Some("rss".into()));
        assert_eq!(restored.action_directives.len(), 1);
    }

    #[test]
    fn test_directive_expired() {
        let d = ActionDirective {
            trigger: DirectiveTrigger::Exact("yes".into()),
            tool: "rss".into(),
            params: serde_json::json!({}),
            single_use: false,
            ttl_ms: 1,
            created_at_ms: now_ms() - 1000,
        };
        assert!(d.is_expired(now_ms()));
    }

    #[test]
    fn test_directive_not_expired() {
        let d = ActionDirective {
            trigger: DirectiveTrigger::Exact("yes".into()),
            tool: "rss".into(),
            params: serde_json::json!({}),
            single_use: false,
            ttl_ms: 300_000,
            created_at_ms: now_ms(),
        };
        assert!(!d.is_expired(now_ms()));
    }

    #[test]
    fn test_prune_expired() {
        let mut ctx = RouterContext {
            active_tool: Some("rss".into()),
            action_directives: vec![
                ActionDirective {
                    trigger: DirectiveTrigger::Exact("old".into()),
                    tool: "rss".into(),
                    params: serde_json::json!({}),
                    single_use: false,
                    ttl_ms: 1,
                    created_at_ms: now_ms() - 1000,
                },
                ActionDirective {
                    trigger: DirectiveTrigger::Exact("fresh".into()),
                    tool: "rss".into(),
                    params: serde_json::json!({}),
                    single_use: false,
                    ttl_ms: 300_000,
                    created_at_ms: now_ms(),
                },
            ],
            updated_at_ms: now_ms(),
        };
        ctx.prune_expired(now_ms());
        assert_eq!(ctx.action_directives.len(), 1);
        assert!(
            matches!(&ctx.action_directives[0].trigger, DirectiveTrigger::Exact(s) if s == "fresh")
        );
    }

    #[test]
    fn test_max_directives_cap() {
        let mut ctx = RouterContext::default();
        let directives: Vec<ActionDirective> = (0..25)
            .map(|i| ActionDirective {
                trigger: DirectiveTrigger::Exact(format!("d{i}")),
                tool: "t".into(),
                params: serde_json::json!({}),
                single_use: false,
                ttl_ms: 300_000,
                created_at_ms: now_ms(),
            })
            .collect();
        ctx.install_directives(directives);
        assert_eq!(ctx.action_directives.len(), MAX_DIRECTIVES);
    }

    #[test]
    fn test_context_switch_clears_directives() {
        let mut ctx = RouterContext {
            active_tool: Some("rss".into()),
            action_directives: vec![ActionDirective {
                trigger: DirectiveTrigger::Exact("yes".into()),
                tool: "rss".into(),
                params: serde_json::json!({}),
                single_use: false,
                ttl_ms: 300_000,
                created_at_ms: now_ms(),
            }],
            updated_at_ms: now_ms(),
        };
        ctx.set_active_tool(Some("google_calendar".into()));
        assert!(ctx.action_directives.is_empty());
        assert_eq!(ctx.active_tool, Some("google_calendar".into()));
    }

    #[test]
    fn test_same_tool_does_not_clear() {
        let mut ctx = RouterContext {
            active_tool: Some("rss".into()),
            action_directives: vec![ActionDirective {
                trigger: DirectiveTrigger::Exact("yes".into()),
                tool: "rss".into(),
                params: serde_json::json!({}),
                single_use: false,
                ttl_ms: 300_000,
                created_at_ms: now_ms(),
            }],
            updated_at_ms: now_ms(),
        };
        ctx.set_active_tool(Some("rss".into()));
        assert_eq!(ctx.action_directives.len(), 1);
    }

    #[test]
    fn test_directive_trigger_match_exact() {
        let t = DirectiveTrigger::Exact("next".into());
        assert!(t.matches("next"));
        assert!(t.matches("Next"));
        assert!(t.matches("  NEXT  "));
        assert!(!t.matches("next article"));
    }

    #[test]
    fn test_directive_trigger_match_one_of() {
        let t = DirectiveTrigger::OneOf(vec!["yes".into(), "accept".into(), "ok".into()]);
        assert!(t.matches("yes"));
        assert!(t.matches("Accept"));
        assert!(t.matches("OK"));
        assert!(!t.matches("yeah"));
    }

    #[test]
    fn test_directive_trigger_match_pattern() {
        let t = DirectiveTrigger::Pattern(r"^accept\s+(\S+)$".into());
        assert!(t.matches("accept abc123"));
        assert!(!t.matches("accept"));
        assert!(!t.matches("reject abc123"));
    }

    #[test]
    fn test_session_metadata_roundtrip() {
        let ctx = RouterContext {
            active_tool: Some("rss".into()),
            action_directives: vec![],
            updated_at_ms: 12345,
        };
        let mut metadata = std::collections::HashMap::new();
        ctx.to_session_metadata(&mut metadata);
        let restored = RouterContext::from_session_metadata(&metadata);
        assert_eq!(restored.active_tool, Some("rss".into()));
        assert_eq!(restored.updated_at_ms, 12345);
    }

    #[test]
    fn test_session_metadata_missing_returns_default() {
        let metadata = std::collections::HashMap::new();
        let ctx = RouterContext::from_session_metadata(&metadata);
        assert!(ctx.active_tool.is_none());
    }
}
