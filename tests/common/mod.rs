use async_trait::async_trait;
use nanobot::providers::base::{ChatRequest, LLMProvider, LLMResponse, Message, ToolDefinition};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct RecordedCall {
    pub messages: Vec<Message>,
    pub tools: Option<Vec<ToolDefinition>>,
}

pub struct MockLLMProvider {
    responses: Arc<Mutex<VecDeque<LLMResponse>>>,
    pub calls: Arc<Mutex<Vec<RecordedCall>>>,
    pub default_response: String,
}

impl MockLLMProvider {
    pub fn new() -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::new())),
            calls: Arc::new(Mutex::new(Vec::new())),
            default_response: "Mock response".to_string(),
        }
    }

    pub fn with_responses(responses: Vec<LLMResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            calls: Arc::new(Mutex::new(Vec::new())),
            default_response: "Mock response".to_string(),
        }
    }
}

#[async_trait]
impl LLMProvider for MockLLMProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        self.calls.lock().unwrap().push(RecordedCall {
            messages: req.messages,
            tools: req.tools,
        });

        let response = self.responses.lock().unwrap().pop_front();
        Ok(response.unwrap_or_else(|| LLMResponse {
            content: Some(self.default_response.clone()),
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
        }))
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }
}
