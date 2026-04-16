use moka::sync::Cache;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Maximum length for a regex pattern trigger. Patterns exceeding this
/// are rejected to prevent `ReDoS`.
const MAX_PATTERN_LEN: usize = 256;

/// Concurrent cache of compiled regexes keyed by anchored pattern string.
static PATTERN_CACHE: LazyLock<Cache<String, Option<regex::Regex>>> =
    LazyLock::new(|| Cache::builder().max_capacity(64).build());

/// Trigger condition for a routing directive or static rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DirectiveTrigger {
    /// Single literal -- "next", "done". Hash lookup.
    Exact(String),
    /// Alternative literals -- "yes|accept|ok".
    OneOf(Vec<String>),
    /// Regex with captures. Compiled lazily. Rare.
    Pattern(String),
}

impl DirectiveTrigger {
    #[must_use]
    pub fn normalized(self) -> Self {
        match self {
            Self::Exact(s) => Self::Exact(s.to_lowercase()),
            Self::OneOf(options) => {
                Self::OneOf(options.into_iter().map(|o| o.to_lowercase()).collect())
            }
            Self::Pattern(_) => self,
        }
    }

    pub fn matches(&self, message: &str) -> bool {
        let normalized = message.trim().to_lowercase();
        self.matches_normalized(&normalized)
    }

    pub fn matches_normalized(&self, normalized: &str) -> bool {
        match self {
            Self::Exact(s) => normalized == *s,
            Self::OneOf(options) => options.iter().any(|o| o == normalized),
            Self::Pattern(pat) => {
                if pat.len() > MAX_PATTERN_LEN {
                    return false;
                }
                let anchored = format!("^(?:{pat})$");
                let compiled =
                    PATTERN_CACHE.get_with(anchored.clone(), || regex::Regex::new(&anchored).ok());
                compiled.as_ref().is_some_and(|re| re.is_match(normalized))
            }
        }
    }
}

/// Tool-declared static routing rule. Compiled at startup.
#[derive(Debug, Clone)]
pub struct StaticRule {
    pub tool: String,
    pub trigger: DirectiveTrigger,
    pub params: serde_json::Value,
    /// Only matches when this tool is the `active_tool` in `RouterContext`.
    pub requires_context: bool,
}

impl StaticRule {
    /// Check if this rule matches the message given the active tool context.
    pub fn matches(&self, message: &str, active_tool: Option<&str>) -> bool {
        let normalized = message.trim().to_lowercase();
        self.matches_normalized(&normalized, active_tool)
    }

    /// Match against a pre-lowercased, pre-trimmed message.
    /// Use this when checking multiple rules against the same message to
    /// avoid redundant `to_lowercase()` allocations.
    pub fn matches_normalized(&self, normalized: &str, active_tool: Option<&str>) -> bool {
        if self.requires_context && !active_tool.is_some_and(|a| a.eq_ignore_ascii_case(&self.tool))
        {
            return false;
        }
        self.trigger.matches_normalized(normalized)
    }
}
