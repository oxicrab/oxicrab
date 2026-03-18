//! Provider factory — creates LLM provider instances from config.
//!
//! These functions were extracted from `Config` methods to decouple the config
//! schema (pure data types + validation) from provider implementations.

use crate::config::TaskRouting;
use crate::config::routing::{ResolvedChatRouting, ResolvedRouting};
use crate::config::schema::{Config, normalize_provider, parse_model_ref};
use crate::providers::base::LLMProvider;
use crate::providers::strategy::ProviderFactory;
use crate::utils::credential_store::OAuthTokenStore;
use std::collections::HashMap;
use std::sync::Arc;

/// Create an LLM provider instance based on configuration.
///
/// Uses a 2-tier resolution strategy: prefix notation, then model-name
/// inference. When fallbacks are configured, wraps the primary provider
/// in a `FallbackProvider` chain.
pub fn create_provider(
    config: &Config,
    model: Option<&str>,
    db: Option<Arc<dyn OAuthTokenStore>>,
) -> anyhow::Result<Arc<dyn LLMProvider>> {
    let model = model.unwrap_or(&config.agents.defaults.model_routing.default);
    let factory = ProviderFactory::with_db(config, db);

    // Build fallback chain from modelRouting.fallbacks
    let routing = &config.agents.defaults.model_routing;
    if !routing.fallbacks.is_empty() {
        let mut primary = factory.create_provider(model)?;
        if should_use_prompt_guided_tools(config, model) {
            primary = crate::providers::prompt_guided::PromptGuidedToolsProvider::wrap(primary);
        }
        let primary_bare = parse_model_ref(model).model.to_string();
        let mut chain = vec![(primary, primary_bare)];
        for fb_model in &routing.fallbacks {
            let mut fb_provider = factory.create_provider(fb_model)?;
            if should_use_prompt_guided_tools(config, fb_model) {
                fb_provider =
                    crate::providers::prompt_guided::PromptGuidedToolsProvider::wrap(fb_provider);
            }
            let fb_bare = parse_model_ref(fb_model).model.to_string();
            chain.push((fb_provider, fb_bare));
        }
        return Ok(Arc::new(crate::providers::fallback::FallbackProvider::new(
            chain,
        )?));
    }

    let provider = factory.create_provider(model)?;
    if should_use_prompt_guided_tools(config, model) {
        return Ok(crate::providers::prompt_guided::PromptGuidedToolsProvider::wrap(provider));
    }

    Ok(provider)
}

/// Create providers for all configured model routing task overrides.
pub fn create_routed_providers(
    config: &Config,
    db: Option<Arc<dyn OAuthTokenStore>>,
) -> anyhow::Result<Option<ResolvedRouting>> {
    let routing = &config.agents.defaults.model_routing;
    if routing.tasks.is_empty() {
        return Ok(None);
    }
    let factory = ProviderFactory::with_db(config, db);

    // Deduplicated provider cache: model_str -> (provider, model)
    let mut provider_cache: HashMap<String, (Arc<dyn LLMProvider>, String)> = HashMap::new();

    let mut get_or_create = |model_str: &str| -> anyhow::Result<(Arc<dyn LLMProvider>, String)> {
        if let Some(cached) = provider_cache.get(model_str) {
            return Ok(cached.clone());
        }
        let bare_model = parse_model_ref(model_str).model.to_string();
        let mut provider = factory.create_provider(model_str)?;
        if should_use_prompt_guided_tools(config, model_str) {
            provider = crate::providers::prompt_guided::PromptGuidedToolsProvider::wrap(provider);
        }
        let entry = (provider, bare_model);
        provider_cache.insert(model_str.to_string(), entry.clone());
        Ok(entry)
    };

    let mut tasks = HashMap::new();
    let mut chat = None;

    for (task_name, task_routing) in &routing.tasks {
        match task_routing {
            TaskRouting::Model(model_str) => {
                let entry = get_or_create(model_str)?;
                tasks.insert(task_name.clone(), entry);
            }
            TaskRouting::Chat(chat_config) => {
                let standard = get_or_create(&chat_config.models.standard)?;
                let heavy = get_or_create(&chat_config.models.heavy)?;
                chat = Some(ResolvedChatRouting {
                    thresholds: chat_config.thresholds.clone(),
                    standard,
                    heavy,
                    weights: chat_config.weights.clone(),
                });
            }
        }
    }

    Ok(Some(ResolvedRouting::new(tasks, chat)))
}

/// Check if a model should use prompt-guided tool calling based on its
/// resolved provider config.
fn should_use_prompt_guided_tools(config: &Config, model: &str) -> bool {
    use crate::config::schema::infer_provider_from_model;

    let model_ref = parse_model_ref(model);
    if let Some(prefix_provider) = model_ref.provider {
        let normalized = normalize_provider(prefix_provider);
        return match normalized.as_ref() {
            "ollama" => config.providers.ollama.prompt_guided_tools,
            "vllm" => config.providers.vllm.prompt_guided_tools,
            _ => false,
        };
    }

    if let Some(inferred) = infer_provider_from_model(model_ref.model) {
        return match inferred {
            "ollama" => config.providers.ollama.prompt_guided_tools,
            "vllm" => config.providers.vllm.prompt_guided_tools,
            _ => false,
        };
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_prompt_guided_tools_prefix_notation() {
        let mut config = Config::default();
        config.providers.ollama.prompt_guided_tools = true;
        assert!(should_use_prompt_guided_tools(&config, "ollama/llama3"));
    }

    #[test]
    fn test_prompt_guided_tools_known_model_returns_false() {
        let config = Config::default();
        assert!(!should_use_prompt_guided_tools(
            &config,
            "claude-sonnet-4-5-20250929"
        ));
        assert!(!should_use_prompt_guided_tools(&config, "gpt-4"));
        assert!(!should_use_prompt_guided_tools(&config, "gemini-pro"));
    }
}
