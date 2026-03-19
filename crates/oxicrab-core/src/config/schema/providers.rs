use serde::{Deserialize, Serialize};
use std::borrow::Cow;

use super::default_true;

// ---------------------------------------------------------------------------
// Model reference parser — pure string parsing, no provider instantiation
// ---------------------------------------------------------------------------

/// Parsed model reference: optional provider prefix + bare model name.
pub struct ModelRef<'a> {
    pub provider: Option<&'a str>,
    pub model: &'a str,
}

/// Known provider prefixes recognized in `provider/model` notation.
const KNOWN_PREFIXES: &[&str] = &[
    "anthropic",
    "openai",
    "gemini",
    "openrouter",
    "deepseek",
    "groq",
    "minimax",
    "moonshot",
    "zhipu",
    "dashscope",
    "vllm",
    "ollama",
];

/// Parse `"provider/model"` notation. Returns `provider=None` if there is no
/// slash or if the part before the slash isn't a recognized provider prefix.
///
/// This prevents `meta-llama/Llama-3.3-70B` from being incorrectly split,
/// since `meta-llama` is not a known provider.
pub fn parse_model_ref(raw: &str) -> ModelRef<'_> {
    if let Some(idx) = raw.find('/') {
        let candidate = &raw[..idx];
        let candidate_lower = candidate.to_lowercase();
        if KNOWN_PREFIXES.contains(&candidate_lower.as_str()) && idx + 1 < raw.len() {
            return ModelRef {
                provider: Some(candidate),
                model: &raw[idx + 1..],
            };
        }
    }
    ModelRef {
        provider: None,
        model: raw,
    }
}

/// Infer the provider from a bare model name using `starts_with` patterns.
///
/// This is the convenience fallback — only matches well-known model name
/// prefixes. Returns `None` for unrecognized names (caller should error).
pub fn infer_provider_from_model(model: &str) -> Option<&'static str> {
    let m = model.to_lowercase();
    if m.starts_with("claude-") || m.starts_with("claude_") {
        return Some("anthropic");
    }
    if m.starts_with("gpt-")
        || m == "o1"
        || m.starts_with("o1-")
        || m == "o3"
        || m.starts_with("o3-")
        || m == "o4"
        || m.starts_with("o4-")
    {
        return Some("openai");
    }
    if m.starts_with("gemini") {
        return Some("gemini");
    }
    if m.starts_with("deepseek") {
        return Some("deepseek");
    }
    None
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
    #[serde(default, rename = "apiBase")]
    pub api_base: Option<String>,
    /// Custom HTTP headers injected into every request to this provider.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub headers: std::collections::HashMap<String, String>,
    /// Per-provider temperature override. When set, this takes priority over
    /// the global `agents.defaults.temperature` for all requests to this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
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
            .field("temperature", &self.temperature)
            .finish()
    }
}

/// Extended provider config for local inference servers (ollama, vllm)
/// that may need prompt-guided tool calling.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct LocalProviderConfig {
    #[serde(flatten)]
    pub base: ProviderConfig,
    /// Enable prompt-guided tool calling for local models that don't support
    /// native function calling. Injects tool definitions into the system prompt
    /// and parses `<tool_call>` XML blocks from text responses.
    #[serde(default, rename = "promptGuidedTools")]
    pub prompt_guided_tools: bool,
}

impl std::fmt::Debug for LocalProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redacted_headers: std::collections::HashMap<&String, &str> = self
            .base
            .headers
            .keys()
            .map(|k| (k, "[REDACTED]"))
            .collect();
        f.debug_struct("LocalProviderConfig")
            .field(
                "api_key",
                &if self.base.api_key.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field("api_base", &self.base.api_base)
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
    pub vllm: LocalProviderConfig,
    #[serde(default)]
    pub gemini: ProviderConfig,
    #[serde(default)]
    pub minimax: ProviderConfig,
    #[serde(default)]
    pub moonshot: ProviderConfig,
    #[serde(default)]
    pub ollama: LocalProviderConfig,
    #[serde(default, rename = "circuitBreaker")]
    pub circuit_breaker: CircuitBreakerConfig,
}

impl ProvidersConfig {
    /// Get the API key for a given model by resolving the provider name.
    ///
    /// Uses the same 2-tier resolution as `ProviderFactory`: prefix notation,
    /// model-name inference, then fallback to first available key.
    pub fn get_api_key(&self, model: &str) -> Option<&str> {
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
            "minimax" => &self.minimax,
            "moonshot" => &self.moonshot,
            "zhipu" => &self.zhipu,
            "dashscope" => &self.dashscope,
            "vllm" => &self.vllm.base,
            "ollama" => &self.ollama.base,
            _ => return None,
        };
        if config.api_key.is_empty() {
            None
        } else {
            Some(&config.api_key)
        }
    }

    /// Get the per-provider temperature override for a given model.
    ///
    /// Uses the same 2-tier resolution as `get_api_key`: prefix notation,
    /// then model-name inference.
    pub fn get_temperature_for_model(&self, model: &str) -> Option<f32> {
        let model_ref = parse_model_ref(model);
        let provider_name = model_ref
            .provider
            .or_else(|| infer_provider_from_model(model_ref.model));

        if let Some(name) = provider_name {
            let normalized = normalize_provider(name);
            let config = match normalized.as_ref() {
                "anthropic" => Some(&self.anthropic),
                "openai" => Some(&self.openai),
                "gemini" => Some(&self.gemini),
                "openrouter" => Some(&self.openrouter),
                "deepseek" => Some(&self.deepseek),
                "groq" => Some(&self.groq),
                "minimax" => Some(&self.minimax),
                "moonshot" => Some(&self.moonshot),
                "zhipu" => Some(&self.zhipu),
                "dashscope" => Some(&self.dashscope),
                "vllm" => Some(&self.vllm.base),
                "ollama" => Some(&self.ollama.base),
                _ => None,
            };
            if let Some(cfg) = config {
                return cfg.temperature;
            }
        }

        None
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
        "minimax" => Cow::Borrowed("minimax"),
        "moonshot" => Cow::Borrowed("moonshot"),
        "zhipu" => Cow::Borrowed("zhipu"),
        "dashscope" => Cow::Borrowed("dashscope"),
        "vllm" => Cow::Borrowed("vllm"),
        "ollama" => Cow::Borrowed("ollama"),
        _ => Cow::Owned(provider.to_lowercase()),
    }
}
