use crate::config::schema::{
    AnthropicOAuthConfig, ProviderConfig, ProvidersConfig, normalize_provider,
};
use crate::providers::base::LLMProvider;
use anyhow::Result;
use std::sync::Arc;
use tracing::info;

/// Default base URLs for OpenAI-compatible providers.
const OPENAI_COMPAT_URLS: &[(&str, &str)] = &[
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
    if m.starts_with("openrouter/") {
        return Some("openrouter");
    }
    None
}

// ---------------------------------------------------------------------------
// Provider factory — 3-tier resolution
// ---------------------------------------------------------------------------

/// Provider factory using 3-tier resolution:
///
/// 1. **Explicit provider** — `agents.defaults.provider` in config or `--provider` CLI flag
/// 2. **Prefix notation** — `provider/model` syntax (e.g. `groq/llama-3.1-70b`)
/// 3. **Model-name inference** — known prefixes like `claude-*` → Anthropic
pub struct ProviderFactory {
    providers_config: ProvidersConfig,
    oauth_config: AnthropicOAuthConfig,
    explicit_provider: Option<String>,
}

impl ProviderFactory {
    pub fn new(config: &crate::config::schema::Config) -> Self {
        Self {
            providers_config: config.providers.clone(),
            oauth_config: config.providers.anthropic_oauth.clone(),
            explicit_provider: config.agents.defaults.provider.clone(),
        }
    }

    pub fn create_provider(&self, model: &str) -> Result<Arc<dyn LLMProvider>> {
        // Step 1: Parse model reference for prefix notation
        let model_ref = parse_model_ref(model);

        // Step 2: Determine effective provider (explicit > prefix > inference)
        let effective_provider = self.explicit_provider.as_deref().or(model_ref.provider);
        let bare_model = model_ref.model;

        // Step 3: If we have an explicit/prefix provider, route directly
        if let Some(provider_name) = effective_provider {
            return self.create_for_provider(provider_name, bare_model);
        }

        // Step 4: Convenience fallback — infer from model name patterns
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
                if !self.providers_config.anthropic.api_key.is_empty() {
                    info!("using Anthropic API key provider for model: {}", model);
                    return Ok(Arc::new(AnthropicProvider::new(
                        self.providers_config.anthropic.api_key.clone(),
                        Some(model.to_string()),
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
                if !self.providers_config.openai.api_key.is_empty() {
                    info!("using OpenAI provider for model: {}", model);
                    return Ok(Arc::new(OpenAIProvider::new(
                        self.providers_config.openai.api_key.clone(),
                        Some(model.to_string()),
                    )));
                }
                anyhow::bail!("no OpenAI API key configured for model: {model}")
            }
            "gemini" => {
                if !self.providers_config.gemini.api_key.is_empty() {
                    info!("using Gemini provider for model: {}", model);
                    return Ok(Arc::new(GeminiProvider::new(
                        self.providers_config.gemini.api_key.clone(),
                        Some(model.to_string()),
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
            return Ok(Some(Arc::new(AnthropicOAuthProvider::new(
                self.oauth_config.access_token.clone(),
                self.oauth_config.refresh_token.clone(),
                self.oauth_config.expires_at,
                Some(model.to_string()),
                self.oauth_config
                    .credentials_path
                    .as_ref()
                    .map(std::path::PathBuf::from),
            )?)));
        }

        // Try Claude CLI auto-detection
        if let Ok(Some(provider)) = AnthropicOAuthProvider::from_claude_cli(Some(model.to_string()))
        {
            return Ok(Some(Arc::new(provider)));
        }

        // Try OpenClaw auto-detection
        if let Ok(Some(provider)) = AnthropicOAuthProvider::from_openclaw(Some(model.to_string())) {
            return Ok(Some(Arc::new(provider)));
        }

        // Try credentials file
        if let Some(ref path) = self.oauth_config.credentials_path {
            let path_buf = std::path::PathBuf::from(path);
            if let Ok(Some(provider)) =
                AnthropicOAuthProvider::from_credentials_file(&path_buf, Some(model.to_string()))
            {
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
            "moonshot" => Some(&self.providers_config.moonshot),
            "zhipu" => Some(&self.providers_config.zhipu),
            "dashscope" => Some(&self.providers_config.dashscope),
            "vllm" => Some(&self.providers_config.vllm),
            "ollama" => Some(&self.providers_config.ollama),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::ProvidersConfig;

    // -----------------------------------------------------------------------
    // parse_model_ref tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_model_ref_with_known_provider() {
        let r = parse_model_ref("groq/llama-3.1-70b");
        assert_eq!(r.provider, Some("groq"));
        assert_eq!(r.model, "llama-3.1-70b");
    }

    #[test]
    fn test_parse_model_ref_no_slash() {
        let r = parse_model_ref("claude-sonnet-4-5-20250929");
        assert!(r.provider.is_none());
        assert_eq!(r.model, "claude-sonnet-4-5-20250929");
    }

    #[test]
    fn test_parse_model_ref_unknown_prefix() {
        // meta-llama is not a known provider — should NOT split
        let r = parse_model_ref("meta-llama/Llama-3.3-70B");
        assert!(r.provider.is_none());
        assert_eq!(r.model, "meta-llama/Llama-3.3-70B");
    }

    #[test]
    fn test_parse_model_ref_ollama_prefix() {
        let r = parse_model_ref("ollama/qwen3-coder:30b");
        assert_eq!(r.provider, Some("ollama"));
        assert_eq!(r.model, "qwen3-coder:30b");
    }

    #[test]
    fn test_parse_model_ref_anthropic_prefix() {
        let r = parse_model_ref("anthropic/claude-opus-4-6");
        assert_eq!(r.provider, Some("anthropic"));
        assert_eq!(r.model, "claude-opus-4-6");
    }

    #[test]
    fn test_parse_model_ref_trailing_slash() {
        // "anthropic/" should NOT produce an empty model name
        let r = parse_model_ref("anthropic/");
        assert!(r.provider.is_none());
        assert_eq!(r.model, "anthropic/");
    }

    #[test]
    fn test_parse_model_ref_case_insensitive() {
        let r = parse_model_ref("Anthropic/claude-opus-4-6");
        assert_eq!(r.provider, Some("Anthropic"));
        assert_eq!(r.model, "claude-opus-4-6");
    }

    // -----------------------------------------------------------------------
    // infer_provider_from_model tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_infer_provider_claude() {
        assert_eq!(
            infer_provider_from_model("claude-sonnet-4-5-20250929"),
            Some("anthropic")
        );
    }

    #[test]
    fn test_infer_provider_gpt() {
        assert_eq!(infer_provider_from_model("gpt-4o"), Some("openai"));
    }

    #[test]
    fn test_infer_provider_o1() {
        assert_eq!(infer_provider_from_model("o1-preview"), Some("openai"));
    }

    #[test]
    fn test_infer_provider_o3() {
        assert_eq!(infer_provider_from_model("o3-mini"), Some("openai"));
    }

    #[test]
    fn test_infer_provider_deepseek() {
        assert_eq!(infer_provider_from_model("deepseek-chat"), Some("deepseek"));
    }

    #[test]
    fn test_infer_provider_gemini() {
        assert_eq!(infer_provider_from_model("gemini-pro"), Some("gemini"));
    }

    #[test]
    fn test_infer_provider_unknown() {
        assert_eq!(infer_provider_from_model("my-custom-model"), None);
    }

    #[test]
    fn test_no_false_match_contains() {
        // "my-groq-benchmark" should NOT match — starts_with prevents this
        assert_eq!(infer_provider_from_model("my-groq-benchmark"), None);
        assert_eq!(infer_provider_from_model("my-deepseek-finetune"), None);
        assert_eq!(infer_provider_from_model("custom-claude-fork"), None);
    }

    // -----------------------------------------------------------------------
    // normalize_provider tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_normalize_provider_aliases() {
        assert_eq!(normalize_provider("claude").as_ref(), "anthropic");
        assert_eq!(normalize_provider("anthropic").as_ref(), "anthropic");
        assert_eq!(normalize_provider("gpt").as_ref(), "openai");
        assert_eq!(normalize_provider("openai").as_ref(), "openai");
        assert_eq!(normalize_provider("google").as_ref(), "gemini");
        assert_eq!(normalize_provider("gemini").as_ref(), "gemini");
        assert_eq!(normalize_provider("Anthropic").as_ref(), "anthropic");
    }

    #[test]
    fn test_normalize_provider_passthrough() {
        assert_eq!(normalize_provider("groq").as_ref(), "groq");
        assert_eq!(normalize_provider("ollama").as_ref(), "ollama");
        assert_eq!(normalize_provider("UnKnOwN").as_ref(), "unknown");
    }

    // -----------------------------------------------------------------------
    // ProviderFactory / create_for_provider tests
    // -----------------------------------------------------------------------

    fn config_with_deepseek() -> ProvidersConfig {
        let mut config = ProvidersConfig::default();
        config.deepseek.api_key = "sk-deepseek-test".to_string();
        config
    }

    fn factory_with_config(providers: &ProvidersConfig) -> ProviderFactory {
        ProviderFactory {
            providers_config: providers.clone(),
            oauth_config: providers.anthropic_oauth.clone(),
            explicit_provider: None,
        }
    }

    fn factory_with_explicit(providers: &ProvidersConfig, explicit: &str) -> ProviderFactory {
        ProviderFactory {
            providers_config: providers.clone(),
            oauth_config: providers.anthropic_oauth.clone(),
            explicit_provider: Some(explicit.to_string()),
        }
    }

    #[test]
    fn test_deepseek_routing() {
        let factory = factory_with_config(&config_with_deepseek());
        let provider = factory.create_provider("deepseek-chat").unwrap();
        assert_eq!(provider.default_model(), "deepseek-chat");
    }

    #[test]
    fn test_groq_prefix_routing() {
        let mut config = ProvidersConfig::default();
        config.groq.api_key = "gsk-test".to_string();
        let factory = factory_with_config(&config);
        let provider = factory.create_provider("groq/llama-3.1-70b").unwrap();
        assert_eq!(provider.default_model(), "llama-3.1-70b");
    }

    #[test]
    fn test_ollama_routing_no_api_key() {
        let factory = factory_with_config(&ProvidersConfig::default());
        let provider = factory.create_provider("ollama/qwen3-coder:30b").unwrap();
        assert_eq!(provider.default_model(), "qwen3-coder:30b");
    }

    #[test]
    fn test_ollama_custom_api_base() {
        let mut config = ProvidersConfig::default();
        config.ollama.api_base = Some("http://192.168.1.100:11434/v1/chat/completions".to_string());
        let factory = factory_with_config(&config);
        let provider = factory.create_provider("ollama/qwen3-coder:30b").unwrap();
        assert_eq!(provider.default_model(), "qwen3-coder:30b");
    }

    #[test]
    fn test_vllm_routing_no_api_key() {
        let factory = factory_with_config(&ProvidersConfig::default());
        let provider = factory.create_provider("vllm/my-model").unwrap();
        assert_eq!(provider.default_model(), "my-model");
    }

    #[test]
    fn test_deepseek_no_api_key_errors() {
        let factory = factory_with_config(&ProvidersConfig::default());
        let result = factory.create_provider("deepseek-chat");
        assert!(result.is_err());
    }

    #[test]
    fn test_explicit_provider_overrides_prefix() {
        let mut config = ProvidersConfig::default();
        config.anthropic.api_key = "sk-ant-test".to_string();
        // Model has groq/ prefix, but explicit provider says anthropic
        let factory = factory_with_explicit(&config, "anthropic");
        let provider = factory.create_provider("groq/some-model").unwrap();
        // Should use anthropic, not groq — model sent is bare "some-model"
        // (prefix still stripped by parse_model_ref, but provider is explicit)
        assert_eq!(provider.default_model(), "some-model");
    }

    #[test]
    fn test_explicit_provider_with_unprefixed_model() {
        let mut config = ProvidersConfig::default();
        config.anthropic.api_key = "sk-ant-test".to_string();
        let factory = factory_with_explicit(&config, "anthropic");
        let provider = factory.create_provider("my-custom-fine-tune").unwrap();
        assert_eq!(provider.default_model(), "my-custom-fine-tune");
    }

    #[test]
    fn test_unknown_model_no_provider_errors() {
        let factory = factory_with_config(&ProvidersConfig::default());
        let result = factory.create_provider("my-custom-model");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("no provider configured"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn test_native_providers_not_openai_compat() {
        let mut config = ProvidersConfig::default();
        config.anthropic.api_key = "sk-ant-test".to_string();
        let factory = factory_with_config(&config);
        let provider = factory
            .create_provider("claude-sonnet-4-5-20250929")
            .unwrap();
        assert_eq!(provider.default_model(), "claude-sonnet-4-5-20250929");
    }

    #[test]
    fn test_openai_routing() {
        let mut config = ProvidersConfig::default();
        config.openai.api_key = "sk-openai-test".to_string();
        let factory = factory_with_config(&config);
        let provider = factory.create_provider("gpt-4o").unwrap();
        assert_eq!(provider.default_model(), "gpt-4o");
    }

    #[test]
    fn test_gemini_routing() {
        let mut config = ProvidersConfig::default();
        config.gemini.api_key = "gm-test".to_string();
        let factory = factory_with_config(&config);
        let provider = factory.create_provider("gemini-pro").unwrap();
        assert_eq!(provider.default_model(), "gemini-pro");
    }

    #[test]
    fn test_get_provider_config_all_known() {
        let factory = factory_with_config(&ProvidersConfig::default());
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
                factory.get_provider_config(keyword).is_some(),
                "missing config for {}",
                keyword
            );
        }
        assert!(factory.get_provider_config("nonexistent").is_none());
    }

    #[test]
    fn test_meta_llama_with_explicit_ollama() {
        let factory = factory_with_explicit(&ProvidersConfig::default(), "ollama");
        let provider = factory.create_provider("meta-llama/Llama-3.3-70B").unwrap();
        assert_eq!(provider.default_model(), "meta-llama/Llama-3.3-70B");
    }

    #[test]
    fn test_provider_alias_claude() {
        let mut config = ProvidersConfig::default();
        config.anthropic.api_key = "sk-ant-test".to_string();
        let factory = factory_with_explicit(&config, "claude");
        let provider = factory.create_provider("my-model").unwrap();
        assert_eq!(provider.default_model(), "my-model");
    }
}
