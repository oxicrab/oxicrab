use crate::agent::AgentRunOverrides;
use crate::providers::base::LLMProvider;
use std::collections::HashMap;
use std::sync::Arc;

/// Resolved model routing: maps tier names to pre-created providers.
pub struct ResolvedRouting {
    tiers: HashMap<String, (Arc<dyn LLMProvider>, String)>,
    rules: HashMap<String, String>,
}

impl ResolvedRouting {
    pub fn new(
        tiers: HashMap<String, (Arc<dyn LLMProvider>, String)>,
        rules: HashMap<String, String>,
    ) -> Self {
        Self { tiers, rules }
    }

    /// Resolve overrides for a task type (e.g. "daemon", "cron", "subagent").
    pub fn resolve_overrides(&self, task_type: &str) -> AgentRunOverrides {
        if let Some(tier_name) = self.rules.get(task_type)
            && let Some((provider, model)) = self.tiers.get(tier_name)
        {
            return AgentRunOverrides {
                model: Some(model.clone()),
                provider: Some(provider.clone()),
                max_iterations: None,
                response_format: None,
            };
        }
        AgentRunOverrides::default()
    }

    /// Resolve overrides by tier name directly (bypasses the rules map).
    /// Used by complexity-aware routing which already knows the tier name.
    pub fn resolve_tier_direct(&self, tier_name: &str) -> AgentRunOverrides {
        if let Some((provider, model)) = self.tiers.get(tier_name) {
            return AgentRunOverrides {
                model: Some(model.clone()),
                provider: Some(provider.clone()),
                max_iterations: None,
                response_format: None,
            };
        }
        AgentRunOverrides::default()
    }

    /// Number of configured tiers.
    pub fn tier_count(&self) -> usize {
        self.tiers.len()
    }
}
