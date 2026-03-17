# Message Router Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace reactive LLM control systems (intent classification, 4-layer hallucination detection, tool filtering) with a proactive message router that fuses deterministic dispatch with guided LLM interpretation.

**Architecture:** New `src/router/` module contains a stateless `MessageRouter` that makes sub-100μs routing decisions. New `src/dispatch/` module contains shared types for structured payloads. The router sits at the top of `process_message_unlocked()`, checking sources in priority order: button payloads → action directives → config commands → static tool rules → remember fast path → guided LLM → semantic filter → full LLM. Old systems (intent classification, hallucination layers 0/2/3, tool category filtering) are deleted.

**Tech Stack:** Rust, serde_json, phf (compile-time hash sets), insta (snapshot testing). Existing: aho-corasick, regex, EmbeddingService.

**Spec:** `docs/superpowers/specs/2026-03-16-message-router-design.md`

---

## Chunk 1: Foundation Types

### Task 1: Create `src/dispatch/mod.rs` — shared dispatch types

**Files:**
- Create: `src/dispatch/mod.rs`
- Modify: `src/lib.rs:16-29`

- [ ] **Step 1: Write tests**

```rust
// src/dispatch/mod.rs — tests at bottom

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
        assert_eq!(ActionSource::Button { action_id: "x".into() }.label(), "button");
        assert_eq!(ActionSource::Webhook { webhook_name: "x".into() }.label(), "webhook");
        assert_eq!(ActionSource::Cron { job_id: "x".into() }.label(), "cron");
        assert_eq!(ActionSource::ToolChain { parent_tool: "x".into() }.label(), "chain");
        assert_eq!(ActionSource::Command { raw: "x".into() }.label(), "command");
    }

    #[test]
    fn test_dispatch_context_store_insert_get() {
        let store = DispatchContextStore::new(100);
        let p = ActionDispatchPayload { tool: "rss".into(), params: serde_json::json!({}) };
        store.insert("btn-1".into(), p);
        assert!(store.get("btn-1").is_some());
        assert!(store.get("missing").is_none());
    }

    #[test]
    fn test_dispatch_context_store_eviction() {
        let store = DispatchContextStore::new(2);
        let p = |t: &str| ActionDispatchPayload { tool: t.into(), params: serde_json::json!({}) };
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
        let p = ActionDispatchPayload { tool: "x".into(), params: serde_json::json!({}) };
        store.insert("btn".into(), p);
        assert!(store.get("btn").is_some());
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(store.get("btn").is_none());
    }

    #[test]
    fn test_dispatch_context_store_duplicate_key() {
        let store = DispatchContextStore::new(2);
        let p = |t: &str| ActionDispatchPayload { tool: t.into(), params: serde_json::json!({}) };
        store.insert("a".into(), p("v1"));
        store.insert("a".into(), p("v2"));
        assert_eq!(store.get("a").unwrap().tool, "v2");
    }
}
```

- [ ] **Step 2: Implement dispatch types**

```rust
// src/dispatch/mod.rs

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

const DEFAULT_DISPATCH_TTL: Duration = Duration::from_secs(15 * 60);

struct DispatchStoreInner {
    entries: HashMap<String, (ActionDispatchPayload, Instant)>,
    order: VecDeque<String>,
}

/// In-memory LRU store for Discord button dispatch contexts.
pub struct DispatchContextStore {
    inner: Mutex<DispatchStoreInner>,
    capacity: usize,
    ttl: Duration,
}

impl DispatchContextStore {
    pub fn new(capacity: usize) -> Self {
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
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.entries.contains_key(&key) {
            inner.order.retain(|k| k != &key);
        }
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
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((payload, inserted_at)) = inner.entries.get(key) {
            if inserted_at.elapsed() > self.ttl {
                inner.entries.remove(key);
                return None;
            }
            Some(payload.clone())
        } else {
            None
        }
    }
}
```

- [ ] **Step 3: Add `pub mod dispatch;` to `src/lib.rs`**

Add after `pub mod config;` (around line 22).

- [ ] **Step 4: Run tests**

Run: `cargo test --lib dispatch::tests -- --test-threads=1`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/dispatch/mod.rs src/lib.rs
git commit -m "feat(dispatch): add shared dispatch types

ActionDispatch, ActionSource, ActionDispatchPayload, DispatchContextStore."
```

---

### Task 2: Create `src/router/context.rs` — RouterContext and directives

**Files:**
- Create: `src/router/context.rs`
- Create: `src/router/mod.rs` (stub for module declaration)
- Modify: `src/lib.rs`

- [ ] **Step 1: Write tests**

```rust
// src/router/context.rs — tests at bottom

#[cfg(test)]
mod tests {
    use super::*;

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0))
    }

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
            ttl_ms: 1, // 1ms TTL
            created_at_ms: now_ms() - 1000, // created 1s ago
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
        assert!(matches!(&ctx.action_directives[0].trigger, DirectiveTrigger::Exact(s) if s == "fresh"));
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
        assert!(t.matches("Next")); // case insensitive
        assert!(t.matches("  NEXT  ")); // trimmed
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
}
```

- [ ] **Step 2: Implement RouterContext**

```rust
// src/router/context.rs

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

    pub fn set_active_tool(&mut self, tool: Option<String>) {
        if tool != self.active_tool {
            self.action_directives.clear();
        }
        self.active_tool = tool;
    }

    pub fn install_directives(&mut self, directives: Vec<ActionDirective>) {
        self.action_directives = directives;
        self.action_directives.truncate(MAX_DIRECTIVES);
    }

    pub fn prune_expired(&mut self, now_ms: i64) {
        self.action_directives.retain(|d| !d.is_expired(now_ms));
    }

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
    Exact(String),
    OneOf(Vec<String>),
    Pattern(String),
}

impl DirectiveTrigger {
    /// Case-insensitive whole-message match (trimmed).
    pub fn matches(&self, message: &str) -> bool {
        let normalized = message.trim().to_lowercase();
        match self {
            Self::Exact(s) => normalized == s.to_lowercase(),
            Self::OneOf(options) => options.iter().any(|o| normalized == o.to_lowercase()),
            Self::Pattern(pat) => {
                if pat.len() > MAX_PATTERN_LEN {
                    return false;
                }
                regex::Regex::new(pat)
                    .map(|re| re.is_match(&normalized))
                    .unwrap_or(false)
            }
        }
    }
}
```

- [ ] **Step 3: Create stub `src/router/mod.rs`**

```rust
// src/router/mod.rs
pub mod context;
```

- [ ] **Step 4: Add `pub mod router;` to `src/lib.rs`**

- [ ] **Step 5: Run tests**

Run: `cargo test --lib router::context::tests -- --test-threads=1`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/router/ src/lib.rs
git commit -m "feat(router): add RouterContext with directive matching

Per-session state with active_tool, action directives, TTL expiry,
context switch clearing, and case-insensitive whole-message matching."
```

---

### Task 3: Create `src/router/rules.rs` — static and config rules

**Files:**
- Create: `src/router/rules.rs`
- Modify: `src/router/mod.rs`

- [ ] **Step 1: Write tests**

```rust
// src/router/rules.rs — tests at bottom

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_rule_substitute_positional() {
        let rule = ConfigRule {
            trigger: "weather".into(),
            tool: "weather".into(),
            params: serde_json::json!({"location": "$1"}),
        };
        let result = rule.substitute(&["portland"]);
        assert_eq!(result["location"], "portland");
    }

    #[test]
    fn test_config_rule_substitute_remainder() {
        let rule = ConfigRule {
            trigger: "note".into(),
            tool: "memory".into(),
            params: serde_json::json!({"content": "$*"}),
        };
        let result = rule.substitute(&["buy", "milk", "tomorrow"]);
        assert_eq!(result["content"], "buy milk tomorrow");
    }

    #[test]
    fn test_config_rule_missing_arg() {
        let rule = ConfigRule {
            trigger: "weather".into(),
            tool: "weather".into(),
            params: serde_json::json!({"location": "$1", "units": "$2"}),
        };
        let result = rule.substitute(&["portland"]);
        assert_eq!(result["location"], "portland");
        assert_eq!(result["units"], "");
    }

    #[test]
    fn test_parse_prefixed_command() {
        let (cmd, args) = parse_prefixed_command("!weather portland oregon", "!");
        assert_eq!(cmd, "weather");
        assert_eq!(args, vec!["portland", "oregon"]);
    }

    #[test]
    fn test_parse_prefixed_command_no_args() {
        let (cmd, args) = parse_prefixed_command("!todo", "!");
        assert_eq!(cmd, "todo");
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_prefixed_not_prefixed() {
        let (cmd, _) = parse_prefixed_command("hello world", "!");
        assert_eq!(cmd, "");
    }

    #[test]
    fn test_static_rule_matches_with_context() {
        let rule = StaticRule {
            tool: "rss".into(),
            trigger: super::super::context::DirectiveTrigger::Exact("next".into()),
            params: serde_json::json!({"action": "next"}),
            requires_context: true,
        };
        assert!(rule.matches("next", Some("rss")));
        assert!(!rule.matches("next", Some("cron")));
        assert!(!rule.matches("next", None));
    }

    #[test]
    fn test_static_rule_matches_without_context() {
        let rule = StaticRule {
            tool: "cron".into(),
            trigger: super::super::context::DirectiveTrigger::Exact("list jobs".into()),
            params: serde_json::json!({"action": "list"}),
            requires_context: false,
        };
        assert!(rule.matches("list jobs", None));
        assert!(rule.matches("list jobs", Some("rss")));
    }
}
```

- [ ] **Step 2: Implement rules**

```rust
// src/router/rules.rs

use serde::{Deserialize, Serialize};
use super::context::DirectiveTrigger;

/// Tool-declared static routing rule. Compiled at startup.
pub struct StaticRule {
    pub tool: String,
    pub trigger: DirectiveTrigger,
    pub params: serde_json::Value,
    pub requires_context: bool,
}

impl StaticRule {
    pub fn matches(&self, message: &str, active_tool: Option<&str>) -> bool {
        if self.requires_context {
            if active_tool != Some(self.tool.as_str()) {
                return false;
            }
        }
        self.trigger.matches(message)
    }
}

/// User-defined prefix command from config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigRule {
    pub trigger: String,
    pub tool: String,
    pub params: serde_json::Value,
}

impl ConfigRule {
    /// Substitute $1, $2, ... and $* in params with positional args.
    pub fn substitute(&self, args: &[&str]) -> serde_json::Value {
        let template = serde_json::to_string(&self.params).unwrap_or_default();
        let mut result = template.clone();

        // $* = entire remainder
        let remainder = args.join(" ");
        result = result.replace("$*", &remainder);

        // $1, $2, ... positional
        for (i, arg) in args.iter().enumerate() {
            result = result.replace(&format!("${}", i + 1), arg);
        }

        // Clean up unmatched $N references
        let re = regex::Regex::new(r"\$\d+").unwrap();
        result = re.replace_all(&result, "").to_string();

        serde_json::from_str(&result).unwrap_or(self.params.clone())
    }
}

/// Parse a prefixed command. Returns (command_word, args).
/// If message doesn't start with prefix, returns ("", vec![]).
pub fn parse_prefixed_command<'a>(message: &'a str, prefix: &str) -> (&'a str, Vec<&'a str>) {
    let trimmed = message.trim();
    if !trimmed.starts_with(prefix) {
        return ("", vec![]);
    }
    let without_prefix = &trimmed[prefix.len()..];
    let mut parts = without_prefix.split_whitespace();
    let command = parts.next().unwrap_or("");
    let args: Vec<&str> = parts.collect();
    (command, args)
}
```

- [ ] **Step 3: Add `pub mod rules;` to `src/router/mod.rs`**

- [ ] **Step 4: Run tests**

Run: `cargo test --lib router::rules::tests -- --test-threads=1`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/router/rules.rs src/router/mod.rs
git commit -m "feat(router): add static and config rule types

StaticRule for tool-declared shortcuts, ConfigRule for user-defined
prefix commands with \$N positional substitution."
```

---

### Task 4: Implement `MessageRouter` and `route()` logic

**Files:**
- Modify: `src/router/mod.rs`

- [ ] **Step 1: Write tests**

```rust
// src/router/mod.rs — tests at bottom

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::{ActionDispatch, ActionSource};

    fn make_router() -> MessageRouter {
        let static_rules = vec![
            rules::StaticRule {
                tool: "rss".into(),
                trigger: context::DirectiveTrigger::OneOf(vec!["next".into(), "more".into()]),
                params: serde_json::json!({"action": "next"}),
                requires_context: true,
            },
            rules::StaticRule {
                tool: "cron".into(),
                trigger: context::DirectiveTrigger::Exact("list jobs".into()),
                params: serde_json::json!({"action": "list"}),
                requires_context: false,
            },
        ];
        let mut config_rules = std::collections::HashMap::new();
        config_rules.insert("weather".into(), rules::ConfigRule {
            trigger: "weather".into(),
            tool: "weather".into(),
            params: serde_json::json!({"location": "$1"}),
        });
        MessageRouter::new(static_rules, config_rules, "!".into())
    }

    #[test]
    fn test_route_action_dispatch() {
        let router = make_router();
        let ctx = context::RouterContext::default();
        let dispatch = ActionDispatch {
            tool: "rss".into(),
            params: serde_json::json!({"action": "accept"}),
            source: ActionSource::Button { action_id: "btn".into() },
        };
        let decision = router.route("ignored", &ctx, Some(&dispatch));
        assert!(matches!(decision, RoutingDecision::DirectDispatch { source: DispatchSource::Button, .. }));
    }

    #[test]
    fn test_route_directive_match() {
        let router = make_router();
        let mut ctx = context::RouterContext::default();
        ctx.active_tool = Some("rss".into());
        ctx.action_directives.push(context::ActionDirective {
            trigger: context::DirectiveTrigger::OneOf(vec!["yes".into(), "accept".into()]),
            tool: "rss".into(),
            params: serde_json::json!({"action": "accept", "article_ids": ["abc"]}),
            single_use: true,
            ttl_ms: 300_000,
            created_at_ms: now_ms(),
        });
        let decision = router.route("yes", &ctx, None);
        assert!(matches!(decision, RoutingDecision::DirectDispatch { source: DispatchSource::ActionDirective, .. }));
    }

    #[test]
    fn test_route_config_command() {
        let router = make_router();
        let ctx = context::RouterContext::default();
        let decision = router.route("!weather portland", &ctx, None);
        match decision {
            RoutingDecision::DirectDispatch { tool, params, source: DispatchSource::ConfigRule } => {
                assert_eq!(tool, "weather");
                assert_eq!(params["location"], "portland");
            }
            _ => panic!("expected DirectDispatch"),
        }
    }

    #[test]
    fn test_route_static_rule_with_context() {
        let router = make_router();
        let mut ctx = context::RouterContext::default();
        ctx.active_tool = Some("rss".into());
        let decision = router.route("next", &ctx, None);
        assert!(matches!(decision, RoutingDecision::DirectDispatch { source: DispatchSource::StaticRule, .. }));
    }

    #[test]
    fn test_route_static_rule_wrong_context() {
        let router = make_router();
        let mut ctx = context::RouterContext::default();
        ctx.active_tool = Some("cron".into());
        let decision = router.route("next", &ctx, None);
        // "next" requires rss context, should not match
        assert!(!matches!(decision, RoutingDecision::DirectDispatch { source: DispatchSource::StaticRule, .. }));
    }

    #[test]
    fn test_route_static_rule_no_context_required() {
        let router = make_router();
        let ctx = context::RouterContext::default();
        let decision = router.route("list jobs", &ctx, None);
        assert!(matches!(decision, RoutingDecision::DirectDispatch { source: DispatchSource::StaticRule, .. }));
    }

    #[test]
    fn test_route_guided_llm_active_context_no_match() {
        let router = make_router();
        let mut ctx = context::RouterContext::default();
        ctx.active_tool = Some("rss".into());
        let decision = router.route("show me something interesting", &ctx, None);
        assert!(matches!(decision, RoutingDecision::GuidedLLM { .. }));
    }

    #[test]
    fn test_route_full_llm_no_context() {
        let router = make_router();
        let ctx = context::RouterContext::default();
        let decision = router.route("hello how are you", &ctx, None);
        assert!(matches!(decision, RoutingDecision::FullLLM));
    }

    #[test]
    fn test_route_remember_fast_path() {
        let router = make_router();
        let ctx = context::RouterContext::default();
        let decision = router.route("remember that my favorite color is blue", &ctx, None);
        assert!(matches!(decision, RoutingDecision::DirectDispatch { source: DispatchSource::RememberFastPath, .. }));
    }

    #[test]
    fn test_route_empty_message() {
        let router = make_router();
        let ctx = context::RouterContext::default();
        let decision = router.route("", &ctx, None);
        assert!(matches!(decision, RoutingDecision::FullLLM));
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0))
    }
}
```

- [ ] **Step 2: Implement MessageRouter**

```rust
// src/router/mod.rs

pub mod context;
pub mod rules;

use crate::dispatch::{ActionDispatch, ActionSource};
use context::RouterContext;
use rules::{ConfigRule, StaticRule};
use std::collections::HashMap;
use tracing::{debug, info};

pub enum RoutingDecision {
    DirectDispatch {
        tool: String,
        params: serde_json::Value,
        source: DispatchSource,
    },
    GuidedLLM {
        tool_subset: Vec<String>,
        context_hint: String,
    },
    SemanticFilter {
        tool_subset: Vec<String>,
    },
    FullLLM,
}

pub enum DispatchSource {
    Button,
    ActionDirective,
    StaticRule,
    ConfigRule,
    RememberFastPath,
    Webhook,
}

pub struct MessageRouter {
    static_rules: Vec<StaticRule>,
    config_rules: HashMap<String, ConfigRule>,
    prefix: String,
}

impl MessageRouter {
    pub fn new(
        static_rules: Vec<StaticRule>,
        config_rules: HashMap<String, ConfigRule>,
        prefix: String,
    ) -> Self {
        Self {
            static_rules,
            config_rules,
            prefix,
        }
    }

    /// Make a routing decision. Pure function — no IO, no side effects.
    /// `action` is from InboundMessage.action (buttons, webhooks).
    pub fn route(
        &self,
        message: &str,
        ctx: &RouterContext,
        action: Option<&ActionDispatch>,
    ) -> RoutingDecision {
        // Priority 1: Structured action dispatch (buttons, webhooks)
        if let Some(dispatch) = action {
            let source = match &dispatch.source {
                ActionSource::Button { .. } => DispatchSource::Button,
                ActionSource::Webhook { .. } => DispatchSource::Webhook,
                _ => DispatchSource::Button,
            };
            return RoutingDecision::DirectDispatch {
                tool: dispatch.tool.clone(),
                params: dispatch.params.clone(),
                source,
            };
        }

        let trimmed = message.trim();
        if trimmed.is_empty() {
            return RoutingDecision::FullLLM;
        }

        let now = now_ms();

        // Priority 2: Action directives from RouterContext
        for (i, directive) in ctx.action_directives.iter().enumerate() {
            if directive.is_expired(now) {
                continue;
            }
            if directive.trigger.matches(trimmed) {
                info!(
                    "router: decision=DirectDispatch tool={} source=directive",
                    directive.tool
                );
                return RoutingDecision::DirectDispatch {
                    tool: directive.tool.clone(),
                    params: directive.params.clone(),
                    source: DispatchSource::ActionDirective,
                };
            }
        }

        // Priority 3: Prefixed config rules
        if trimmed.starts_with(&self.prefix) {
            let (cmd, args) = rules::parse_prefixed_command(trimmed, &self.prefix);
            if let Some(rule) = self.config_rules.get(cmd) {
                let args_refs: Vec<&str> = args.iter().copied().collect();
                let params = rule.substitute(&args_refs);
                info!(
                    "router: decision=DirectDispatch tool={} source=config_rule",
                    rule.tool
                );
                return RoutingDecision::DirectDispatch {
                    tool: rule.tool.clone(),
                    params,
                    source: DispatchSource::ConfigRule,
                };
            }
        }

        // Priority 4: Static tool rules
        let active = ctx.active_tool.as_deref();
        for rule in &self.static_rules {
            if rule.matches(trimmed, active) {
                info!(
                    "router: decision=DirectDispatch tool={} source=static_rule",
                    rule.tool
                );
                return RoutingDecision::DirectDispatch {
                    tool: rule.tool.clone(),
                    params: rule.params.clone(),
                    source: DispatchSource::StaticRule,
                };
            }
        }

        // Priority 5: Remember fast path
        if crate::agent::memory::remember::extract_remember_content(trimmed).is_some() {
            info!("router: decision=DirectDispatch source=remember");
            return RoutingDecision::DirectDispatch {
                tool: "_remember".into(),
                params: serde_json::json!({"content": trimmed}),
                source: DispatchSource::RememberFastPath,
            };
        }

        // Priority 6: Active tool context → GuidedLLM
        if let Some(tool) = &ctx.active_tool {
            let hint = build_context_hint(ctx);
            info!(
                "router: decision=GuidedLLM tool_subset=[{}]",
                tool
            );
            return RoutingDecision::GuidedLLM {
                tool_subset: vec![tool.clone()],
                context_hint: hint,
            };
        }

        // Priority 7/8: SemanticFilter or FullLLM
        // SemanticFilter is applied by the agent loop (requires EmbeddingService),
        // not by the router (which is IO-free). Router returns FullLLM here.
        debug!("router: decision=FullLLM");
        RoutingDecision::FullLLM
    }

    /// Index of the matched directive (for single_use removal).
    pub fn matched_directive_index(
        &self,
        message: &str,
        ctx: &RouterContext,
    ) -> Option<usize> {
        let trimmed = message.trim();
        let now = now_ms();
        for (i, directive) in ctx.action_directives.iter().enumerate() {
            if !directive.is_expired(now) && directive.trigger.matches(trimmed) {
                return Some(i);
            }
        }
        None
    }
}

fn build_context_hint(ctx: &RouterContext) -> String {
    let tool = ctx.active_tool.as_deref().unwrap_or("unknown");
    let actions: Vec<String> = ctx
        .action_directives
        .iter()
        .filter_map(|d| match &d.trigger {
            context::DirectiveTrigger::Exact(s) => Some(s.clone()),
            context::DirectiveTrigger::OneOf(v) => Some(v.join("/")),
            context::DirectiveTrigger::Pattern(_) => None,
        })
        .collect();
    let actions_str = if actions.is_empty() {
        String::new()
    } else {
        format!(" Available shortcuts: {}", actions.join(", "))
    };
    format!(
        "The user is interacting with the {} tool.{}",
        tool, actions_str
    )
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0))
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib router::tests -- --test-threads=1`
Expected: PASS

- [ ] **Step 4: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add src/router/
git commit -m "feat(router): implement MessageRouter with priority-based routing

route() checks: action payloads → directives → config commands →
static rules → remember → guided LLM → full LLM. Sub-100μs decisions."
```

---

### Task 5: Add config schema for router

**Files:**
- Create: `src/config/schema/router.rs`
- Modify: `src/config/schema/mod.rs` (add `mod router; pub use router::*;`)
- Modify: top-level `Config` struct to include `router: Option<RouterConfig>`

- [ ] **Step 1: Implement RouterConfig**

```rust
// src/config/schema/router.rs

use serde::{Deserialize, Serialize};

fn default_prefix() -> String {
    "!".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default)]
    pub rules: Vec<ConfigRuleConfig>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            prefix: default_prefix(),
            rules: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigRuleConfig {
    pub trigger: String,
    pub tool: String,
    pub params: serde_json::Value,
}
```

- [ ] **Step 2: Add to Config struct**

Find the top-level `Config` struct in `src/config/schema/mod.rs`. Add:

```rust
    #[serde(default)]
    pub router: RouterConfig,
```

Update `Default` impl if manual, or verify `#[serde(default)]` handles it.

- [ ] **Step 3: Run tests** (verify config example test passes)

Run: `cargo test --lib test_config -- --test-threads=1`

If the config example test (`test_config_example_is_up_to_date`) fails, update `config.example.json` to include the `router` field with default values. Also update `credential_overlays()` in `src/config/schema/tests.rs` if needed.

- [ ] **Step 4: Commit**

```bash
git add src/config/schema/router.rs src/config/schema/mod.rs
git commit -m "feat(config): add router configuration schema

RouterConfig with prefix (default '!') and user-defined command rules."
```

---

## Chunk 2: Tool Trait Extensions and Bus Changes

### Task 6: Extend Tool trait with `routing_rules()` and `usage_examples()`

**Files:**
- Modify: `src/agent/tools/base/mod.rs:68-109` (Tool trait)

- [ ] **Step 1: Add new types and trait methods**

Add `StaticRule` import and new types near the top of the file:

```rust
pub struct ToolExample {
    pub user_request: String,
    pub params: serde_json::Value,
}
```

Add to the `Tool` trait (after `capabilities()`):

```rust
    /// Static routing rules for deterministic dispatch. Called once at registration.
    fn routing_rules(&self) -> Vec<crate::router::rules::StaticRule> {
        Vec::new()
    }

    /// Usage examples appended to tool description for LLM schema.
    fn usage_examples(&self) -> Vec<ToolExample> {
        Vec::new()
    }
```

- [ ] **Step 2: Update `to_schema()` to include examples in description**

In the default `to_schema()` implementation, after building the description, append examples:

```rust
    fn to_schema(&self) -> Value {
        let examples = self.usage_examples();
        let description = if examples.is_empty() {
            self.description().to_string()
        } else {
            let mut desc = self.description().to_string();
            desc.push_str("\n\nExamples:");
            for ex in &examples {
                desc.push_str(&format!(
                    "\n- \"{}\" → {}",
                    ex.user_request,
                    serde_json::to_string(&ex.params).unwrap_or_default()
                ));
            }
            desc
        };
        // ... rest of schema building using `description` instead of `self.description()`
    }
```

Note: Check the existing `to_schema()` implementation to understand how description is currently used, and modify accordingly.

- [ ] **Step 3: Run tests**

Run: `cargo test --lib -- --test-threads=1`
Expected: PASS (default impls return empty vecs, no behavior change)

- [ ] **Step 4: Commit**

```bash
git add src/agent/tools/base/mod.rs
git commit -m "feat(tools): add routing_rules() and usage_examples() to Tool trait

Default impls return empty vecs. routing_rules() collected at
registration for router. usage_examples() appended to schema
descriptions for LLM accuracy improvement."
```

---

### Task 7: Add `action` field to InboundMessage and AgentRunOverrides

**Files:**
- Modify: `src/bus/events/mod.rs:36-45` (InboundMessage)
- Modify: `src/bus/events/mod.rs:74-111` (InboundMessageBuilder)
- Modify: `src/agent/loop/config.rs:11-26` (AgentRunOverrides)

- [ ] **Step 1: Add fields and builder method**

In `InboundMessage` struct, add after `metadata` field:

```rust
    #[serde(skip)]
    pub action: Option<crate::dispatch::ActionDispatch>,
```

In `InboundMessageBuilder`, add method:

```rust
    pub fn action(mut self, dispatch: crate::dispatch::ActionDispatch) -> Self {
        self.inner.action = Some(dispatch);
        self
    }
```

In `AgentRunOverrides`, add field:

```rust
    pub action: Option<crate::dispatch::ActionDispatch>,
```

Search all `AgentRunOverrides` construction sites and add `action: None`:

Run: `grep -rn "AgentRunOverrides" src/ tests/ --include="*.rs"`

- [ ] **Step 2: Run full tests**

Run: `cargo test -- --test-threads=1`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/bus/events/mod.rs src/agent/loop/config.rs
git commit -m "feat(dispatch): add action field to InboundMessage and AgentRunOverrides"
```

---

### Task 8: Collect routing_rules in ToolRegistry

**Files:**
- Modify: `src/agent/tools/registry/mod.rs`

- [ ] **Step 1: Add routing rules collection**

Add a field to `ToolRegistry`:

```rust
    routing_rules: Vec<crate::router::rules::StaticRule>,
```

In `register()` (or wherever tools are added), after inserting the tool, collect rules:

```rust
    let rules = tool.routing_rules();
    self.routing_rules.extend(rules);
```

Add a public accessor:

```rust
    pub fn routing_rules(&self) -> &[crate::router::rules::StaticRule] {
        &self.routing_rules
    }
```

Initialize the field in `new()` and `with_stash()`.

- [ ] **Step 2: Run tests**

Run: `cargo test --lib -- --test-threads=1`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/agent/tools/registry/mod.rs
git commit -m "feat(registry): collect tool routing_rules at registration"
```

---

## Chunk 3: Agent Loop Overhaul

### Task 9: Delete intent module and tool_filter module

**Files:**
- Delete: `src/agent/loop/intent/` (entire directory)
- Delete: `src/agent/loop/tool_filter.rs`
- Modify: `src/agent/loop/mod.rs:1-9` (remove mod declarations)

- [ ] **Step 1: Remove `mod intent;` and `mod tool_filter;` from `src/agent/loop/mod.rs`**

- [ ] **Step 2: Delete the files/directories**

```bash
rm -rf src/agent/loop/intent/
rm src/agent/loop/tool_filter.rs
```

- [ ] **Step 3: Fix compilation errors**

Search for all references to the deleted modules:

Run: `cargo build 2>&1 | head -50`

Fix all compilation errors. Key references to remove:
- `use super::intent;` in hallucination.rs
- `infer_tool_categories()` calls in iteration.rs
- `TOOL_FILTER_THRESHOLD` references in iteration.rs
- `classify_and_record_intent()` in processing.rs
- `classify_action_intent()` calls anywhere
- Any `record_intent_event()` calls
- `user_has_action_intent` parameter from `run_agent_loop_with_overrides()` and all callers
- `get_intent_stats()` and `get_recent_hallucinations()` in stats CLI command

For each reference, either remove the code entirely or stub it temporarily (e.g., replace `user_has_action_intent` with `false` in hallucination calls until those layers are also removed in the next task).

- [ ] **Step 4: Remove `stats intent` CLI subcommand**

Find the stats command handler and remove the `intent` subcommand. Remove associated DB query methods (`get_intent_stats`, `get_recent_hallucinations`).

- [ ] **Step 5: Run tests**

Run: `cargo test -- --test-threads=1`
Expected: PASS (some intent-specific tests will be gone with the module)

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: remove intent classification and tool filter modules

Replaced by the message router. Removes regex + semantic intent
classification, tool category filtering, and stats intent CLI command."
```

---

### Task 10: Gut hallucination to Layer 1 only

**Files:**
- Modify: `src/agent/loop/hallucination.rs`
- Modify: `src/agent/loop/hallucination/helpers.rs` (if separate file)

- [ ] **Step 1: Simplify `handle_text_response()`**

Replace the entire function with a simplified version that only checks Layer 1 (action claim detection):

```rust
pub(super) fn handle_text_response(
    content: &str,
    messages: &mut Vec<Message>,
    any_tools_called: bool,
    layer1_fired: &mut bool,
    tool_names: &[String],
    tools_used: &[String],
) -> TextAction {
    // Layer 1 only: action claims without tool calls.
    // Single retry — if LLM claims actions without calling tools, inject correction.
    if !*layer1_fired && !any_tools_called && !tool_names.is_empty() {
        if contains_action_claims(content) {
            warn!("hallucination layer 1: action claims detected without tool calls");
            *layer1_fired = true;

            ContextBuilder::add_tool_result(
                messages,
                "hallucination-check",
                "system",
                "You claimed to perform actions but did not call any tools. \
                 Please use the available tools to perform the requested actions.",
                true,
            );
            return TextAction::Continue;
        }
    }

    TextAction::Return
}
```

- [ ] **Step 2: Remove everything else**

Remove:
- `CorrectionState` struct and impl
- `MAX_LAYER0_CORRECTIONS`
- `is_false_no_tools_claim()` function
- `is_legitimate_refusal()` function
- `mentions_multiple_tools()` function
- `mentions_any_tool()` function
- Layer 0, 2, 3 code blocks
- `use super::intent;`
- All `record_intent_event()` calls
- Parameters: `reasoning_content`, `user_has_action_intent`, `db`, `request_id`, `tool_mention_ac`

Keep:
- `TextAction` enum
- `contains_action_claims()` (in helpers.rs)
- The action claim regex patterns

- [ ] **Step 3: Update callers in `iteration.rs`**

In `iteration.rs`, update the `handle_text_response()` call site (around line 296) to match the new simplified signature:

```rust
match hallucination::handle_text_response(
    &content,
    &mut messages,
    any_tools_called,
    &mut layer1_fired,
    &tool_names,
    &tools_used,
) {
```

Replace `CorrectionState::new()` with `let mut layer1_fired = false;`.

Remove `tool_mention_ac` construction (the AC automaton built from tool names, around line 135).

- [ ] **Step 4: Remove anti-hallucination system prompt injection**

In `iteration.rs`, remove the block (around lines 128-133):

```rust
    if !tool_names.is_empty()
        && let Some(system_msg) = messages.first_mut()
    {
        system_msg.content.push_str(
            "\n\nYou have tools available. If a user asks for external actions, \
             do not claim tools are unavailable — call the matching tool directly.",
        );
    }
```

- [ ] **Step 5: Remove tool category filtering from iteration.rs**

Remove the `cached_categories` block (around lines 52-81) and the `infer_tool_categories` import. Replace with:

```rust
    let tools_defs = self
        .tools
        .get_tool_definitions_with_activated(&activated_snapshot);
```

The `GuidedLLM` path will handle tool filtering via `tool_subset` later.

- [ ] **Step 6: Run tests**

Run: `cargo test -- --test-threads=1`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor: gut hallucination detection to Layer 1 only

Remove layers 0/2/3, CorrectionState, intent dependency, anti-hallucination
prompt injection, tool category filtering, and tool mention AC automaton.
Keep only action claim regex as lightweight safety net."
```

---

### Task 11: Integrate router into processing.rs

**Files:**
- Modify: `src/agent/loop/processing.rs`
- Modify: `src/agent/loop/mod.rs` (add `router` field to AgentLoop)

- [ ] **Step 1: Add `router` field to AgentLoop**

In `src/agent/loop/mod.rs`, add to the `AgentLoop` struct:

```rust
    router: std::sync::Arc<crate::router::MessageRouter>,
```

Find where `AgentLoop` is constructed (in `AgentLoop::new()` or equivalent) and build the router from tool registry rules + config:

```rust
    let static_rules: Vec<_> = tools.routing_rules().to_vec();
    // Convert config rules
    let config_rules: HashMap<String, crate::router::rules::ConfigRule> = config
        .router
        .rules
        .iter()
        .map(|r| (r.trigger.clone(), crate::router::rules::ConfigRule {
            trigger: r.trigger.clone(),
            tool: r.tool.clone(),
            params: r.params.clone(),
        }))
        .collect();
    let router = std::sync::Arc::new(crate::router::MessageRouter::new(
        static_rules,
        config_rules,
        config.router.prefix.clone(),
    ));
```

Note: The exact location depends on how `AgentLoop` is constructed. Search for `AgentLoop {` to find it. The router config comes from the top-level `Config.router` field added in Task 5.

Also update `tests/common/mod.rs` `create_test_agent_with()` to include the router field.

- [ ] **Step 2: Rewrite the top of `process_message_unlocked()`**

After the system message check and typing indicator, but BEFORE the existing secret scanning / prompt guard / remember check, add the router integration. The key change: **move session loading earlier** (before routing) and add the router call:

```rust
    // Load session early — router needs RouterContext
    let session_key = msg.session_key();
    let session = self.sessions.get_or_create(&session_key).await?;
    let mut router_context = crate::router::context::RouterContext::from_session_metadata(&session.metadata);

    // Router decides
    let decision = self.router.route(
        &msg.content,
        &router_context,
        msg.action.as_ref(),
    );

    match decision {
        crate::router::RoutingDecision::DirectDispatch { tool, params, source } => {
            return self.handle_direct_dispatch(
                tool, params, source, &msg, &session_key, &mut router_context,
            ).await;
        }
        crate::router::RoutingDecision::GuidedLLM { tool_subset, context_hint } => {
            // Continue to normal pipeline but with tool_subset and context_hint
            // (passed to run_agent_loop_with_overrides via new parameters)
            // ... implementation below
        }
        crate::router::RoutingDecision::SemanticFilter { tool_subset } => {
            // Same as GuidedLLM but no context_hint
        }
        crate::router::RoutingDecision::FullLLM => {
            // Continue existing pipeline unchanged
        }
    }
```

- [ ] **Step 3: Implement `handle_direct_dispatch()` method**

Add a new method to `AgentLoop` (in processing.rs or a new dispatch.rs under agent/loop/):

```rust
async fn handle_direct_dispatch(
    &self,
    tool: String,
    params: serde_json::Value,
    source: crate::router::DispatchSource,
    msg: &InboundMessage,
    session_key: &str,
    router_context: &mut crate::router::context::RouterContext,
) -> Result<Option<OutboundMessage>> {
    info!(
        "direct dispatch: tool={} source={:?} channel={}",
        tool,
        source,
        msg.channel
    );

    // Handle remember fast path specially
    if tool == "_remember" {
        return self.handle_remember_dispatch(&msg.content, session_key, msg).await;
    }

    // Validate tool exists
    let Some(tool_ref) = self.tools.get(&tool) else {
        return Ok(Some(OutboundMessage {
            channel: msg.channel.clone(),
            chat_id: msg.chat_id.clone(),
            content: format!("Action failed: tool '{}' is not available.", tool),
            reply_to: msg.metadata.get(crate::bus::meta::TS).and_then(|v| v.as_str()).map(String::from),
            media: Vec::new(),
            metadata: HashMap::new(),
        }));
    };

    // Reject approval-required tools
    if tool_ref.requires_approval() {
        return Ok(Some(OutboundMessage {
            channel: msg.channel.clone(),
            chat_id: msg.chat_id.clone(),
            content: format!("Action failed: tool '{}' requires approval.", tool),
            reply_to: msg.metadata.get(crate::bus::meta::TS).and_then(|v| v.as_str()).map(String::from),
            media: Vec::new(),
            metadata: HashMap::new(),
        }));
    }

    // Secret-scan params
    let params_str = serde_json::to_string(&params).unwrap_or_default();
    let redacted = self.leak_detector.redact(&params_str);
    let params = if redacted != params_str {
        warn!("direct dispatch: secrets redacted from params");
        serde_json::from_str(&redacted).unwrap_or(params)
    } else {
        params
    };

    // Build execution context
    let ctx = ExecutionContext {
        channel: msg.channel.clone(),
        chat_id: msg.chat_id.clone(),
        context_summary: None,
        metadata: msg.metadata.clone(),
    };

    // Execute
    let result = self.tools.execute(&tool, params, &ctx).await?;

    // Extract directives from result metadata
    if let Some(ref meta) = result.metadata {
        self.update_router_context(router_context, meta);
    }

    // Handle single-use directive consumption
    if matches!(source, crate::router::DispatchSource::ActionDirective) {
        if let Some(idx) = self.router.matched_directive_index(&msg.content, router_context) {
            if router_context.action_directives.get(idx).is_some_and(|d| d.single_use) {
                router_context.remove_directive_at(idx);
            }
        }
    }

    // Save router context to session
    let mut session = self.sessions.get_or_create(session_key).await?;
    router_context.to_session_metadata(&mut session.metadata);

    // Record synthetic session history
    let action_name = result.metadata.as_ref()
        .and_then(|m| m.get("action"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let source_label = match source {
        crate::router::DispatchSource::Button => "button",
        crate::router::DispatchSource::ActionDirective => "directive",
        crate::router::DispatchSource::StaticRule => "rule",
        crate::router::DispatchSource::ConfigRule => "command",
        crate::router::DispatchSource::RememberFastPath => "remember",
        crate::router::DispatchSource::Webhook => "webhook",
    };
    session.add_message(
        "user",
        format!("[action: {}.{} via {}]", tool, action_name, source_label),
        HashMap::new(),
    );
    session.add_message("assistant", &result.content, HashMap::new());
    let _ = self.sessions.save(&session).await;

    // Build outbound message
    let mut metadata = HashMap::new();
    // Extract suggested_buttons from tool result metadata
    if let Some(ref meta) = result.metadata {
        if let Some(buttons) = meta.get("suggested_buttons") {
            metadata.insert(crate::bus::meta::BUTTONS.to_string(), buttons.clone());
        }
    }

    Ok(Some(OutboundMessage {
        channel: msg.channel.clone(),
        chat_id: msg.chat_id.clone(),
        content: result.content,
        reply_to: msg.metadata.get(crate::bus::meta::TS).and_then(|v| v.as_str()).map(String::from),
        media: Vec::new(),
        metadata,
    }))
}
```

- [ ] **Step 4: Implement `update_router_context()` helper**

```rust
fn update_router_context(
    &self,
    ctx: &mut crate::router::context::RouterContext,
    metadata: &HashMap<String, serde_json::Value>,
) {
    // Update active tool (clears directives on context switch)
    if let Some(active) = metadata.get("active_tool").and_then(|v| v.as_str()) {
        ctx.set_active_tool(Some(active.to_string()));
    }

    // Install new directives (full replacement)
    if let Some(directives_val) = metadata.get("action_directives") {
        if let Ok(directives) = serde_json::from_value::<Vec<crate::router::context::ActionDirective>>(
            directives_val.clone(),
        ) {
            ctx.install_directives(directives);
        }
    }

    ctx.updated_at_ms = crate::router::now_ms();
}
```

- [ ] **Step 5: Implement `handle_remember_dispatch()`**

Move the remember fast path logic from the old `try_remember_fast_path()` into this method. It should:
1. Extract remember content from the message
2. Run quality gates
3. Write to daily notes
4. Return confirmation OutboundMessage

This reuses the existing remember logic, just called from a different entry point.

- [ ] **Step 6: Remove old remember check from processing.rs**

Remove the `extract_remember_content()` check and `try_remember_fast_path()` call that previously existed in `process_message_unlocked()`.

- [ ] **Step 7: Remove old `classify_and_record_intent()` call**

Remove the intent classification call and its method body from processing.rs.

- [ ] **Step 8: Wire GuidedLLM and SemanticFilter paths**

For the `GuidedLLM` path, pass `tool_subset` and `context_hint` to the agent loop. This requires either:
- Adding `tool_subset` and `context_hint` fields to `AgentRunOverrides`, or
- Passing them as separate parameters

The simplest approach: add `tool_filter: Option<Vec<String>>` and `context_hint: Option<String>` to `AgentRunOverrides`. When set, `run_agent_loop_with_overrides()` filters tool definitions and injects the hint.

In `iteration.rs`, before building `tools_defs`, check for tool filter:

```rust
    let tools_defs = if let Some(ref filter) = overrides.tool_filter {
        self.tools.get_tool_definitions_with_activated(&activated_snapshot)
            .into_iter()
            .filter(|td| filter.contains(&td.name) || td.name == "add_buttons" || td.name == "tool_search")
            .collect()
    } else {
        self.tools.get_tool_definitions_with_activated(&activated_snapshot)
    };
```

And for context hint, inject into system prompt:

```rust
    if let Some(ref hint) = overrides.context_hint {
        if let Some(system_msg) = messages.first_mut() {
            system_msg.content.push_str(&format!("\n\n## Active Interaction\n\n{hint}"));
        }
    }
```

- [ ] **Step 9: Add directive extraction to GuidedLLM/FullLLM paths**

After `run_agent_loop_with_overrides()` returns, extract directives from the loop result's tool metadata. The `collected_tool_metadata` from the loop contains all tool result metadata. Extract `active_tool` and `action_directives` from the **last** tool result's metadata:

```rust
    // After agent loop completes
    if let Some(last_meta) = loop_result.response_metadata.get("_last_tool_metadata") {
        // ... extract directives
    }
```

Note: This requires threading `collected_tool_metadata` out of the agent loop. Currently it's local to `run_agent_loop_with_overrides()`. Add a field to `AgentLoopResult`:

```rust
    pub tool_metadata: Vec<HashMap<String, serde_json::Value>>,
```

Then in processing.rs, after the loop:

```rust
    if let Some(last_meta) = loop_result.tool_metadata.last() {
        self.update_router_context(&mut router_context, last_meta);
        router_context.to_session_metadata(&mut session.metadata);
        let _ = self.sessions.save(&session).await;
    }
```

- [ ] **Step 10: Run full test suite**

Run: `cargo test -- --test-threads=1`
Expected: PASS

- [ ] **Step 11: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

- [ ] **Step 12: Commit**

```bash
git add -A
git commit -m "feat(router): integrate message router into agent loop

Router sits at top of process_message_unlocked(). DirectDispatch
bypasses LLM entirely. GuidedLLM filters tools and injects context.
Directive extraction from tool results updates RouterContext per session."
```

---

## Chunk 4: Channel Integration

### Task 12: Slack button handler → ActionDispatch

**Files:**
- Modify: `src/channels/slack/mod.rs:1246-1274`

- [ ] **Step 1: Update handle_interactive_payload()**

Replace the content construction (around lines 1246-1251):

```rust
    // Try structured dispatch from button context
    let (content, dispatch) = if !action_value.is_empty() {
        if let Ok(payload) =
            serde_json::from_str::<crate::dispatch::ActionDispatchPayload>(&action_value)
        {
            let dispatch = crate::dispatch::ActionDispatch {
                tool: payload.tool,
                params: payload.params,
                source: crate::dispatch::ActionSource::Button {
                    action_id: action_id.to_string(),
                },
            };
            (format!("[button:{action_id}]"), Some(dispatch))
        } else {
            (format!("[button:{action_id}]\nButton context: {action_value}"), None)
        }
    } else {
        (format!("[button:{action_id}]"), None)
    };
```

Add `.action()` to the builder when dispatch is present:

```rust
    if let Some(d) = dispatch {
        builder = builder.action(d);
    }
```

- [ ] **Step 2: Run tests and commit**

Run: `cargo test -- --test-threads=1`

```bash
git add src/channels/slack/mod.rs
git commit -m "feat(dispatch): Slack button handler creates ActionDispatch from context"
```

---

### Task 13: Discord button handler + DispatchContextStore

**Files:**
- Modify: `src/channels/discord/mod.rs:28-35,185-226,537-562`

- [ ] **Step 1: Add dispatch_store to Handler struct**

```rust
    dispatch_store: std::sync::Arc<crate::dispatch::DispatchContextStore>,
```

Initialize in constructor with `DispatchContextStore::new(1000)`.

- [ ] **Step 2: Store payloads on button render**

In `parse_unified_buttons()`, change signature to accept optional store:

```rust
fn parse_unified_buttons(
    metadata: &HashMap<String, serde_json::Value>,
    dispatch_store: Option<&crate::dispatch::DispatchContextStore>,
) -> Vec<CreateActionRow>
```

Inside, when creating each button, try to store dispatch context:

```rust
    if let Some(store) = dispatch_store {
        if let Some(ctx_str) = b["context"].as_str() {
            if let Ok(payload) = serde_json::from_str::<crate::dispatch::ActionDispatchPayload>(ctx_str) {
                store.insert(id.to_string(), payload);
            }
        }
    }
```

Update `parse_components_from_metadata()` similarly. Update all callers to pass the store or `None`.

- [ ] **Step 3: Look up store on button click**

In `handle_component()`, after extracting `custom_id`:

```rust
    let dispatch = self.dispatch_store.get(&custom_id).map(|payload| {
        crate::dispatch::ActionDispatch {
            tool: payload.tool,
            params: payload.params,
            source: crate::dispatch::ActionSource::Button {
                action_id: custom_id.clone(),
            },
        }
    });

    let content = format!("[button:{custom_id}]");
    let mut builder = InboundMessage::builder("discord", sender_id, comp.channel_id.to_string(), content)
        .metadata(metadata);
    if let Some(d) = dispatch {
        builder = builder.action(d);
    }
```

- [ ] **Step 4: Run tests and commit**

```bash
git add src/channels/discord/mod.rs
git commit -m "feat(dispatch): Discord button handler with DispatchContextStore

Store dispatch payloads on render, look up on click. Bridges Discord's
lack of button value field."
```

---

### Task 14: Webhook dispatch

**Files:**
- Modify: `src/config/schema/mod.rs` (WebhookConfig)
- Modify: `src/gateway/mod.rs` (webhook handler)

- [ ] **Step 1: Add dispatch config to WebhookConfig**

```rust
    #[serde(default)]
    pub dispatch: Option<WebhookDispatchConfig>,
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDispatchConfig {
    pub tool: String,
    pub params_template: serde_json::Value,
}
```

Update `WebhookConfig::default()` with `dispatch: None`.

- [ ] **Step 2: Add dispatch path to webhook handler**

In the webhook handler, after template substitution, before the `agent_turn` check:

```rust
    if let Some(ref dispatch_config) = webhook_config.dispatch {
        let template_str = serde_json::to_string(&dispatch_config.params_template).unwrap_or_default();
        let rendered = apply_template(&template_str, &body_str, json_value.as_ref());
        let params: serde_json::Value = match serde_json::from_str(&rendered) {
            Ok(v) => v,
            Err(e) => {
                warn!("webhook dispatch: params parse failure: {e}");
                return (axum::http::StatusCode::BAD_REQUEST, "dispatch parse failure").into_response();
            }
        };

        let dispatch = crate::dispatch::ActionDispatch {
            tool: dispatch_config.tool.clone(),
            params,
            source: crate::dispatch::ActionSource::Webhook {
                webhook_name: webhook_name.to_string(),
            },
        };

        for target in &webhook_config.targets {
            let inbound = crate::bus::events::InboundMessage::builder(
                &target.channel, "webhook".to_string(), &target.chat_id,
                format!("[webhook-dispatch:{}]", webhook_name),
            )
            .action(dispatch.clone())
            .build();
            if let Err(e) = inbound_tx.send(inbound).await {
                warn!("webhook dispatch send error: {e}");
            }
        }
        return (axum::http::StatusCode::OK, "dispatched").into_response();
    }
```

- [ ] **Step 3: Run tests and commit**

```bash
git add src/config/schema/mod.rs src/gateway/mod.rs
git commit -m "feat(dispatch): add webhook dispatch config and handler"
```

---

## Chunk 5: Tool Migrations

### Task 15: RSS tool — structured buttons + routing_rules + directives + examples

**Files:**
- Modify: `src/agent/tools/rss/articles.rs` (button contexts, accept/reject return next inline)
- Modify: `src/agent/tools/rss/mod.rs` (routing_rules, usage_examples)

- [ ] **Step 1: Update button contexts to ActionDispatchPayload format**

Replace free-text button contexts with structured JSON:

```rust
let accept_ctx = serde_json::json!({
    "tool": "rss",
    "params": {"action": "accept", "article_ids": [&short_id]}
}).to_string();
```

- [ ] **Step 2: Add action_directives and active_tool to result metadata**

In `handle_next()`, after building the article text, add directives to metadata:

```rust
metadata.insert("active_tool".to_string(), serde_json::json!("rss"));
metadata.insert("action_directives".to_string(), serde_json::json!([
    {
        "trigger": {"OneOf": ["yes", "accept", "ok", "👍"]},
        "tool": "rss",
        "params": {"action": "accept", "article_ids": [&short_id]},
        "single_use": true,
        "ttl_ms": 300000,
        "created_at_ms": now_ms()
    },
    {
        "trigger": {"OneOf": ["no", "reject", "skip", "👎"]},
        "tool": "rss",
        "params": {"action": "reject", "article_ids": [&short_id]},
        "single_use": true,
        "ttl_ms": 300000,
        "created_at_ms": now_ms()
    },
    {
        "trigger": {"OneOf": ["next", "more"]},
        "tool": "rss",
        "params": {"action": "next"},
        "single_use": false,
        "ttl_ms": 300000,
        "created_at_ms": now_ms()
    },
    {
        "trigger": {"OneOf": ["done", "stop", "done reviewing"]},
        "tool": "rss",
        "params": {"action": "done"},
        "single_use": true,
        "ttl_ms": 300000,
        "created_at_ms": now_ms()
    }
]));
```

- [ ] **Step 3: Make accept/reject return next article inline**

In `handle_feedback()`, after processing feedback, call `handle_next()` and concatenate results.

- [ ] **Step 4: Implement `routing_rules()` on RssTool**

```rust
fn routing_rules(&self) -> Vec<crate::router::rules::StaticRule> {
    vec![
        crate::router::rules::StaticRule {
            tool: "rss".into(),
            trigger: crate::router::context::DirectiveTrigger::OneOf(
                vec!["next".into(), "more".into()],
            ),
            params: serde_json::json!({"action": "next"}),
            requires_context: true,
        },
        crate::router::rules::StaticRule {
            tool: "rss".into(),
            trigger: crate::router::context::DirectiveTrigger::OneOf(
                vec!["done".into(), "done reviewing".into(), "stop".into()],
            ),
            params: serde_json::json!({"action": "done"}),
            requires_context: true,
        },
    ]
}
```

- [ ] **Step 5: Implement `usage_examples()` on RssTool**

```rust
fn usage_examples(&self) -> Vec<crate::agent::tools::base::ToolExample> {
    vec![
        crate::agent::tools::base::ToolExample {
            user_request: "show me the next article".into(),
            params: serde_json::json!({"action": "next"}),
        },
        crate::agent::tools::base::ToolExample {
            user_request: "scan my feeds for new articles".into(),
            params: serde_json::json!({"action": "scan"}),
        },
    ]
}
```

- [ ] **Step 6: Run tests and commit**

Run: `cargo test --lib rss -- --test-threads=1`

```bash
git add src/agent/tools/rss/
git commit -m "feat(router): RSS tool with structured buttons, directives, and routing rules"
```

---

### Tasks 16-21: Remaining tool migrations

Each tool follows the same pattern as Task 15 but simpler (no inline chaining needed). For each:

1. Update button contexts to `{"tool": "...", "params": {...}}` format
2. Add `action_directives` and `active_tool` to relevant result metadata
3. Implement `routing_rules()` if the tool has context-sensitive shortcuts
4. Implement `usage_examples()` for tools with complex params (cron, calendar, github)
5. Run tool-specific tests and commit

**Task 16: Google Calendar** — Modify `src/agent/tools/google_calendar/mod.rs`. RSVP/delete buttons, `usage_examples()` for scheduling.

**Task 17: Google Mail** — Modify `src/agent/tools/google_mail/mod.rs`. Read/reply/archive buttons.

**Task 18: Google Tasks** — Modify `src/agent/tools/google_tasks/mod.rs`. Complete buttons.

**Task 19: Todoist** — Modify `src/agent/tools/todoist/mod.rs`. Complete buttons.

**Task 20: GitHub** — Modify `src/agent/tools/github/mod.rs`. Close/approve/request-changes buttons, `usage_examples()`.

**Task 21: Cron** — Modify `src/agent/tools/cron/mod.rs`. Pause/resume/remove buttons, `routing_rules()` for "list jobs", `usage_examples()` for scheduling.

Each task is one commit:
```bash
git commit -m "feat(router): migrate {tool} buttons to structured context"
```

---

## Chunk 6: Finalization

### Task 22: Integration tests

**Files:**
- Create: `tests/router_integration.rs`

- [ ] **Step 1: Write integration tests**

```rust
// Tests using MockLLMProvider and TempDir from tests/common/mod.rs

#[tokio::test]
async fn test_direct_dispatch_bypasses_llm() {
    // Build test agent with router
    // Send InboundMessage with action: Some(ActionDispatch)
    // Assert: tool executed, outbound has result
    // Assert: MockLLMProvider was never called
}

#[tokio::test]
async fn test_directive_cycle() {
    // Send message that triggers tool call via LLM
    // Tool result sets action_directives
    // Send "yes" → DirectDispatch, no LLM
    // Verify directive consumed (single_use)
}

#[tokio::test]
async fn test_context_switch_clears_directives() {
    // Establish RSS context with directives
    // Send message that triggers calendar tool
    // Verify RSS directives cleared
}

#[tokio::test]
async fn test_config_command_dispatch() {
    // Configure !weather rule
    // Send "!weather portland"
    // Verify DirectDispatch to weather tool with location=portland
}

#[tokio::test]
async fn test_router_context_persists_across_messages() {
    // First message: tool sets directives
    // Save session
    // Second message: load session, directives still there
    // "yes" → DirectDispatch
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --test router_integration -- --test-threads=1`

- [ ] **Step 3: Commit**

```bash
git add tests/router_integration.rs
git commit -m "test(router): add integration tests for message router"
```

---

### Task 23: Update CLAUDE.md documentation

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add message router documentation**

Add a new bullet in the Common Pitfalls section:

```markdown
- **Message router**: `src/router/mod.rs` contains `MessageRouter` — a stateless, sub-100μs routing engine that decides whether messages need LLM involvement. Checks in priority order: structured action payloads → session action directives → prefixed config commands (`!weather`) → static tool rules → remember fast path → guided LLM (active context, filtered tools) → semantic filter (embedding similarity) → full LLM. `RouterContext` (active tool, action directives) persists in `Session.metadata["router_context"]`. Tools declare static rules via `routing_rules()` and dynamic directives via `ToolResult.metadata["action_directives"]` + `["active_tool"]`. Directives are case-insensitive whole-message matches with TTL expiry. Config: `router.prefix` (default "!"), `router.rules`. Hallucination detection reduced to Layer 1 only (`contains_action_claims()` regex). Intent classification module and tool category filtering removed — replaced by router.
```

Also update any references to the removed modules (intent classification, hallucination layers, tool filtering).

- [ ] **Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: document message router in CLAUDE.md"
```

---

### Task 24: Final verification

- [ ] **Step 1: Run all unit tests**

Run: `cargo test --lib -- --test-threads=1`

- [ ] **Step 2: Run all integration tests**

Run: `cargo test --test session_management --test cron_jobs --test tool_registry --test message_flow --test router_integration -- --test-threads=1`

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

- [ ] **Step 4: Run fmt**

Run: `cargo fmt -- --check`

- [ ] **Step 5: Build release**

Run: `cargo build --release`
