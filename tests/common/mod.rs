// Shared test helpers — not all items used by every test binary.
#![allow(unused)]

use async_trait::async_trait;
use oxicrab::agent::{AgentLoop, AgentLoopConfig};
use oxicrab::bus::MessageBus;
use oxicrab::config::{
    CognitiveConfig, CompactionConfig, ExfiltrationGuardConfig, PromptGuardConfig,
};
use oxicrab::providers::base::{
    ChatRequest, LLMProvider, LLMResponse, Message, ToolCallRequest, ToolDefinition,
};
use std::collections::VecDeque;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct RecordedCall {
    pub messages: Vec<Message>,
    pub model: Option<String>,
    pub tools: Option<Vec<ToolDefinition>>,
    pub temperature: f32,
    pub max_tokens: u32,
}

pub struct MockLLMProvider {
    responses: Arc<std::sync::Mutex<VecDeque<LLMResponse>>>,
    pub calls: Arc<std::sync::Mutex<Vec<RecordedCall>>>,
    pub default_response: String,
}

impl MockLLMProvider {
    pub fn with_responses(responses: Vec<LLMResponse>) -> Self {
        Self {
            responses: Arc::new(std::sync::Mutex::new(VecDeque::from(responses))),
            calls: Arc::new(std::sync::Mutex::new(Vec::new())),
            default_response: "Mock response".to_string(),
        }
    }
}

#[async_trait]
impl LLMProvider for MockLLMProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        self.calls.lock().unwrap().push(RecordedCall {
            messages: req.messages,
            model: req.model.map(|s| s.to_string()),
            tools: req.tools,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
        });

        let response = self.responses.lock().unwrap().pop_front();
        Ok(response.unwrap_or_else(|| LLMResponse {
            content: Some(self.default_response.clone()),
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
        }))
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }
}

// --- Response builders ---

pub fn text_response(content: &str) -> LLMResponse {
    LLMResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        reasoning_content: None,
        input_tokens: None,
        output_tokens: None,
    }
}

pub fn tool_response(calls: Vec<ToolCallRequest>) -> LLMResponse {
    LLMResponse {
        content: None,
        tool_calls: calls,
        reasoning_content: None,
        input_tokens: None,
        output_tokens: None,
    }
}

pub fn tool_call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCallRequest {
    ToolCallRequest {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
    }
}

// --- Tool-capturing provider ---

/// A mock provider that captures tool definitions passed by the agent loop.
pub struct ToolCapturingProvider {
    responses: Arc<std::sync::Mutex<VecDeque<LLMResponse>>>,
    pub tool_defs: Arc<std::sync::Mutex<Vec<Option<Vec<ToolDefinition>>>>>,
    pub default_response: String,
}

impl ToolCapturingProvider {
    pub fn new() -> Self {
        Self {
            responses: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            tool_defs: Arc::new(std::sync::Mutex::new(Vec::new())),
            default_response: "Mock response".to_string(),
        }
    }

    pub fn with_responses(responses: Vec<LLMResponse>) -> Self {
        Self {
            responses: Arc::new(std::sync::Mutex::new(VecDeque::from(responses))),
            tool_defs: Arc::new(std::sync::Mutex::new(Vec::new())),
            default_response: "Mock response".to_string(),
        }
    }
}

#[async_trait]
impl LLMProvider for ToolCapturingProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        self.tool_defs.lock().unwrap().push(req.tools);
        let response = self.responses.lock().unwrap().pop_front();
        Ok(response.unwrap_or_else(|| LLMResponse {
            content: Some(self.default_response.clone()),
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
        }))
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }
}

// --- Agent constructor with overrides ---

#[derive(Default)]
pub struct TestAgentOverrides {
    pub allowed_commands: Option<Vec<String>>,
    pub exec_timeout: Option<u64>,
    pub compaction_config: Option<CompactionConfig>,
    pub restrict_to_workspace: Option<bool>,
    pub max_iterations: Option<usize>,
    pub cognitive_config: Option<CognitiveConfig>,
    pub exfiltration_guard: Option<ExfiltrationGuardConfig>,
    pub prompt_guard_config: Option<PromptGuardConfig>,
}

pub async fn create_test_agent_with(
    provider: impl LLMProvider + 'static,
    tmp: &TempDir,
    overrides: TestAgentOverrides,
) -> AgentLoop {
    let bus = Arc::new(Mutex::new(MessageBus::default()));
    let (outbound_tx, _outbound_rx) = tokio::sync::mpsc::channel(100);
    let outbound_tx = Arc::new(outbound_tx);

    let mut config = AgentLoopConfig::test_defaults(
        bus,
        Arc::new(provider),
        tmp.path().to_path_buf(),
        outbound_tx,
    );
    if let Some(v) = overrides.allowed_commands {
        config.allowed_commands = v;
    }
    if let Some(v) = overrides.exec_timeout {
        config.exec_timeout = v;
    }
    if let Some(v) = overrides.compaction_config {
        config.compaction_config = v;
    }
    if let Some(v) = overrides.restrict_to_workspace {
        config.restrict_to_workspace = v;
    }
    if let Some(v) = overrides.max_iterations {
        config.max_iterations = v;
    }
    if let Some(v) = overrides.cognitive_config {
        config.cognitive_config = v;
    }
    if let Some(v) = overrides.exfiltration_guard {
        config.exfiltration_guard = v;
    }
    if let Some(v) = overrides.prompt_guard_config {
        config.prompt_guard_config = v;
    }

    AgentLoop::new(config)
        .await
        .expect("Failed to create AgentLoop")
}

// --- Failing mock provider ---

/// An LLM provider that always returns an error.
pub struct FailingMockProvider {
    error_message: String,
    pub calls: Arc<std::sync::Mutex<Vec<RecordedCall>>>,
}

impl FailingMockProvider {
    pub fn new(error_message: &str) -> Self {
        Self {
            error_message: error_message.to_string(),
            calls: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl LLMProvider for FailingMockProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        self.calls.lock().unwrap().push(RecordedCall {
            messages: req.messages,
            model: req.model.map(|s| s.to_string()),
            tools: req.tools,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
        });
        Err(anyhow::anyhow!("{}", self.error_message))
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }
}
