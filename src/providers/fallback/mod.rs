use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse, ToolCallRequest};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::{debug, warn};

/// An LLM provider that tries a primary (cloud) provider first,
/// falling back to a secondary (local) provider on error or malformed tool calls.
pub struct FallbackProvider {
    primary: Arc<dyn LLMProvider>,
    fallback: Arc<dyn LLMProvider>,
    primary_model: String,
    fallback_model: String,
}

impl FallbackProvider {
    pub fn new(
        primary: Arc<dyn LLMProvider>,
        fallback: Arc<dyn LLMProvider>,
        primary_model: String,
        fallback_model: String,
    ) -> Self {
        Self {
            primary,
            fallback,
            primary_model,
            fallback_model,
        }
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
    async fn chat(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        // Build a request for the primary (local) model.
        // Use model: None so the provider uses its own default_model() (already stripped of prefix).
        let primary_req = ChatRequest {
            messages: req.messages.clone(),
            tools: req.tools.clone(),
            model: None,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            tool_choice: req.tool_choice.clone(),
            response_format: req.response_format.clone(),
        };

        match self.primary.chat(primary_req).await {
            Ok(response) => {
                // If there are tool calls, validate them
                if response.has_tool_calls() && !validate_tool_calls(&response.tool_calls) {
                    warn!(
                        "primary provider ({}) returned malformed tool calls, falling back to {}",
                        self.primary_model, self.fallback_model
                    );
                } else {
                    return Ok(response);
                }
            }
            Err(e) => {
                warn!(
                    "primary provider ({}) failed: {}, falling back to {}",
                    self.primary_model, e, self.fallback_model
                );
            }
        }

        // Fall back to cloud provider — also use None to let it pick its own default
        let fallback_req = ChatRequest {
            messages: req.messages,
            tools: req.tools,
            model: None,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            tool_choice: req.tool_choice,
            response_format: req.response_format,
        };

        let response = self.fallback.chat(fallback_req).await?;
        debug!("fallback provider ({}) succeeded", self.fallback_model);
        Ok(response)
    }

    fn default_model(&self) -> &str {
        &self.primary_model
    }

    fn metrics(&self) -> crate::providers::base::ProviderMetrics {
        // Aggregate metrics from both providers
        let p = self.primary.metrics();
        let f = self.fallback.metrics();
        crate::providers::base::ProviderMetrics {
            request_count: p.request_count + f.request_count,
            token_count: p.token_count + f.token_count,
            error_count: p.error_count + f.error_count,
        }
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        // Warm up both providers in parallel
        let (primary_result, fallback_result) =
            tokio::join!(self.primary.warmup(), self.fallback.warmup());
        if let Err(e) = primary_result {
            warn!("primary provider warmup failed: {}", e);
        }
        if let Err(e) = fallback_result {
            warn!("fallback provider warmup failed: {}", e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
