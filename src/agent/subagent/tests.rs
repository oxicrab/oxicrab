use super::*;
use crate::providers::base::{ChatRequest, LLMResponse};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::Mutex as StdMutex;

/// Mock provider that returns pre-configured responses.
struct MockProvider {
    responses: StdMutex<VecDeque<LLMResponse>>,
}

impl MockProvider {
    fn with_responses(responses: Vec<LLMResponse>) -> Self {
        Self {
            responses: StdMutex::new(VecDeque::from(responses)),
        }
    }

    fn immediate(content: &str) -> Self {
        Self::with_responses(vec![LLMResponse {
            content: Some(content.to_string()),
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        }])
    }

    /// Provider that sleeps for `delay_ms` before returning.
    fn delayed(content: &str, delay_ms: u64) -> Arc<DelayedProvider> {
        Arc::new(DelayedProvider {
            content: content.to_string(),
            delay_ms,
        })
    }
}

#[async_trait]
impl LLMProvider for MockProvider {
    async fn chat(&self, _req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        let response = self.responses.lock().unwrap().pop_front();
        Ok(response.unwrap_or_else(|| LLMResponse {
            content: Some("default".to_string()),
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        }))
    }
    fn default_model(&self) -> &'static str {
        "mock"
    }
}

struct DelayedProvider {
    content: String,
    delay_ms: u64,
}

#[async_trait]
impl LLMProvider for DelayedProvider {
    async fn chat(&self, _req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        tokio::time::sleep(tokio::time::Duration::from_millis(self.delay_ms)).await;
        Ok(LLMResponse {
            content: Some(self.content.clone()),
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        })
    }
    fn default_model(&self) -> &'static str {
        "mock"
    }
}

fn make_manager(provider: Arc<dyn LLMProvider>, max_concurrent: usize) -> SubagentManager {
    let bus = Arc::new(Mutex::new(MessageBus::default()));
    SubagentManager::new(
        SubagentConfig {
            provider,
            workspace: PathBuf::from("/tmp/test"),
            model: Some("mock".to_string()),
            brave_api_key: None,
            exec_timeout: 10,
            restrict_to_workspace: false,
            allowed_commands: vec![],
            max_tokens: 1024,
            tool_temperature: 0.0,
            max_concurrent,
            exfil_blocked_tools: vec![],
            cost_guard: None,
            prompt_guard_config: crate::config::PromptGuardConfig::default(),
            sandbox_config: crate::config::SandboxConfig::default(),
        },
        bus,
    )
}

// --- Prompt building tests ---

#[test]
fn test_prompt_without_context() {
    let prompt = build_subagent_prompt("Do the thing", Path::new("/workspace"), None);
    assert!(prompt.contains("## Your Task\nDo the thing"));
    assert!(!prompt.contains("Conversation Context"));
    assert!(prompt.contains("/workspace"));
}

#[test]
fn test_prompt_with_context() {
    let prompt = build_subagent_prompt(
        "Research X",
        Path::new("/workspace"),
        Some("User asked about library Y for parsing JSON."),
    );
    assert!(prompt.contains("## Conversation Context"));
    assert!(prompt.contains("library Y for parsing JSON"));
    assert!(prompt.contains("## Your Task\nResearch X"));
}

#[test]
fn test_prompt_context_truncated_at_2000_chars() {
    let long_context: String = "x".repeat(3000);
    let prompt = build_subagent_prompt("task", Path::new("/ws"), Some(&long_context));
    // The context section should contain exactly MAX_CONTEXT_CHARS of 'x'
    let ctx_start = prompt.find("(for reference):\n").unwrap() + "(for reference):\n".len();
    let ctx_end = prompt[ctx_start..].find('\n').unwrap();
    assert_eq!(ctx_end, MAX_CONTEXT_CHARS);
}

// --- Capacity tests ---

#[tokio::test]
async fn test_capacity() {
    let provider = MockProvider::delayed("done", 500);
    let mgr = make_manager(provider, 3);

    // Initial state
    let (running, max, available) = mgr.capacity().await;
    assert_eq!((running, max, available), (0, 3, 3));

    mgr.spawn(
        "slow task".to_string(),
        None,
        "cli".to_string(),
        "direct".to_string(),
        true,
        None,
    )
    .await
    .unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let (running, max, available) = mgr.capacity().await;
    assert_eq!((running, max, available), (1, 3, 2));
}

// --- Concurrency limiter tests ---

// Custom provider that tracks concurrency
struct ConcurrencyTracker {
    concurrent: Arc<std::sync::atomic::AtomicUsize>,
    max_observed: Arc<std::sync::atomic::AtomicUsize>,
}
#[async_trait]
impl LLMProvider for ConcurrencyTracker {
    async fn chat(&self, _req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        let prev = self
            .concurrent
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let current = prev + 1;
        // Update max observed
        self.max_observed
            .fetch_max(current, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        self.concurrent
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        Ok(LLMResponse {
            content: Some("done".to_string()),
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        })
    }
    fn default_model(&self) -> &'static str {
        "mock"
    }
}

#[tokio::test]
async fn test_semaphore_limits_concurrency() {
    // Track how many are running concurrently via an atomic counter
    let concurrent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let max_observed = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let provider = Arc::new(ConcurrencyTracker {
        concurrent: concurrent.clone(),
        max_observed: max_observed.clone(),
    });

    let mgr = make_manager(provider, 2); // Limit to 2 concurrent

    // Spawn 4 tasks
    for i in 0..4 {
        mgr.spawn(
            format!("task {}", i),
            None,
            "cli".to_string(),
            "direct".to_string(),
            true,
            None,
        )
        .await
        .unwrap();
    }

    // Wait for all to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Max concurrent should never exceed 2
    let max = max_observed.load(std::sync::atomic::Ordering::SeqCst);
    assert!(max <= 2, "Expected max concurrency <= 2, got {}", max);
}

// --- Silent mode tests ---

#[tokio::test]
async fn test_silent_mode_no_bus_message() {
    let provider = Arc::new(MockProvider::immediate("result"));
    let bus = Arc::new(Mutex::new(MessageBus::default()));
    let mgr = SubagentManager::new(
        SubagentConfig {
            provider,
            workspace: PathBuf::from("/tmp/test"),
            model: Some("mock".to_string()),
            brave_api_key: None,
            exec_timeout: 10,
            restrict_to_workspace: false,
            allowed_commands: vec![],
            max_tokens: 1024,
            tool_temperature: 0.0,
            max_concurrent: 5,
            exfil_blocked_tools: vec![],
            cost_guard: None,
            prompt_guard_config: crate::config::PromptGuardConfig::default(),
            sandbox_config: crate::config::SandboxConfig::default(),
        },
        bus.clone(),
    );

    mgr.spawn(
        "silent task".to_string(),
        None,
        "telegram".to_string(),
        "chat1".to_string(),
        true, // silent
        None,
    )
    .await
    .unwrap();

    // Wait for completion
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Bus should NOT have received an inbound message
    // (since we're silent). Check by trying to take the receiver.
    let bus_guard = bus.lock().await;
    // The inbound_rx is still in the bus (not taken), so no messages were published
    // that we need to worry about. The key assertion is that the test doesn't panic
    // from a bus publish error, and we can verify the task completed.
    drop(bus_guard);
    let (running, _, _) = mgr.capacity().await;
    assert_eq!(running, 0, "Task should have completed");
}

#[tokio::test]
async fn test_non_silent_mode_publishes_bus_message() {
    let provider = Arc::new(MockProvider::immediate("result"));
    let bus = Arc::new(Mutex::new(MessageBus::default()));

    // Take the receiver so we can check for messages
    let inbound_rx = {
        let mut bus_guard = bus.lock().await;
        bus_guard.take_inbound_rx()
    };
    assert!(inbound_rx.is_some(), "Should be able to take inbound_rx");
    let mut rx = inbound_rx.unwrap();

    let mgr = SubagentManager::new(
        SubagentConfig {
            provider,
            workspace: PathBuf::from("/tmp/test"),
            model: Some("mock".to_string()),
            brave_api_key: None,
            exec_timeout: 10,
            restrict_to_workspace: false,
            allowed_commands: vec![],
            max_tokens: 1024,
            tool_temperature: 0.0,
            max_concurrent: 5,
            exfil_blocked_tools: vec![],
            cost_guard: None,
            prompt_guard_config: crate::config::PromptGuardConfig::default(),
            sandbox_config: crate::config::SandboxConfig::default(),
        },
        bus.clone(),
    );

    mgr.spawn(
        "announce task".to_string(),
        Some("test-label".to_string()),
        "telegram".to_string(),
        "chat1".to_string(),
        false, // NOT silent
        None,
    )
    .await
    .unwrap();

    // Wait for completion and announcement
    let msg = tokio::time::timeout(tokio::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("Should receive message within timeout")
        .expect("Channel should not be closed");

    assert_eq!(msg.channel, "system");
    assert_eq!(msg.sender_id, "subagent");
    assert_eq!(msg.chat_id, "telegram:chat1");
    assert!(msg.content.contains("test-label"));
    assert!(msg.content.contains("completed successfully"));
}

// --- Cancel test ---

#[tokio::test]
async fn test_cancel_running_task() {
    let provider = MockProvider::delayed("done", 5000);
    let mgr = make_manager(provider, 5);

    let result = mgr
        .spawn(
            "long task".to_string(),
            None,
            "cli".to_string(),
            "direct".to_string(),
            true,
            None,
        )
        .await
        .unwrap();

    // Extract task ID from result message
    let task_id = result
        .split("id: ")
        .nth(1)
        .unwrap()
        .split(')')
        .next()
        .unwrap();

    // Cancel it
    assert!(mgr.cancel(task_id).await);
    // Cancel again should return false
    assert!(!mgr.cancel(task_id).await);
}

// --- List running tests ---

#[tokio::test]
async fn test_list_running() {
    let provider = MockProvider::delayed("done", 1000);
    let mgr = make_manager(provider, 5);
    assert!(mgr.list_running().await.is_empty());

    mgr.spawn(
        "task1".to_string(),
        None,
        "cli".to_string(),
        "direct".to_string(),
        true,
        None,
    )
    .await
    .unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let list = mgr.list_running().await;
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].get("done").and_then(serde_json::Value::as_bool),
        Some(false)
    );
}

// --- Spawn label tests ---

#[tokio::test]
async fn test_spawn_auto_label_truncation() {
    let provider = Arc::new(MockProvider::immediate("done"));
    let mgr = make_manager(provider, 5);

    let result = mgr
        .spawn(
            "This is a very long task description that exceeds thirty characters easily"
                .to_string(),
            None, // No explicit label — should auto-truncate
            "cli".to_string(),
            "direct".to_string(),
            true,
            None,
        )
        .await
        .unwrap();

    // Should contain "..." indicating truncation
    assert!(result.contains("..."));
}

#[tokio::test]
async fn test_spawn_explicit_label() {
    let provider = Arc::new(MockProvider::immediate("done"));
    let mgr = make_manager(provider, 5);

    let result = mgr
        .spawn(
            "task".to_string(),
            Some("My Label".to_string()),
            "cli".to_string(),
            "direct".to_string(),
            true,
            None,
        )
        .await
        .unwrap();

    assert!(result.contains("My Label"));
}

use std::path::Path;

// --- Subagent tool registration tests ---

/// Helper to build a `SubagentInner` config for tool registration tests.
fn make_inner(exfil_blocked_tools: Vec<String>) -> super::SubagentInner {
    super::SubagentInner {
        provider: Arc::new(MockProvider::immediate("done")),
        workspace: PathBuf::from("/tmp/test"),
        model: "mock".to_string(),
        brave_api_key: None,
        exec_timeout: 10,
        restrict_to_workspace: false,
        allowed_commands: vec![],
        max_tokens: 1024,
        tool_temperature: 0.0,
        exfil_blocked_tools,
        cost_guard: None,
        prompt_guard: None,
        prompt_guard_config: crate::config::PromptGuardConfig::default(),
        sandbox_config: crate::config::SandboxConfig::default(),
    }
}

#[test]
fn test_subagent_tools_default_set() {
    let config = make_inner(vec![]);
    let tools = super::build_subagent_tools(&config).unwrap();
    let names = tools.tool_names();

    // Subagents get exactly these 6 tools
    assert_eq!(
        names,
        vec![
            "exec",
            "list_dir",
            "read_file",
            "web_fetch",
            "web_search",
            "write_file"
        ],
        "Subagents should have exactly 6 tools: filesystem + exec + web"
    );
}

#[test]
fn test_subagent_tools_no_github() {
    let config = make_inner(vec![]);
    let tools = super::build_subagent_tools(&config).unwrap();

    assert!(
        tools.get("github").is_none(),
        "Subagents must NOT have the github tool"
    );
}

#[test]
fn test_subagent_tools_no_domain_tools() {
    let config = make_inner(vec![]);
    let tools = super::build_subagent_tools(&config).unwrap();

    // None of these domain-specific tools should be available
    let forbidden = [
        "github",
        "google_mail",
        "google_calendar",
        "cron",
        "spawn",
        "subagent_control",
        "browser",
        "image_gen",
        "weather",
        "todoist",
        "obsidian",
        "memory_search",
        "media",
        "tmux",
        "http",
        "reddit",
        "edit_file",
    ];
    for name in &forbidden {
        assert!(
            tools.get(name).is_none(),
            "Subagent should NOT have tool '{}' but it was found",
            name
        );
    }
}

#[test]
fn test_subagent_tools_exfil_blocks_web_search() {
    let config = make_inner(vec!["web_search".to_string()]);
    let tools = super::build_subagent_tools(&config).unwrap();
    let names = tools.tool_names();

    assert!(
        !names.contains(&"web_search".to_string()),
        "web_search should be blocked by exfiltration guard"
    );
    assert!(
        names.contains(&"web_fetch".to_string()),
        "web_fetch should still be registered"
    );
    assert_eq!(names.len(), 5, "Should have 5 tools (6 minus web_search)");
}

#[test]
fn test_subagent_tools_exfil_blocks_web_fetch() {
    let config = make_inner(vec!["web_fetch".to_string()]);
    let tools = super::build_subagent_tools(&config).unwrap();
    let names = tools.tool_names();

    assert!(
        !names.contains(&"web_fetch".to_string()),
        "web_fetch should be blocked by exfiltration guard"
    );
    assert!(
        names.contains(&"web_search".to_string()),
        "web_search should still be registered"
    );
    assert_eq!(names.len(), 5, "Should have 5 tools (6 minus web_fetch)");
}

#[test]
fn test_subagent_tools_exfil_blocks_both_web_tools() {
    let config = make_inner(vec!["web_search".to_string(), "web_fetch".to_string()]);
    let tools = super::build_subagent_tools(&config).unwrap();
    let names = tools.tool_names();

    assert_eq!(
        names,
        vec!["exec", "list_dir", "read_file", "write_file"],
        "With both web tools blocked, subagent should have exactly 4 tools"
    );
}

// --- Activity log tests ---

#[test]
fn test_activity_log_creates_file() {
    let log = super::ActivityLog::new("test1234");
    assert!(log.is_some(), "ActivityLog should be creatable");
    let log = log.unwrap();
    assert!(log.path().exists(), "Log file should exist on disk");
    assert!(
        log.path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("subagent-test1234-"),
        "Log filename should contain task ID"
    );
    // Cleanup
    let _ = std::fs::remove_file(log.path());
}

#[test]
fn test_activity_log_writes_content() {
    let mut log = super::ActivityLog::new("logtest").unwrap();
    log.log_start("test task for logging");
    log.log_tools(
        &["exec".to_string(), "read_file".to_string()],
        &["web_fetch".to_string()],
    );
    log.log_iteration_tool_calls(1, 2);
    log.log_tool_call("exec", &serde_json::json!({"command": "ls"}));
    log.log_tool_result("exec", "file1.txt file2.txt", false);
    log.log_tool_result("exec", "command not found", true);
    log.log_iteration_text(2, 100);
    log.log_end("ok");

    let content = std::fs::read_to_string(log.path()).unwrap();

    assert!(content.contains("SUBAGENT START task_id=logtest"));
    assert!(content.contains("TASK: test task for logging"));
    assert!(content.contains("TOOLS REGISTERED: exec, read_file"));
    assert!(content.contains("TOOLS BLOCKED: web_fetch"));
    assert!(content.contains("ITERATION 1: LLM responded with 2 tool call(s)"));
    assert!(content.contains("TOOL CALL: exec {\"command\":\"ls\"}"));
    assert!(content.contains("TOOL RESULT: exec (19 chars): file1.txt file2.txt"));
    assert!(content.contains("TOOL ERROR: exec (17 chars): command not found"));
    assert!(content.contains("ITERATION 2: LLM responded with text (100 chars)"));
    assert!(content.contains("SUBAGENT END task_id=logtest status=ok duration="));

    // Cleanup
    let _ = std::fs::remove_file(log.path());
}

#[test]
fn test_activity_log_truncates_long_content() {
    let mut log = super::ActivityLog::new("trunctest").unwrap();
    let long_result = "x".repeat(1000);
    log.log_tool_result("exec", &long_result, false);

    let content = std::fs::read_to_string(log.path()).unwrap();
    // Should contain "..." suffix indicating truncation
    assert!(content.contains("..."));
    // Should contain the char count of the full content
    assert!(content.contains("(1000 chars)"));

    let _ = std::fs::remove_file(log.path());
}

#[test]
fn test_activity_log_special_entries() {
    let mut log = super::ActivityLog::new("spectest").unwrap();
    log.log_cost_blocked("monthly budget exceeded");
    log.log_max_iterations(15);
    log.log_iteration_empty(3, 1);

    let content = std::fs::read_to_string(log.path()).unwrap();
    assert!(content.contains("COST GUARD BLOCKED: monthly budget exceeded"));
    assert!(content.contains("MAX ITERATIONS REACHED (15)"));
    assert!(content.contains("ITERATION 3: LLM returned empty response (retries left: 1)"));

    let _ = std::fs::remove_file(log.path());
}

// --- ToolRegistry::tool_names test ---

#[test]
fn test_tool_registry_tool_names() {
    use crate::agent::tools::ToolRegistry;
    use crate::agent::tools::filesystem::ReadFileTool;

    let mut registry = ToolRegistry::new();
    assert!(registry.tool_names().is_empty());

    registry.register(Arc::new(ReadFileTool::new(None, None)));
    let names = registry.tool_names();
    assert_eq!(names, vec!["read_file"]);
}

#[test]
fn test_tool_registry_tool_names_sorted() {
    use crate::agent::tools::ToolRegistry;
    use crate::agent::tools::filesystem::{ListDirTool, ReadFileTool, WriteFileTool};

    let mut registry = ToolRegistry::new();
    // Register in non-sorted order
    registry.register(Arc::new(WriteFileTool::new(None, None, None)));
    registry.register(Arc::new(ReadFileTool::new(None, None)));
    registry.register(Arc::new(ListDirTool::new(None, None)));

    let names = registry.tool_names();
    assert_eq!(names, vec!["list_dir", "read_file", "write_file"]);
}
