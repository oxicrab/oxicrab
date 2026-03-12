use super::*;
use crate::config::schema::{ChatThresholds, ComplexityWeights};
use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse};
use std::sync::Arc;

struct MockProvider;

#[async_trait::async_trait]
impl LLMProvider for MockProvider {
    async fn chat(&self, _req: ChatRequest) -> anyhow::Result<LLMResponse> {
        Ok(LLMResponse::default())
    }

    fn default_model(&self) -> &'static str {
        "mock-model"
    }
}

fn mock_provider() -> Arc<dyn LLMProvider> {
    Arc::new(MockProvider)
}

fn make_chat_routing() -> ResolvedChatRouting {
    ResolvedChatRouting {
        thresholds: ChatThresholds {
            standard: 0.3,
            heavy: 0.7,
        },
        standard: (mock_provider(), "standard-model".to_string()),
        heavy: (mock_provider(), "heavy-model".to_string()),
        weights: ComplexityWeights::default(),
    }
}

#[test]
fn resolve_overrides_returns_provider_for_known_task() {
    let mut tasks = HashMap::new();
    tasks.insert("cron".to_string(), (mock_provider(), "model-a".to_string()));
    let routing = ResolvedRouting::new(tasks, None);

    let overrides = routing.resolve_overrides("cron");
    assert_eq!(overrides.model.as_deref(), Some("model-a"));
    assert!(overrides.provider.is_some());
}

#[test]
fn resolve_overrides_returns_default_for_unknown_task() {
    let routing = ResolvedRouting::new(HashMap::new(), None);

    let overrides = routing.resolve_overrides("unknown");
    assert!(overrides.model.is_none());
    assert!(overrides.provider.is_none());
}

#[test]
fn resolve_overrides_multiple_tasks() {
    let mut tasks = HashMap::new();
    tasks.insert(
        "cron".to_string(),
        (mock_provider(), "cron-model".to_string()),
    );
    tasks.insert(
        "subagent".to_string(),
        (mock_provider(), "sub-model".to_string()),
    );
    let routing = ResolvedRouting::new(tasks, None);

    assert_eq!(
        routing.resolve_overrides("cron").model.as_deref(),
        Some("cron-model")
    );
    assert_eq!(
        routing.resolve_overrides("subagent").model.as_deref(),
        Some("sub-model")
    );
    assert!(routing.resolve_overrides("other").model.is_none());
}

#[test]
fn resolve_chat_returns_none_when_no_chat_routing() {
    let routing = ResolvedRouting::new(HashMap::new(), None);
    assert!(routing.resolve_chat(0.5).is_none());
}

#[test]
fn resolve_chat_below_standard_returns_none() {
    let routing = ResolvedRouting::new(HashMap::new(), Some(make_chat_routing()));
    assert!(routing.resolve_chat(0.1).is_none());
    assert!(routing.resolve_chat(0.0).is_none());
    assert!(routing.resolve_chat(0.29).is_none());
}

#[test]
fn resolve_chat_at_standard_threshold() {
    let routing = ResolvedRouting::new(HashMap::new(), Some(make_chat_routing()));

    let overrides = routing.resolve_chat(0.3).unwrap();
    assert_eq!(overrides.model.as_deref(), Some("standard-model"));
    assert!(overrides.provider.is_some());
}

#[test]
fn resolve_chat_between_standard_and_heavy() {
    let routing = ResolvedRouting::new(HashMap::new(), Some(make_chat_routing()));

    let overrides = routing.resolve_chat(0.5).unwrap();
    assert_eq!(overrides.model.as_deref(), Some("standard-model"));
}

#[test]
fn resolve_chat_at_heavy_threshold() {
    let routing = ResolvedRouting::new(HashMap::new(), Some(make_chat_routing()));

    let overrides = routing.resolve_chat(0.7).unwrap();
    assert_eq!(overrides.model.as_deref(), Some("heavy-model"));
    assert!(overrides.provider.is_some());
}

#[test]
fn resolve_chat_above_heavy_threshold() {
    let routing = ResolvedRouting::new(HashMap::new(), Some(make_chat_routing()));

    let overrides = routing.resolve_chat(0.95).unwrap();
    assert_eq!(overrides.model.as_deref(), Some("heavy-model"));
}

#[test]
fn task_count_empty() {
    let routing = ResolvedRouting::new(HashMap::new(), None);
    assert_eq!(routing.task_count(), 0);
}

#[test]
fn task_count_tasks_only() {
    let mut tasks = HashMap::new();
    tasks.insert("cron".to_string(), (mock_provider(), "m".to_string()));
    tasks.insert("subagent".to_string(), (mock_provider(), "m".to_string()));
    let routing = ResolvedRouting::new(tasks, None);
    assert_eq!(routing.task_count(), 2);
}

#[test]
fn task_count_includes_chat() {
    let mut tasks = HashMap::new();
    tasks.insert("cron".to_string(), (mock_provider(), "m".to_string()));
    let routing = ResolvedRouting::new(tasks, Some(make_chat_routing()));
    assert_eq!(routing.task_count(), 2);
}

#[test]
fn has_chat_routing_false_when_none() {
    let routing = ResolvedRouting::new(HashMap::new(), None);
    assert!(!routing.has_chat_routing());
}

#[test]
fn has_chat_routing_true_when_configured() {
    let routing = ResolvedRouting::new(HashMap::new(), Some(make_chat_routing()));
    assert!(routing.has_chat_routing());
}

#[test]
fn chat_weights_returns_none_without_chat() {
    let routing = ResolvedRouting::new(HashMap::new(), None);
    assert!(routing.chat_weights().is_none());
}

#[test]
fn chat_weights_returns_weights_with_chat() {
    let routing = ResolvedRouting::new(HashMap::new(), Some(make_chat_routing()));
    let weights = routing.chat_weights().unwrap();
    // ComplexityWeights::default() has known values
    assert!(weights.message_length > 0.0);
}

#[test]
fn chat_thresholds_returns_none_without_chat() {
    let routing = ResolvedRouting::new(HashMap::new(), None);
    assert!(routing.chat_thresholds().is_none());
}

#[test]
fn chat_thresholds_returns_configured_values() {
    let routing = ResolvedRouting::new(HashMap::new(), Some(make_chat_routing()));
    let thresholds = routing.chat_thresholds().unwrap();
    assert!((thresholds.standard - 0.3).abs() < f64::EPSILON);
    assert!((thresholds.heavy - 0.7).abs() < f64::EPSILON);
}

#[test]
fn providers_empty_routing() {
    let routing = ResolvedRouting::new(HashMap::new(), None);
    let providers: Vec<_> = routing.providers().collect();
    assert!(providers.is_empty());
}

#[test]
fn providers_with_tasks_and_chat() {
    let mut tasks = HashMap::new();
    tasks.insert(
        "cron".to_string(),
        (mock_provider(), "cron-model".to_string()),
    );
    let routing = ResolvedRouting::new(tasks, Some(make_chat_routing()));
    let providers: Vec<_> = routing.providers().collect();
    // 1 task + 2 chat (standard + heavy)
    assert_eq!(providers.len(), 3);
    let models: Vec<&str> = providers.iter().map(|(_, m)| *m).collect();
    assert!(models.contains(&"cron-model"));
    assert!(models.contains(&"standard-model"));
    assert!(models.contains(&"heavy-model"));
}

#[test]
fn providers_tasks_only() {
    let mut tasks = HashMap::new();
    tasks.insert("cron".to_string(), (mock_provider(), "a".to_string()));
    tasks.insert("subagent".to_string(), (mock_provider(), "b".to_string()));
    let routing = ResolvedRouting::new(tasks, None);
    let providers: Vec<_> = routing.providers().collect();
    assert_eq!(providers.len(), 2);
}
