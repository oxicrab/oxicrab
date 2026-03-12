use crate::providers::base::{ChatRequest, LLMProvider, Message};
use std::sync::Arc;
use tracing::{error, info, warn};

/// A provider+model pair to verify, with a human-readable label.
pub struct ProviderEntry {
    pub provider: Arc<dyn LLMProvider>,
    pub model: String,
    pub label: String,
}

/// Result of verifying a single provider.
pub struct VerifyResult {
    pub model: String,
    pub label: String,
    pub success: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// Send a minimal 1-token request to check if a provider+model is reachable.
pub async fn verify_provider(provider: &Arc<dyn LLMProvider>, model: &str) -> VerifyResult {
    let start = std::time::Instant::now();
    let req = ChatRequest {
        messages: vec![Message::user("hi")],
        model: Some(model.to_string()),
        max_tokens: 1,
        temperature: Some(0.0),
        ..Default::default()
    };

    let result = tokio::time::timeout(std::time::Duration::from_secs(30), provider.chat(req)).await;

    let latency_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(Ok(_)) => VerifyResult {
            model: model.to_string(),
            label: String::new(),
            success: true,
            latency_ms,
            error: None,
        },
        Ok(Err(e)) => VerifyResult {
            model: model.to_string(),
            label: String::new(),
            success: false,
            latency_ms,
            error: Some(e.to_string()),
        },
        Err(_) => VerifyResult {
            model: model.to_string(),
            label: String::new(),
            success: false,
            latency_ms,
            error: Some("timed out after 30s".to_string()),
        },
    }
}

/// Verify all providers in parallel, returning results in the same order.
pub async fn verify_all_providers(entries: &[ProviderEntry]) -> Vec<VerifyResult> {
    let futures: Vec<_> = entries
        .iter()
        .map(|entry| {
            let provider = entry.provider.clone();
            let model = entry.model.clone();
            let label = entry.label.clone();
            async move {
                let mut result = verify_provider(&provider, &model).await;
                result.label = label;
                result
            }
        })
        .collect();

    futures_util::future::join_all(futures).await
}

/// Log verification results and return whether all passed.
pub fn log_verify_results(results: &[VerifyResult]) -> bool {
    if results.is_empty() {
        warn!("no models to verify");
        return true;
    }
    let mut all_ok = true;
    for r in results {
        if r.success {
            info!(
                "model check passed: {} ({}) — {}ms",
                r.model, r.label, r.latency_ms
            );
        } else {
            all_ok = false;
            error!(
                "model check FAILED: {} ({}) — {}",
                r.model,
                r.label,
                r.error.as_deref().unwrap_or("unknown error")
            );
        }
    }
    if all_ok && !results.is_empty() {
        info!("all {} model(s) verified successfully", results.len());
    }
    all_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::base::LLMResponse;
    use async_trait::async_trait;

    struct OkProvider;

    #[async_trait]
    impl LLMProvider for OkProvider {
        async fn chat(&self, _req: ChatRequest) -> anyhow::Result<LLMResponse> {
            Ok(LLMResponse::default())
        }
        fn default_model(&self) -> &'static str {
            "test-model"
        }
    }

    struct FailProvider;

    #[async_trait]
    impl LLMProvider for FailProvider {
        async fn chat(&self, _req: ChatRequest) -> anyhow::Result<LLMResponse> {
            Err(anyhow::anyhow!("connection refused"))
        }
        fn default_model(&self) -> &'static str {
            "fail-model"
        }
    }

    #[tokio::test]
    async fn test_verify_provider_ok() {
        let provider: Arc<dyn LLMProvider> = Arc::new(OkProvider);
        let result = verify_provider(&provider, "test-model").await;
        assert!(result.success);
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn test_verify_provider_fail() {
        let provider: Arc<dyn LLMProvider> = Arc::new(FailProvider);
        let result = verify_provider(&provider, "fail-model").await;
        assert!(!result.success);
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("connection refused"));
    }

    #[tokio::test]
    async fn test_verify_all_providers() {
        let ok: Arc<dyn LLMProvider> = Arc::new(OkProvider);
        let fail: Arc<dyn LLMProvider> = Arc::new(FailProvider);
        let entries = vec![
            ProviderEntry {
                provider: ok,
                model: "good-model".to_string(),
                label: "primary".to_string(),
            },
            ProviderEntry {
                provider: fail,
                model: "bad-model".to_string(),
                label: "fallback".to_string(),
            },
        ];
        let results = verify_all_providers(&entries).await;
        assert_eq!(results.len(), 2);
        assert!(results[0].success);
        assert!(!results[1].success);
    }

    #[test]
    fn test_log_verify_results_all_ok() {
        let results = vec![VerifyResult {
            model: "m".to_string(),
            label: "primary".to_string(),
            success: true,
            latency_ms: 42,
            error: None,
        }];
        assert!(log_verify_results(&results));
    }

    #[test]
    fn test_log_verify_results_with_failure() {
        let results = vec![VerifyResult {
            model: "m".to_string(),
            label: "primary".to_string(),
            success: false,
            latency_ms: 0,
            error: Some("boom".to_string()),
        }];
        assert!(!log_verify_results(&results));
    }
}
