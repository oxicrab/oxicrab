use crate::config::schema::{AnthropicOAuthConfig, ProvidersConfig};
use crate::providers::base::LLMProvider;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

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
            || (model.to_lowercase().contains("anthropic") || model.to_lowercase().contains("claude"))
    }

    async fn create_provider(&self, model: &str) -> Result<Option<Arc<dyn LLMProvider>>> {
        use crate::providers::anthropic_oauth::AnthropicOAuthProvider;

        let should_try_oauth = model.starts_with("anthropic/")
            || self.config.enabled
            || self.config.auto_detect;

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
                    .map(|p| std::path::PathBuf::from(p)),
            )?)));
        }

        // Try auto-detection
        if model.starts_with("anthropic/") || self.config.auto_detect {
            // Try Claude CLI
            if let Ok(Some(provider)) =
                AnthropicOAuthProvider::from_claude_cli(Some(model.to_string())).await
            {
                return Ok(Some(Arc::new(provider)));
            }

            // Try OpenClaw
            if let Ok(Some(provider)) =
                AnthropicOAuthProvider::from_openclaw(Some(model.to_string())).await
            {
                return Ok(Some(Arc::new(provider)));
            }

            // Try credentials file
            if let Some(ref path) = self.config.credentials_path {
                let path_buf = std::path::PathBuf::from(path);
                if let Ok(Some(provider)) =
                    AnthropicOAuthProvider::from_credentials_file(&path_buf, Some(model.to_string())).await
                {
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
        use crate::providers::{anthropic::AnthropicProvider, gemini::GeminiProvider, openai::OpenAIProvider};

        let model_lower = model.to_lowercase();

        // For Claude models, try Anthropic API key directly
        if (model_lower.contains("anthropic") || model_lower.contains("claude"))
            && !self.config.anthropic.api_key.is_empty()
        {
            tracing::info!("Using Anthropic API key provider for model: {}", model);
            return Ok(Some(Arc::new(AnthropicProvider::new(
                self.config.anthropic.api_key.clone(),
                Some(model.to_string()),
            ))));
        }

        // Get API key for model
        let api_key = self.get_api_key(model);
        let model_str = model.to_string();

        if let Some(key) = api_key {
            tracing::info!("Using API key provider for model: {}", model);
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
        let model_lower = model.to_lowercase();

        // Match provider by model name
        if model_lower.contains("openrouter") && !self.config.openrouter.api_key.is_empty() {
            return Some(self.config.openrouter.api_key.clone());
        }
        if model_lower.contains("deepseek") && !self.config.deepseek.api_key.is_empty() {
            return Some(self.config.deepseek.api_key.clone());
        }
        if (model_lower.contains("anthropic") || model_lower.contains("claude"))
            && !self.config.anthropic.api_key.is_empty()
        {
            return Some(self.config.anthropic.api_key.clone());
        }
        if (model_lower.contains("openai") || model_lower.contains("gpt"))
            && !self.config.openai.api_key.is_empty()
        {
            return Some(self.config.openai.api_key.clone());
        }
        if model_lower.contains("gemini") && !self.config.gemini.api_key.is_empty() {
            return Some(self.config.gemini.api_key.clone());
        }

        // Fallback: first available key
        if !self.config.openrouter.api_key.is_empty() {
            return Some(self.config.openrouter.api_key.clone());
        }
        if !self.config.anthropic.api_key.is_empty() {
            return Some(self.config.anthropic.api_key.clone());
        }
        if !self.config.openai.api_key.is_empty() {
            return Some(self.config.openai.api_key.clone());
        }
        if !self.config.gemini.api_key.is_empty() {
            return Some(self.config.gemini.api_key.clone());
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
        let mut strategies: Vec<Box<dyn ProviderStrategy>> = Vec::new();

        // Add OAuth strategy first (higher priority)
        strategies.push(Box::new(AnthropicOAuthStrategy::new(
            config.providers.anthropic_oauth.clone(),
        )));

        // Add API key strategy as fallback
        strategies.push(Box::new(ApiKeyProviderStrategy::new(
            config.providers.clone(),
        )));

        Self { strategies }
    }

    pub async fn create_provider(
        &self,
        model: &str,
    ) -> Result<Arc<dyn LLMProvider>> {
        // Try each strategy in order
        for strategy in &self.strategies {
            if strategy.can_handle(model) {
                if let Some(provider) = strategy.create_provider(model).await? {
                    return Ok(provider);
                }
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
                3. Set 'providers.anthropicOAuth.credentialsPath' in ~/.nanobot/config.json\n\
                4. Use an API key model instead (e.g., 'claude-sonnet-4-5-20250929')",
                model
            );
        }

        anyhow::bail!("No API key configured for model: {}", model);
    }
}
