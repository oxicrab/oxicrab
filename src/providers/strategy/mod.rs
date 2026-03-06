use crate::agent::memory::memory_db::MemoryDB;
use crate::config::schema::{
    AnthropicOAuthConfig, ProviderConfig, ProvidersConfig, normalize_provider,
};
use crate::providers::base::LLMProvider;
use anyhow::Result;
use std::sync::Arc;
use tracing::info;

/// Default API URL for first-party `Anthropic` provider.
const API_URL_ANTHROPIC: &str = "https://api.anthropic.com/v1/messages";
/// Default API URL for first-party `OpenAI` provider.
const API_URL_OPENAI: &str = "https://api.openai.com/v1/chat/completions";
/// Default base URL for first-party `Gemini` provider.
const BASE_URL_GEMINI: &str = "https://generativelanguage.googleapis.com/v1";

/// Default base URLs for OpenAI-compatible providers.
const OPENAI_COMPAT_URLS: &[(&str, &str)] = &[
    (
        "openrouter",
        "https://openrouter.ai/api/v1/chat/completions",
    ),
    ("deepseek", "https://api.deepseek.com/v1/chat/completions"),
    ("groq", "https://api.groq.com/openai/v1/chat/completions"),
    ("minimax", "https://api.minimax.io/v1/chat/completions"),
    ("moonshot", "https://api.moonshot.ai/v1/chat/completions"),
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

// ---------------------------------------------------------------------------
// Model reference parser
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
    if m.starts_with("gpt-") || m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4") {
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

// ---------------------------------------------------------------------------
// Provider factory — 2-tier resolution
// ---------------------------------------------------------------------------

/// Provider factory using 2-tier resolution:
///
/// 1. **Prefix notation** — `provider/model` syntax (e.g. `groq/llama-3.1-70b`)
/// 2. **Model-name inference** — known prefixes like `claude-*` → Anthropic
pub struct ProviderFactory {
    providers_config: ProvidersConfig,
    oauth_config: AnthropicOAuthConfig,
    db: Option<Arc<MemoryDB>>,
}

impl ProviderFactory {
    pub fn new(config: &crate::config::schema::Config) -> Self {
        Self {
            providers_config: config.providers.clone(),
            oauth_config: config.providers.anthropic_oauth.clone(),
            db: None,
        }
    }

    pub fn with_db(config: &crate::config::schema::Config, db: Option<Arc<MemoryDB>>) -> Self {
        Self {
            providers_config: config.providers.clone(),
            oauth_config: config.providers.anthropic_oauth.clone(),
            db,
        }
    }

    pub fn create_provider(&self, model: &str) -> Result<Arc<dyn LLMProvider>> {
        // Step 1: Parse model reference for prefix notation
        let model_ref = parse_model_ref(model);
        let bare_model = model_ref.model;

        // Step 2: If prefix notation matched, route directly
        if let Some(provider_name) = model_ref.provider {
            return self.create_for_provider(provider_name, bare_model);
        }

        // Step 3: Convenience fallback — infer from model name patterns
        if let Some(inferred) = infer_provider_from_model(bare_model) {
            return self.create_for_provider(inferred, bare_model);
        }

        anyhow::bail!("no provider configured for model: {model}")
    }

    /// Create a provider instance by canonical provider name.
    fn create_for_provider(&self, provider: &str, model: &str) -> Result<Arc<dyn LLMProvider>> {
        use crate::providers::{
            anthropic::AnthropicProvider, gemini::GeminiProvider, openai::OpenAIProvider,
        };

        let normalized = normalize_provider(provider);
        match normalized.as_ref() {
            "anthropic" => {
                // Try OAuth first, then API key
                if let Some(oauth) = self.try_anthropic_oauth(model)? {
                    return Ok(oauth);
                }
                let cfg = &self.providers_config.anthropic;
                if !cfg.api_key.is_empty() {
                    info!("using Anthropic API key provider for model: {}", model);
                    if cfg.headers.is_empty() && cfg.api_base.is_none() {
                        return Ok(Arc::new(AnthropicProvider::new(
                            cfg.api_key.clone(),
                            Some(model.to_string()),
                        )));
                    }
                    let base_url = cfg.api_base.as_deref().unwrap_or(API_URL_ANTHROPIC);
                    return Ok(Arc::new(AnthropicProvider::with_config(
                        cfg.api_key.clone(),
                        Some(model.to_string()),
                        base_url.to_string(),
                        cfg.headers.clone(),
                    )));
                }
                anyhow::bail!(
                    "no Anthropic credentials configured for model: {model}\n\
                     \n\
                     Options:\n\
                     1. Set providers.anthropic.apiKey in ~/.oxicrab/config.json\n\
                     2. Install Claude CLI for OAuth auto-detection\n\
                     3. Install OpenClaw for OAuth auto-detection"
                )
            }
            "openai" => {
                let cfg = &self.providers_config.openai;
                if !cfg.api_key.is_empty() {
                    info!("using OpenAI provider for model: {}", model);
                    if cfg.headers.is_empty() && cfg.api_base.is_none() {
                        return Ok(Arc::new(OpenAIProvider::new(
                            cfg.api_key.clone(),
                            Some(model.to_string()),
                        )));
                    }
                    let base_url = cfg.api_base.as_deref().unwrap_or(API_URL_OPENAI);
                    return Ok(Arc::new(OpenAIProvider::with_config_and_headers(
                        cfg.api_key.clone(),
                        model.to_string(),
                        base_url.to_string(),
                        "OpenAI".to_string(),
                        cfg.headers.clone(),
                    )));
                }
                anyhow::bail!("no OpenAI API key configured for model: {model}")
            }
            "gemini" => {
                let cfg = &self.providers_config.gemini;
                if !cfg.api_key.is_empty() {
                    info!("using Gemini provider for model: {}", model);
                    if cfg.headers.is_empty() && cfg.api_base.is_none() {
                        return Ok(Arc::new(GeminiProvider::new(
                            cfg.api_key.clone(),
                            Some(model.to_string()),
                        )));
                    }
                    let base_url = cfg.api_base.as_deref().unwrap_or(BASE_URL_GEMINI);
                    return Ok(Arc::new(GeminiProvider::with_config(
                        cfg.api_key.clone(),
                        Some(model.to_string()),
                        base_url.to_string(),
                        cfg.headers.clone(),
                    )));
                }
                anyhow::bail!("no Gemini API key configured for model: {model}")
            }
            // OpenAI-compatible providers
            _ => self.create_openai_compat(&normalized, model),
        }
    }

    /// Try to create an Anthropic OAuth provider.
    fn try_anthropic_oauth(&self, model: &str) -> Result<Option<Arc<dyn LLMProvider>>> {
        use crate::providers::anthropic_oauth::AnthropicOAuthProvider;

        let should_try = self.oauth_config.enabled || self.oauth_config.auto_detect;
        if !should_try {
            return Ok(None);
        }

        // Try explicit config first
        if !self.oauth_config.access_token.is_empty() {
            info!(
                "using Anthropic OAuth provider (explicit config) for model: {}",
                model
            );
            return Ok(Some(Arc::new(AnthropicOAuthProvider::new(
                self.oauth_config.access_token.clone(),
                self.oauth_config.refresh_token.clone(),
                self.oauth_config.expires_at,
                Some(model.to_string()),
                self.oauth_config
                    .credentials_path
                    .as_ref()
                    .map(std::path::PathBuf::from),
                self.db.clone(),
            )?)));
        }

        // Try Claude CLI auto-detection
        if let Ok(Some(provider)) =
            AnthropicOAuthProvider::from_claude_cli(Some(model.to_string()), self.db.clone())
        {
            info!(
                "using Anthropic OAuth provider (Claude CLI auto-detect) for model: {}",
                model
            );
            return Ok(Some(Arc::new(provider)));
        }

        // Try OpenClaw auto-detection
        if let Ok(Some(provider)) =
            AnthropicOAuthProvider::from_openclaw(Some(model.to_string()), self.db.clone())
        {
            info!(
                "using Anthropic OAuth provider (OpenClaw auto-detect) for model: {}",
                model
            );
            return Ok(Some(Arc::new(provider)));
        }

        // Try credentials file
        if let Some(ref path) = self.oauth_config.credentials_path {
            let path_buf = std::path::PathBuf::from(path);
            if let Ok(Some(provider)) =
                AnthropicOAuthProvider::from_credentials_file(&path_buf, Some(model.to_string()))
            {
                info!(
                    "using Anthropic OAuth provider (credentials file) for model: {}",
                    model
                );
                return Ok(Some(Arc::new(provider)));
            }
        }

        Ok(None)
    }

    /// Create an OpenAI-compatible provider for the given canonical provider name.
    fn create_openai_compat(
        &self,
        provider_name: &str,
        model: &str,
    ) -> Result<Arc<dyn LLMProvider>> {
        use crate::providers::openai::OpenAIProvider;

        let provider_config = self
            .get_provider_config(provider_name)
            .ok_or_else(|| anyhow::anyhow!("unknown provider: {provider_name}"))?;

        let default_url = OPENAI_COMPAT_URLS
            .iter()
            .find(|&&(k, _)| k == provider_name)
            .map_or("http://localhost:8000/v1/chat/completions", |&(_, url)| url);

        let is_local = matches!(provider_name, "ollama" | "vllm");
        if provider_config.api_key.is_empty() && !is_local {
            anyhow::bail!("no API key configured for provider: {provider_name}");
        }

        let base_url = provider_config
            .api_base
            .as_deref()
            .unwrap_or(default_url)
            .to_string();

        let display_name = provider_name[..1].to_uppercase() + &provider_name[1..];

        info!(
            "using OpenAI-compatible provider ({}) for model: {}",
            display_name, model
        );

        if provider_config.headers.is_empty() {
            Ok(Arc::new(OpenAIProvider::with_config(
                provider_config.api_key.clone(),
                model.to_string(),
                base_url,
                display_name,
            )))
        } else {
            Ok(Arc::new(OpenAIProvider::with_config_and_headers(
                provider_config.api_key.clone(),
                model.to_string(),
                base_url,
                display_name,
                provider_config.headers.clone(),
            )))
        }
    }

    /// Look up the `ProviderConfig` for a given canonical provider name.
    fn get_provider_config(&self, provider: &str) -> Option<&ProviderConfig> {
        match provider {
            "openrouter" => Some(&self.providers_config.openrouter),
            "deepseek" => Some(&self.providers_config.deepseek),
            "groq" => Some(&self.providers_config.groq),
            "minimax" => Some(&self.providers_config.minimax),
            "moonshot" => Some(&self.providers_config.moonshot),
            "zhipu" => Some(&self.providers_config.zhipu),
            "dashscope" => Some(&self.providers_config.dashscope),
            "vllm" => Some(&self.providers_config.vllm.base),
            "ollama" => Some(&self.providers_config.ollama.base),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
