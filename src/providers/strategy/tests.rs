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
