use std::sync::Arc;

use anyhow::Result;

use crate::providers::base::{
    ChatRequest, LLMProvider, LLMResponse, Message, ResponseFormat, RetryConfig, ToolDefinition,
};

/// Provider-facing adapter for chat request/response mapping.
///
/// Keeps provider request construction and retry invocation out of orchestration
/// code so routing/loop behavior remains easier to test in isolation.
pub(super) struct ModelGateway;

impl ModelGateway {
    pub(super) fn build_turn_request(
        messages: Vec<Message>,
        tools: Arc<Vec<ToolDefinition>>,
        model: &str,
        max_tokens: u32,
        temperature: Option<f32>,
        tool_choice: Option<String>,
        response_format: Option<ResponseFormat>,
    ) -> ChatRequest {
        ChatRequest {
            messages,
            tools: Some(tools),
            model: Some(model.to_string()),
            max_tokens,
            temperature,
            tool_choice,
            response_format,
        }
    }

    pub(super) fn build_summary_request(
        messages: Vec<Message>,
        model: &str,
        max_tokens: u32,
        temperature: Option<f32>,
    ) -> ChatRequest {
        ChatRequest {
            messages,
            model: Some(model.to_string()),
            max_tokens,
            temperature,
            ..Default::default()
        }
    }

    pub(super) async fn invoke(
        provider: &dyn LLMProvider,
        req: ChatRequest,
    ) -> Result<LLMResponse> {
        provider
            .chat_with_retry(&req, Some(RetryConfig::default()))
            .await
    }
}
