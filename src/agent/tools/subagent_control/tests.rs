use super::*;
use crate::agent::subagent::SubagentManager;
use crate::agent::tools::Tool;
use crate::agent::tools::base::SubagentAccess;
use crate::bus::MessageBus;
use crate::config::PromptGuardConfig;
use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse};
use std::path::PathBuf;

struct DummyProvider;

#[async_trait::async_trait]
impl LLMProvider for DummyProvider {
    async fn chat(&self, _req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        unreachable!()
    }
    fn default_model(&self) -> &'static str {
        "dummy"
    }
}

fn make_tool() -> SubagentControlTool {
    let bus = Arc::new(tokio::sync::Mutex::new(MessageBus::new(10, 1.0, 16, 16)));
    let config = crate::agent::subagent::SubagentConfig {
        provider: Arc::new(DummyProvider),
        workspace: PathBuf::from("/tmp"),
        model: None,
        max_tokens: 1024,
        tool_temperature: 0.0,
        max_concurrent: 2,
        cost_guard: None,
        prompt_guard_config: PromptGuardConfig::default(),
        exfil_guard: crate::config::ExfiltrationGuardConfig::default(),
        main_tools: None,
    };
    let manager = Arc::new(SubagentManager::new(config, bus));
    SubagentControlTool::new(manager)
}

#[test]
fn test_subagent_control_capabilities() {
    let tool = make_tool();
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(!caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::Denied);
    assert!(caps.actions.is_empty());
}
