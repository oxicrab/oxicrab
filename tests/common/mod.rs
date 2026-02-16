use async_trait::async_trait;
use nanobot::agent::{AgentLoop, AgentLoopConfig};
use nanobot::bus::MessageBus;
use nanobot::config::CompactionConfig;
use nanobot::providers::base::{ChatRequest, LLMProvider, LLMResponse, Message, ToolCallRequest};
use std::collections::VecDeque;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct RecordedCall {
    pub messages: Vec<Message>,
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

// --- Response builders ---

pub fn text_response(content: &str) -> LLMResponse {
    LLMResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        reasoning_content: None,
        input_tokens: None,
    }
}

pub fn tool_response(calls: Vec<ToolCallRequest>) -> LLMResponse {
    LLMResponse {
        content: None,
        tool_calls: calls,
        reasoning_content: None,
        input_tokens: None,
    }
}

pub fn tool_call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCallRequest {
    ToolCallRequest {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
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
}

pub async fn create_test_agent_with(
    provider: impl LLMProvider + 'static,
    tmp: &TempDir,
    overrides: TestAgentOverrides,
) -> AgentLoop {
    let bus = Arc::new(Mutex::new(MessageBus::default()));
    let (outbound_tx, _outbound_rx) = tokio::sync::mpsc::channel(100);
    let outbound_tx = Arc::new(outbound_tx);

    let config = AgentLoopConfig {
        bus,
        provider: Arc::new(provider),
        workspace: tmp.path().to_path_buf(),
        model: Some("mock-model".to_string()),
        max_iterations: overrides.max_iterations.unwrap_or(10),
        brave_api_key: None,
        web_search_config: None,
        exec_timeout: overrides.exec_timeout.unwrap_or(30),
        restrict_to_workspace: overrides.restrict_to_workspace.unwrap_or(true),
        allowed_commands: overrides.allowed_commands.unwrap_or_default(),
        compaction_config: overrides.compaction_config.unwrap_or(CompactionConfig {
            enabled: false,
            threshold_tokens: 40000,
            keep_recent: 10,
            extraction_enabled: false,
            model: None,
        }),
        outbound_tx,
        cron_service: None,
        google_config: None,
        github_config: None,
        weather_config: None,
        todoist_config: None,
        media_config: None,
        obsidian_config: None,
        temperature: 0.7,
        tool_temperature: 0.0,
        session_ttl_days: 0,
        max_tokens: 8192,
        typing_tx: None,
        channels_config: None,
        memory_indexer_interval: 300,
        media_ttl_days: 0,
        max_concurrent_subagents: 5,
        voice_config: None,
        memory_config: None,
        browser_config: None,
        image_gen_config: None,
        mcp_config: None,
    };

    AgentLoop::new(config)
        .await
        .expect("Failed to create AgentLoop")
}
