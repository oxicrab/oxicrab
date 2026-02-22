use serde::{Deserialize, Serialize};
use std::borrow::Cow;

use super::default_true;

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
    #[serde(default, rename = "apiBase")]
    pub api_base: Option<String>,
    /// Custom HTTP headers injected into every request to this provider.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub headers: std::collections::HashMap<String, String>,
    /// Enable prompt-guided tool calling for local models that don't support
    /// native function calling. Injects tool definitions into the system prompt
    /// and parses `<tool_call>` XML blocks from text responses.
    #[serde(default, rename = "promptGuidedTools")]
    pub prompt_guided_tools: bool,
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redacted_headers: std::collections::HashMap<&String, &str> =
            self.headers.keys().map(|k| (k, "[REDACTED]")).collect();
        f.debug_struct("ProviderConfig")
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field("api_base", &self.api_base)
            .field("headers", &redacted_headers)
            .field("prompt_guided_tools", &self.prompt_guided_tools)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AnthropicOAuthConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "accessToken")]
    pub access_token: String,
    #[serde(default, rename = "refreshToken")]
    pub refresh_token: String,
    #[serde(default, rename = "expiresAt")]
    pub expires_at: i64,
    #[serde(default, rename = "credentialsPath")]
    pub credentials_path: Option<String>,
    #[serde(default = "default_true", rename = "autoDetect")]
    pub auto_detect: bool,
}

redact_debug!(
    AnthropicOAuthConfig,
    enabled,
    redact(access_token),
    redact(refresh_token),
    expires_at,
    credentials_path,
    auto_detect,
);

impl Default for AnthropicOAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            access_token: String::new(),
            refresh_token: String::new(),
            expires_at: 0,
            credentials_path: None,
            auto_detect: true,
        }
    }
}

fn default_failure_threshold() -> u32 {
    5
}

fn default_recovery_timeout_secs() -> u64 {
    60
}

fn default_half_open_probes() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_failure_threshold", rename = "failureThreshold")]
    pub failure_threshold: u32,
    #[serde(
        default = "default_recovery_timeout_secs",
        rename = "recoveryTimeoutSecs"
    )]
    pub recovery_timeout_secs: u64,
    #[serde(default = "default_half_open_probes", rename = "halfOpenProbes")]
    pub half_open_probes: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            failure_threshold: default_failure_threshold(),
            recovery_timeout_secs: default_recovery_timeout_secs(),
            half_open_probes: default_half_open_probes(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvidersConfig {
    #[serde(default)]
    pub anthropic: ProviderConfig,
    #[serde(default, rename = "anthropicOAuth")]
    pub anthropic_oauth: AnthropicOAuthConfig,
    #[serde(default)]
    pub openai: ProviderConfig,
    #[serde(default)]
    pub openrouter: ProviderConfig,
    #[serde(default)]
    pub deepseek: ProviderConfig,
    #[serde(default)]
    pub groq: ProviderConfig,
    #[serde(default)]
    pub zhipu: ProviderConfig,
    #[serde(default)]
    pub dashscope: ProviderConfig,
    #[serde(default)]
    pub vllm: ProviderConfig,
    #[serde(default)]
    pub gemini: ProviderConfig,
    #[serde(default)]
    pub moonshot: ProviderConfig,
    #[serde(default)]
    pub ollama: ProviderConfig,
    #[serde(default, rename = "circuitBreaker")]
    pub circuit_breaker: CircuitBreakerConfig,
}

impl ProvidersConfig {
    /// Get the API key for a given model by resolving the provider name.
    ///
    /// Uses the same 3-tier resolution as `ProviderFactory`: explicit prefix,
    /// model-name inference, then fallback to first available key.
    pub fn get_api_key(&self, model: &str) -> Option<&str> {
        use crate::providers::strategy::{infer_provider_from_model, parse_model_ref};

        let model_ref = parse_model_ref(model);
        let provider_name = model_ref
            .provider
            .or_else(|| infer_provider_from_model(model_ref.model));

        if let Some(name) = provider_name {
            let normalized = normalize_provider(name);
            if let Some(key) = self.get_api_key_for_provider(&normalized) {
                return Some(key);
            }
        }

        // Fallback: first available key
        self.first_available_key()
    }

    /// Get the API key for a specific provider by canonical name.
    pub fn get_api_key_for_provider(&self, provider: &str) -> Option<&str> {
        let normalized = normalize_provider(provider);
        let config = match normalized.as_ref() {
            "anthropic" => &self.anthropic,
            "openai" => &self.openai,
            "gemini" => &self.gemini,
            "openrouter" => &self.openrouter,
            "deepseek" => &self.deepseek,
            "groq" => &self.groq,
            "moonshot" => &self.moonshot,
            "zhipu" => &self.zhipu,
            "dashscope" => &self.dashscope,
            "vllm" => &self.vllm,
            "ollama" => &self.ollama,
            _ => return None,
        };
        if config.api_key.is_empty() {
            None
        } else {
            Some(&config.api_key)
        }
    }

    /// Return the first available API key across all providers.
    fn first_available_key(&self) -> Option<&str> {
        for config in [
            &self.openrouter,
            &self.anthropic,
            &self.openai,
            &self.gemini,
        ] {
            if !config.api_key.is_empty() {
                return Some(&config.api_key);
            }
        }
        None
    }
}

/// Normalize provider aliases to canonical names.
///
/// Maps common aliases (e.g. "claude" → "anthropic", "gpt" → "openai")
/// to the canonical provider name used for routing. Unknown values pass through.
pub fn normalize_provider(provider: &str) -> Cow<'_, str> {
    match provider.to_lowercase().as_str() {
        "claude" | "anthropic" => Cow::Borrowed("anthropic"),
        "gpt" | "openai" => Cow::Borrowed("openai"),
        "google" | "gemini" => Cow::Borrowed("gemini"),
        "openrouter" => Cow::Borrowed("openrouter"),
        "deepseek" => Cow::Borrowed("deepseek"),
        "groq" => Cow::Borrowed("groq"),
        "moonshot" => Cow::Borrowed("moonshot"),
        "zhipu" => Cow::Borrowed("zhipu"),
        "dashscope" => Cow::Borrowed("dashscope"),
        "vllm" => Cow::Borrowed("vllm"),
        "ollama" => Cow::Borrowed("ollama"),
        _ => Cow::Owned(provider.to_lowercase()),
    }
}
