use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use oxicrab_core::tools::base::routing_types::DirectiveTrigger;

pub const MAX_DIRECTIVES: usize = 20;
pub const DEFAULT_DIRECTIVE_TTL_MS: i64 = 300_000; // 5 minutes

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextState {
    #[default]
    Idle,
    ToolFocused {
        tool: String,
        directives: Vec<ActionDirective>,
        expires_at_ms: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterState<'a> {
    Idle,
    Focused { tool: &'a str },
}

#[derive(Debug, Clone, Default)]
struct DirectiveMatcher {
    literal_to_index: HashMap<String, usize>,
    pattern_indices: Vec<usize>,
}

impl DirectiveMatcher {
    fn compile(directives: &[ActionDirective]) -> Self {
        let mut m = Self::default();
        for (idx, d) in directives.iter().enumerate() {
            match &d.trigger {
                DirectiveTrigger::Exact(s) => {
                    if m.literal_to_index.contains_key(s) {
                        tracing::debug!(
                            "router: directive literal conflict for '{}' (keeping first)",
                            s
                        );
                    } else {
                        m.literal_to_index.insert(s.clone(), idx);
                    }
                }
                DirectiveTrigger::OneOf(options) => {
                    for opt in options {
                        if m.literal_to_index.contains_key(opt) {
                            tracing::warn!(
                                "router: directive literal conflict for '{}' (keeping first)",
                                opt
                            );
                        } else {
                            m.literal_to_index.insert(opt.clone(), idx);
                        }
                    }
                }
                DirectiveTrigger::Pattern(_) => m.pattern_indices.push(idx),
            }
        }
        m
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouterContext {
    pub state: ContextState,
    pub updated_at_ms: i64,
    /// Pre-compiled directive matcher. `#[serde(skip)]` means this is `None`
    /// after raw deserialization — always use `from_session_metadata()` which
    /// calls `rebuild_matcher()`.
    #[serde(skip, default)]
    matcher: DirectiveMatcher,
}

impl RouterContext {
    /// Load from session metadata. Returns default if missing or malformed.
    pub fn from_session_metadata(
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Self {
        let Some(value) = metadata.get("router_context") else {
            return Self::default();
        };
        match serde_json::from_value::<Self>(value.clone()) {
            Ok(mut ctx) => {
                ctx.rebuild_matcher();
                ctx
            }
            Err(_) => Self::default(),
        }
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

    pub fn active_tool(&self) -> Option<&str> {
        match &self.state {
            ContextState::ToolFocused { tool, .. } => Some(tool.as_str()),
            ContextState::Idle => None,
        }
    }

    pub fn directives(&self) -> &[ActionDirective] {
        match &self.state {
            ContextState::ToolFocused { directives, .. } => directives,
            ContextState::Idle => &[],
        }
    }

    /// Returns the current routing state. Note: when directives are empty,
    /// this returns `Idle` even though `active_tool()` returns `Some(tool)`.
    /// This is intentional — the router's tier 7 check uses `state()`, so an
    /// active tool without directives falls through to FullLLM correctly.
    pub fn state(&self, now_ms: i64) -> RouterState<'_> {
        match &self.state {
            ContextState::ToolFocused {
                tool,
                directives,
                expires_at_ms,
            } if *expires_at_ms > now_ms && directives.iter().any(|d| !d.is_expired(now_ms)) => {
                RouterState::Focused {
                    tool: tool.as_str(),
                }
            }
            _ => RouterState::Idle,
        }
    }

    /// Transition to `Idle` state.
    pub fn set_idle(&mut self) {
        self.state = ContextState::Idle;
        self.matcher = DirectiveMatcher::default();
    }

    /// Transition active tool. Switching tools clears directives.
    pub fn set_active_tool(&mut self, tool: Option<String>) {
        match tool {
            Some(next_tool) => match &mut self.state {
                ContextState::ToolFocused {
                    tool,
                    directives,
                    expires_at_ms,
                } => {
                    if *tool != next_tool {
                        *tool = next_tool;
                        directives.clear();
                        *expires_at_ms = 0;
                        self.matcher = DirectiveMatcher::default();
                    }
                }
                ContextState::Idle => {
                    self.state = ContextState::ToolFocused {
                        tool: next_tool,
                        directives: Vec::new(),
                        expires_at_ms: 0,
                    };
                    self.matcher = DirectiveMatcher::default();
                }
            },
            None => self.set_idle(),
        }
    }

    /// Replace directives in focused state. Caps at `MAX_DIRECTIVES`.
    pub fn install_directives(&mut self, directives: Vec<ActionDirective>) {
        let mut normalized: Vec<ActionDirective> = directives
            .into_iter()
            .map(|mut d| {
                d.trigger = d.trigger.normalized();
                d
            })
            .collect();
        normalized.truncate(MAX_DIRECTIVES);

        match &mut self.state {
            ContextState::ToolFocused {
                directives: current,
                expires_at_ms,
                ..
            } => {
                *current = normalized;
                *expires_at_ms = current
                    .iter()
                    .map(|d| d.created_at_ms + d.ttl_ms)
                    .max()
                    .unwrap_or(0);
                self.rebuild_matcher();
            }
            ContextState::Idle => {
                // No active tool, directives have no meaning.
                self.matcher = DirectiveMatcher::default();
            }
        }
    }

    /// Remove expired directives and transition to idle when focus expires.
    pub fn prune_expired(&mut self, now_ms: i64) {
        if let ContextState::ToolFocused {
            directives,
            expires_at_ms,
            ..
        } = &mut self.state
        {
            directives.retain(|d| !d.is_expired(now_ms));
            *expires_at_ms = directives
                .iter()
                .map(|d| d.created_at_ms + d.ttl_ms)
                .max()
                .unwrap_or(0);
            if directives.is_empty() || *expires_at_ms <= now_ms {
                self.set_idle();
            } else {
                self.rebuild_matcher();
            }
        }
    }

    /// Remove directive at index (for single-use consumption).
    pub fn remove_directive_at(&mut self, index: usize) {
        if let ContextState::ToolFocused {
            directives,
            expires_at_ms,
            ..
        } = &mut self.state
            && index < directives.len()
        {
            directives.remove(index);
            *expires_at_ms = directives
                .iter()
                .map(|d| d.created_at_ms + d.ttl_ms)
                .max()
                .unwrap_or(0);
            if directives.is_empty() {
                self.set_idle();
            } else {
                self.rebuild_matcher();
            }
        }
    }

    /// Match an already normalized input against focused directives.
    pub fn match_directive_index(&self, normalized_message: &str, now_ms: i64) -> Option<usize> {
        let ContextState::ToolFocused { directives, .. } = &self.state else {
            return None;
        };

        if let Some(&idx) = self.matcher.literal_to_index.get(normalized_message)
            && directives.get(idx).is_some_and(|d| !d.is_expired(now_ms))
        {
            return Some(idx);
        }

        for idx in &self.matcher.pattern_indices {
            if directives.get(*idx).is_some_and(|d| {
                !d.is_expired(now_ms) && d.trigger.matches_normalized(normalized_message)
            }) {
                return Some(*idx);
            }
        }
        None
    }

    fn rebuild_matcher(&mut self) {
        self.matcher = DirectiveMatcher::compile(self.directives());
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

#[cfg(test)]
mod tests;
