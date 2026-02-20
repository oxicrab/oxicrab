use crate::config::schema::{AnthropicOAuthConfig, ProviderConfig, ProvidersConfig};
use crate::providers::base::LLMProvider;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// Default base URLs for OpenAI-compatible providers.
/// Each entry maps a provider keyword (matched against model name) to its default chat completions endpoint.
const OPENAI_COMPAT_PROVIDERS: &[(&str, &str)] = &[
    (
        "openrouter",
        "https://openrouter.ai/api/v1/chat/completions",
    ),
    ("deepseek", "https://api.deepseek.com/v1/chat/completions"),
    ("groq", "https://api.groq.com/openai/v1/chat/completions"),
    ("moonshot", "https://api.moonshot.cn/v1/chat/completions"),
    (
        "zhipu",
        "https://open.bigmodel.cn/api/paas/v4/chat/completions",
    ),
    (
        "dashscope",
        "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions",
    ),
    ("vllm", "http://localhost:8000/v1/chat/completions"),
    ("ollama", "http://localhost:11434/v1/chat/completions"),
];

/// Strategy for creating LLM providers based on model name
#[async_trait]
pub trait ProviderStrategy: Send + Sync {
    /// Check if this strategy can handle the given model
    fn can_handle(&self, model: &str) -> bool;

    /// Create a provider for the given model
    async fn create_provider(&self, model: &str) -> Result<Option<Arc<dyn LLMProvider>>>;
}

/// Strategy for Anthropic OAuth providers
pub struct AnthropicOAuthStrategy {
    config: AnthropicOAuthConfig,
}

impl AnthropicOAuthStrategy {
    pub fn new(config: AnthropicOAuthConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ProviderStrategy for AnthropicOAuthStrategy {
    fn can_handle(&self, model: &str) -> bool {
        model.starts_with("anthropic/")
            || (model.to_lowercase().contains("anthropic")
                || model.to_lowercase().contains("claude"))
    }

    async fn create_provider(&self, model: &str) -> Result<Option<Arc<dyn LLMProvider>>> {
        use crate::providers::anthropic_oauth::AnthropicOAuthProvider;

        let should_try_oauth =
            model.starts_with("anthropic/") || self.config.enabled || self.config.auto_detect;

        if !should_try_oauth {
            return Ok(None);
        }

        // Try explicit config first
        if !self.config.access_token.is_empty() {
            return Ok(Some(Arc::new(AnthropicOAuthProvider::new(
                self.config.access_token.clone(),
                self.config.refresh_token.clone(),
                self.config.expires_at,
                Some(model.to_string()),
                self.config
                    .credentials_path
                    .as_ref()
                    .map(std::path::PathBuf::from),
            )?)));
        }

        // Try auto-detection
        if model.starts_with("anthropic/") || self.config.auto_detect {
            // Try Claude CLI
            if let Ok(Some(provider)) =
                AnthropicOAuthProvider::from_claude_cli(Some(model.to_string()))
            {
                return Ok(Some(Arc::new(provider)));
            }

            // Try OpenClaw
            if let Ok(Some(provider)) =
                AnthropicOAuthProvider::from_openclaw(Some(model.to_string()))
            {
                return Ok(Some(Arc::new(provider)));
            }

            // Try credentials file
            if let Some(ref path) = self.config.credentials_path {
                let path_buf = std::path::PathBuf::from(path);
                if let Ok(Some(provider)) = AnthropicOAuthProvider::from_credentials_file(
                    &path_buf,
                    Some(model.to_string()),
                ) {
                    return Ok(Some(Arc::new(provider)));
                }
            }
        }

        Ok(None)
    }
}

/// Strategy for API key-based providers
pub struct ApiKeyProviderStrategy {
    config: ProvidersConfig,
}

impl ApiKeyProviderStrategy {
    pub fn new(config: ProvidersConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ProviderStrategy for ApiKeyProviderStrategy {
    fn can_handle(&self, _model: &str) -> bool {
        true // Fallback strategy - can handle any model
    }

    async fn create_provider(&self, model: &str) -> Result<Option<Arc<dyn LLMProvider>>> {
        use crate::providers::{
            anthropic::AnthropicProvider, gemini::GeminiProvider, openai::OpenAIProvider,
        };

        let model_lower = model.to_lowercase();

        // Try OpenAI-compatible providers first (deepseek, groq, openrouter, etc.)
        if let Some(provider) = self.try_openai_compat(model) {
            return Ok(Some(provider));
        }

        // For Claude models, try Anthropic API key directly
        if (model_lower.contains("anthropic") || model_lower.contains("claude"))
            && !self.config.anthropic.api_key.is_empty()
        {
            info!("Using Anthropic API key provider for model: {}", model);
            return Ok(Some(Arc::new(AnthropicProvider::new(
                self.config.anthropic.api_key.clone(),
                Some(model.to_string()),
            ))));
        }

        // Get API key for model
        let api_key = self.get_api_key(model);
        let model_str = model.to_string();

        if let Some(key) = api_key {
            info!("Using API key provider for model: {}", model);
            if model_lower.contains("anthropic") || model_lower.contains("claude") {
                Ok(Some(Arc::new(AnthropicProvider::new(key, Some(model_str)))))
            } else if model_lower.contains("openai") || model_lower.contains("gpt") {
                Ok(Some(Arc::new(OpenAIProvider::new(key, Some(model_str)))))
            } else if model_lower.contains("gemini") {
                Ok(Some(Arc::new(GeminiProvider::new(key, Some(model_str)))))
            } else {
                // Default to Anthropic
                Ok(Some(Arc::new(AnthropicProvider::new(key, Some(model_str)))))
            }
        } else {
            Ok(None)
        }
    }
}

impl ApiKeyProviderStrategy {
    fn get_api_key(&self, model: &str) -> Option<String> {
        self.config.get_api_key(model).map(str::to_owned)
    }

    /// Look up the `ProviderConfig` for a given keyword from the providers config.
    fn get_provider_config(&self, keyword: &str) -> Option<&ProviderConfig> {
        match keyword {
            "openrouter" => Some(&self.config.openrouter),
            "deepseek" => Some(&self.config.deepseek),
            "groq" => Some(&self.config.groq),
            "moonshot" => Some(&self.config.moonshot),
            "zhipu" => Some(&self.config.zhipu),
            "dashscope" => Some(&self.config.dashscope),
            "vllm" => Some(&self.config.vllm),
            "ollama" => Some(&self.config.ollama),
            _ => None,
        }
    }

    /// Try to create an OpenAI-compatible provider if the model name matches a known keyword.
    fn try_openai_compat(&self, model: &str) -> Option<Arc<dyn LLMProvider>> {
        use crate::providers::openai::OpenAIProvider;

        let model_lower = model.to_lowercase();

        for &(keyword, default_url) in OPENAI_COMPAT_PROVIDERS {
            if !model_lower.contains(keyword) {
                continue;
            }

            // Local providers (ollama, vllm) don't require API keys
            let is_local = matches!(keyword, "ollama" | "vllm");
            let provider_config = self.get_provider_config(keyword)?;
            if provider_config.api_key.is_empty() && !is_local {
                return None;
            }

            let base_url = provider_config
                .api_base
                .as_deref()
                .unwrap_or(default_url)
                .to_string();

            let provider_name = keyword[..1].to_uppercase() + &keyword[1..];

            // Strip the "keyword/" prefix if present — providers expect the bare model name
            let prefix = format!("{}/", keyword);
            let api_model = if model_lower.starts_with(&prefix) {
                // Preserve original casing; prefix is ASCII so byte offset is safe
                &model[prefix.len()..]
            } else {
                model
            };

            info!(
                "Using OpenAI-compatible provider ({}) for model: {} (api_model: {})",
                provider_name, model, api_model
            );

            if provider_config.headers.is_empty() {
                return Some(Arc::new(OpenAIProvider::with_config(
                    provider_config.api_key.clone(),
                    api_model.to_string(),
                    base_url,
                    provider_name,
                )));
            }
            return Some(Arc::new(OpenAIProvider::with_config_and_headers(
                provider_config.api_key.clone(),
                api_model.to_string(),
                base_url,
                provider_name,
                provider_config.headers.clone(),
            )));
        }

        None
    }
}

/// Provider factory that uses strategies to create providers
pub struct ProviderFactory {
    strategies: Vec<Box<dyn ProviderStrategy>>,
}

impl ProviderFactory {
    pub fn new(config: &crate::config::schema::Config) -> Self {
        let strategies: Vec<Box<dyn ProviderStrategy>> = vec![
            // OAuth strategy first (higher priority)
            Box::new(AnthropicOAuthStrategy::new(
                config.providers.anthropic_oauth.clone(),
            )),
            // API key strategy as fallback
            Box::new(ApiKeyProviderStrategy::new(config.providers.clone())),
        ];

        Self { strategies }
    }

    pub async fn create_provider(&self, model: &str) -> Result<Arc<dyn LLMProvider>> {
        // Try each strategy in order
        for strategy in &self.strategies {
            if strategy.can_handle(model)
                && let Some(provider) = strategy.create_provider(model).await?
            {
                return Ok(provider);
            }
        }

        // If we get here and model requires OAuth, provide helpful error
        if model.starts_with("anthropic/") {
            anyhow::bail!(
                "Model '{}' requires OAuth authentication. Auto-detection failed to find credentials.\n\
                \n\
                Options:\n\
                1. Install Claude CLI: https://github.com/anthropics/claude-cli\n\
                2. Install OpenClaw: https://github.com/anthropics/openclaw\n\
                3. Set 'providers.anthropicOAuth.credentialsPath' in ~/.oxicrab/config.json\n\
                4. Use an API key model instead (e.g., 'claude-sonnet-4-5-20250929')",
                model
            );
        }

        anyhow::bail!("No API key configured for model: {}", model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::ProvidersConfig;

    fn config_with_deepseek() -> ProvidersConfig {
        let mut config = ProvidersConfig::default();
        config.deepseek.api_key = "sk-deepseek-test".to_string();
        config
    }

    #[test]
    fn test_openai_compat_deepseek_routing() {
        let strategy = ApiKeyProviderStrategy::new(config_with_deepseek());
        let provider = strategy.try_openai_compat("deepseek-chat");
        assert!(
            provider.is_some(),
            "deepseek-chat should route to OpenAI-compat provider"
        );
        assert_eq!(provider.unwrap().default_model(), "deepseek-chat");
    }

    #[test]
    fn test_openai_compat_uses_custom_api_base() {
        let mut config = config_with_deepseek();
        config.deepseek.api_base =
            Some("https://custom.endpoint.com/v1/chat/completions".to_string());
        let strategy = ApiKeyProviderStrategy::new(config);
        let provider = strategy.try_openai_compat("deepseek-coder");
        assert!(
            provider.is_some(),
            "deepseek-coder should route with custom api_base"
        );
    }

    #[test]
    fn test_openai_compat_no_api_key() {
        let config = ProvidersConfig::default();
        let strategy = ApiKeyProviderStrategy::new(config);
        let provider = strategy.try_openai_compat("deepseek-chat");
        assert!(
            provider.is_none(),
            "should return None when no API key is configured"
        );
    }

    #[test]
    fn test_native_providers_not_affected() {
        let mut config = ProvidersConfig::default();
        config.anthropic.api_key = "sk-ant-test".to_string();
        config.openai.api_key = "sk-openai-test".to_string();
        let strategy = ApiKeyProviderStrategy::new(config);

        // Native providers should not match OpenAI-compat keywords
        assert!(
            strategy
                .try_openai_compat("claude-sonnet-4-5-20250929")
                .is_none()
        );
        assert!(strategy.try_openai_compat("gpt-4o").is_none());
        assert!(strategy.try_openai_compat("gemini-pro").is_none());
    }

    #[tokio::test]
    async fn test_deepseek_routes_through_create_provider() {
        let strategy = ApiKeyProviderStrategy::new(config_with_deepseek());
        let result = strategy.create_provider("deepseek-chat").await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().default_model(), "deepseek-chat");
    }

    #[test]
    fn test_groq_routing() {
        let mut config = ProvidersConfig::default();
        config.groq.api_key = "gsk-test".to_string();
        let strategy = ApiKeyProviderStrategy::new(config);
        let provider = strategy.try_openai_compat("groq/llama-3.1-70b");
        assert!(provider.is_some());
        // Prefix stripped: "groq/llama-3.1-70b" → "llama-3.1-70b"
        assert_eq!(provider.unwrap().default_model(), "llama-3.1-70b");
    }

    #[test]
    fn test_ollama_routing_no_api_key() {
        // Ollama should work without an API key
        let config = ProvidersConfig::default();
        let strategy = ApiKeyProviderStrategy::new(config);
        let provider = strategy.try_openai_compat("ollama/qwen3-coder:30b");
        assert!(
            provider.is_some(),
            "ollama should route even without an API key"
        );
        // Prefix stripped: "ollama/qwen3-coder:30b" → "qwen3-coder:30b"
        assert_eq!(provider.unwrap().default_model(), "qwen3-coder:30b");
    }

    #[test]
    fn test_ollama_routing_with_api_key() {
        let mut config = ProvidersConfig::default();
        config.ollama.api_key = "optional-key".to_string();
        let strategy = ApiKeyProviderStrategy::new(config);
        let provider = strategy.try_openai_compat("ollama/llama3");
        assert!(provider.is_some());
    }

    #[test]
    fn test_ollama_custom_api_base() {
        let mut config = ProvidersConfig::default();
        config.ollama.api_base = Some("http://192.168.1.100:11434/v1/chat/completions".to_string());
        let strategy = ApiKeyProviderStrategy::new(config);
        let provider = strategy.try_openai_compat("ollama/qwen3-coder:30b");
        assert!(provider.is_some());
    }

    #[test]
    fn test_vllm_routing_no_api_key() {
        // vLLM should also work without an API key
        let config = ProvidersConfig::default();
        let strategy = ApiKeyProviderStrategy::new(config);
        let provider = strategy.try_openai_compat("vllm/my-model");
        assert!(
            provider.is_some(),
            "vllm should route even without an API key"
        );
    }

    #[tokio::test]
    async fn test_ollama_routes_through_create_provider() {
        let config = ProvidersConfig::default();
        let strategy = ApiKeyProviderStrategy::new(config);
        let result = strategy
            .create_provider("ollama/qwen3-coder:30b")
            .await
            .unwrap();
        assert!(result.is_some());
        // Prefix stripped
        assert_eq!(result.unwrap().default_model(), "qwen3-coder:30b");
    }

    #[test]
    fn test_get_provider_config_ollama() {
        let config = ProvidersConfig::default();
        let strategy = ApiKeyProviderStrategy::new(config);
        assert!(strategy.get_provider_config("ollama").is_some());
    }

    #[test]
    fn test_non_local_provider_still_needs_key() {
        // Deepseek should still fail without an API key
        let config = ProvidersConfig::default();
        let strategy = ApiKeyProviderStrategy::new(config);
        let provider = strategy.try_openai_compat("deepseek-chat");
        assert!(
            provider.is_none(),
            "deepseek should return None without an API key"
        );
    }

    // --- AnthropicOAuthStrategy tests ---

    #[test]
    fn test_oauth_can_handle_anthropic_prefix() {
        let strategy =
            AnthropicOAuthStrategy::new(crate::config::schema::AnthropicOAuthConfig::default());
        assert!(strategy.can_handle("anthropic/claude-opus-4-6"));
    }

    #[test]
    fn test_oauth_can_handle_claude_model() {
        let strategy =
            AnthropicOAuthStrategy::new(crate::config::schema::AnthropicOAuthConfig::default());
        assert!(strategy.can_handle("claude-sonnet-4-5-20250929"));
    }

    #[test]
    fn test_oauth_cannot_handle_openai() {
        let strategy =
            AnthropicOAuthStrategy::new(crate::config::schema::AnthropicOAuthConfig::default());
        assert!(!strategy.can_handle("gpt-4o"));
    }

    #[test]
    fn test_oauth_cannot_handle_gemini() {
        let strategy =
            AnthropicOAuthStrategy::new(crate::config::schema::AnthropicOAuthConfig::default());
        assert!(!strategy.can_handle("gemini-pro"));
    }

    #[test]
    fn test_api_key_can_handle_anything() {
        let strategy = ApiKeyProviderStrategy::new(ProvidersConfig::default());
        assert!(strategy.can_handle("anything-at-all"));
        assert!(strategy.can_handle(""));
    }

    #[test]
    fn test_openai_compat_strips_keyword_prefix() {
        let mut config = ProvidersConfig::default();
        config.groq.api_key = "gsk-test".to_string();
        let strategy = ApiKeyProviderStrategy::new(config);
        let provider = strategy.try_openai_compat("groq/llama-3.3-70b-versatile");
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().default_model(), "llama-3.3-70b-versatile");
    }

    #[test]
    fn test_openai_compat_no_prefix_preserved() {
        let mut config = ProvidersConfig::default();
        config.deepseek.api_key = "sk-test".to_string();
        let strategy = ApiKeyProviderStrategy::new(config);
        let provider = strategy.try_openai_compat("deepseek-coder-v2");
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().default_model(), "deepseek-coder-v2");
    }

    #[test]
    fn test_get_provider_config_all_known() {
        let strategy = ApiKeyProviderStrategy::new(ProvidersConfig::default());
        for keyword in [
            "openrouter",
            "deepseek",
            "groq",
            "moonshot",
            "zhipu",
            "dashscope",
            "vllm",
            "ollama",
        ] {
            assert!(
                strategy.get_provider_config(keyword).is_some(),
                "missing config for {}",
                keyword
            );
        }
        assert!(strategy.get_provider_config("nonexistent").is_none());
    }
}
