use crate::agent::AgentRunOverrides;
use crate::config::schema::{ChatThresholds, ComplexityWeights};
use crate::providers::base::LLMProvider;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;

/// Pre-resolved chat complexity routing with providers ready to use.
pub struct ResolvedChatRouting {
    pub thresholds: ChatThresholds,
    pub standard: (Arc<dyn LLMProvider>, String),
    pub heavy: (Arc<dyn LLMProvider>, String),
    pub weights: ComplexityWeights,
}

/// Resolved model routing: maps task types directly to pre-created providers.
pub struct ResolvedRouting {
    tasks: HashMap<String, (Arc<dyn LLMProvider>, String)>,
    chat: Option<ResolvedChatRouting>,
}

impl ResolvedRouting {
    pub fn new(
        tasks: HashMap<String, (Arc<dyn LLMProvider>, String)>,
        chat: Option<ResolvedChatRouting>,
    ) -> Self {
        Self { tasks, chat }
    }

    /// Resolve overrides for a task type (e.g. "cron", "subagent").
    /// Direct task→provider lookup with no tier indirection.
    pub fn resolve_overrides(&self, task_type: &str) -> AgentRunOverrides {
        if let Some((provider, model)) = self.tasks.get(task_type) {
            return AgentRunOverrides {
                model: Some(model.clone()),
                provider: Some(provider.clone()),
                ..Default::default()
            };
        }
        AgentRunOverrides::default()
    }

    /// Resolve chat complexity routing: given a composite complexity score,
    /// return overrides for the appropriate model tier.
    /// Returns `None` if score is below the standard threshold (use default model).
    pub fn resolve_chat(&self, composite: f64) -> Option<AgentRunOverrides> {
        let chat = self.chat.as_ref()?;
        let (provider, model) = if composite >= chat.thresholds.heavy {
            debug!("chat complexity → heavy (score={composite:.3})");
            &chat.heavy
        } else if composite >= chat.thresholds.standard {
            debug!("chat complexity → standard (score={composite:.3})");
            &chat.standard
        } else {
            debug!("chat complexity → default (score={composite:.3})");
            return None;
        };
        Some(AgentRunOverrides {
            model: Some(model.clone()),
            provider: Some(provider.clone()),
            ..Default::default()
        })
    }

    /// Number of configured task overrides (includes chat if present).
    pub fn task_count(&self) -> usize {
        self.tasks.len() + usize::from(self.chat.is_some())
    }

    /// Whether chat complexity routing is configured.
    pub fn has_chat_routing(&self) -> bool {
        self.chat.is_some()
    }

    /// Get the complexity weights for chat routing (if configured).
    pub fn chat_weights(&self) -> Option<&ComplexityWeights> {
        self.chat.as_ref().map(|c| &c.weights)
    }

    /// Get the chat thresholds (if configured).
    pub fn chat_thresholds(&self) -> Option<&ChatThresholds> {
        self.chat.as_ref().map(|c| &c.thresholds)
    }
}
