use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse, ToolCallRequest};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::warn;

/// An LLM provider that tries a chain of providers in order,
/// falling back to the next on error or malformed tool calls.
pub struct FallbackProvider {
    providers: Vec<(Arc<dyn LLMProvider>, String)>,
}

impl FallbackProvider {
    /// Create a fallback chain. Providers are tried in order.
    pub fn new(providers: Vec<(Arc<dyn LLMProvider>, String)>) -> Self {
        assert!(
            !providers.is_empty(),
            "FallbackProvider requires at least one provider"
        );
        Self { providers }
    }

    /// Convenience constructor for the common two-provider case.
    pub fn pair(
        primary: Arc<dyn LLMProvider>,
        fallback: Arc<dyn LLMProvider>,
        primary_model: String,
        fallback_model: String,
    ) -> Self {
        Self::new(vec![(primary, primary_model), (fallback, fallback_model)])
    }
}

/// Validate that all tool calls in a response have well-formed names and arguments.
fn validate_tool_calls(tool_calls: &[ToolCallRequest]) -> bool {
    for tc in tool_calls {
        if tc.name.is_empty() {
            return false;
        }
        if !tc.arguments.is_object() {
            return false;
        }
    }
    true
}

#[async_trait]
impl LLMProvider for FallbackProvider {
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<LLMResponse> {
        let mut last_error = None;
        for (i, (provider, model_name)) in self.providers.iter().enumerate() {
            let is_last = i == self.providers.len() - 1;
            let attempt_req = ChatRequest {
                messages: req.messages.clone(),
                tools: req.tools.clone(),
                model: None,
                max_tokens: req.max_tokens,
                temperature: req.temperature,
                tool_choice: req.tool_choice.clone(),
                response_format: req.response_format.clone(),
            };

            match provider.chat(attempt_req).await {
                Ok(mut response) => {
                    if response.has_tool_calls() && !validate_tool_calls(&response.tool_calls) {
                        warn!(
                            "provider {} ({}) returned malformed tool calls{}",
                            i + 1,
                            model_name,
                            if is_last { "" } else { ", trying next" }
                        );
                        if is_last {
                            if i > 0 {
                                response.actual_model = Some(model_name.clone());
                            }
                            return Ok(response);
                        }
                        continue;
                    }
                    // Tag which model actually served the response when it
                    // wasn't the primary, so cost tracking uses the right rates.
                    if i > 0 {
                        response.actual_model = Some(model_name.clone());
                    }
                    return Ok(response);
                }
                Err(e) => {
                    if is_last {
                        warn!("provider {} ({}) failed: {}", i + 1, model_name, e);
                    } else {
                        warn!(
                            "provider {} ({}) failed: {}, trying next",
                            i + 1,
                            model_name,
                            e
                        );
                    }
                    last_error = Some(e);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no providers configured")))
    }

    fn default_model(&self) -> &str {
        &self.providers[0].1
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        let futures: Vec<_> = self
            .providers
            .iter()
            .map(|(p, name)| {
                let p = p.clone();
                let name = name.clone();
                async move {
                    if let Err(e) = p.warmup().await {
                        warn!("provider ({}) warmup failed: {}", name, e);
                    }
                }
            })
            .collect();
        futures_util::future::join_all(futures).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
