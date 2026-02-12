use async_trait::async_trait;
use nanobot::providers::base::{ChatRequest, LLMProvider, LLMResponse, Message, ToolDefinition};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RecordedCall {
    pub messages: Vec<Message>,
    pub tools: Option<Vec<ToolDefinition>>,
    pub model: Option<String>,
    pub max_tokens: u32,
    pub temperature: f32,
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

    #[allow(dead_code)]
    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    #[allow(dead_code)]
    pub fn last_call(&self) -> Option<RecordedCall> {
        self.calls.lock().unwrap().last().cloned()
    }

    #[allow(dead_code)]
    pub fn get_call(&self, index: usize) -> Option<RecordedCall> {
        self.calls.lock().unwrap().get(index).cloned()
    }
}

#[async_trait]
impl LLMProvider for MockLLMProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        self.calls.lock().unwrap().push(RecordedCall {
            messages: req.messages,
            tools: req.tools,
            model: req.model.map(|s| s.to_string()),
            max_tokens: req.max_tokens,
            temperature: req.temperature,
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
