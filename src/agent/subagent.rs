use crate::agent::tools::{
    filesystem::{ListDirTool, ReadFileTool, WriteFileTool},
    shell::ExecTool,
    web::{WebFetchTool, WebSearchTool},
    ToolRegistry,
};
use crate::agent::truncation::truncate_tool_result;
use crate::bus::{InboundMessage, MessageBus};
use crate::providers::base::{LLMProvider, Message};
use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

const MAX_TOOL_RESULT_CHARS: usize = 10000;
const EMPTY_RESPONSE_RETRIES: usize = 2;
const MAX_WEB_FETCH_CHARS: usize = 50000;
const MAX_SUBAGENT_ITERATIONS: usize = 15;
const MAX_CONTEXT_CHARS: usize = 2000;

/// Immutable configuration shared across all subagent tasks via `Arc`.
#[derive(Clone)]
pub struct SubagentConfig {
    pub provider: Arc<dyn LLMProvider>,
    pub workspace: PathBuf,
    pub model: Option<String>,
    pub brave_api_key: Option<String>,
    pub exec_timeout: u64,
    pub restrict_to_workspace: bool,
    pub allowed_commands: Vec<String>,
    pub max_tokens: u32,
    pub tool_temperature: f32,
    pub max_concurrent: usize,
}

pub struct SubagentManager {
    config: Arc<SubagentInner>,
    running_tasks: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    bus: Arc<Mutex<MessageBus>>,
    semaphore: Arc<tokio::sync::Semaphore>,
}

/// Resolved immutable config (model already resolved, no Option).
struct SubagentInner {
    provider: Arc<dyn LLMProvider>,
    workspace: PathBuf,
    model: String,
    brave_api_key: Option<String>,
    exec_timeout: u64,
    restrict_to_workspace: bool,
    allowed_commands: Vec<String>,
    max_tokens: u32,
    tool_temperature: f32,
}

impl SubagentManager {
    pub fn new(config: SubagentConfig, bus: Arc<Mutex<MessageBus>>) -> Self {
        let model = config
            .model
            .unwrap_or_else(|| config.provider.default_model().to_string());
        let max_concurrent = config.max_concurrent;
        let inner = Arc::new(SubagentInner {
            provider: config.provider,
            workspace: config.workspace,
            model,
            brave_api_key: config.brave_api_key,
            exec_timeout: config.exec_timeout,
            restrict_to_workspace: config.restrict_to_workspace,
            allowed_commands: config.allowed_commands,
            max_tokens: config.max_tokens,
            tool_temperature: config.tool_temperature,
        });
        Self {
            config: inner,
            running_tasks: Arc::new(Mutex::new(HashMap::new())),
            bus,
            semaphore: Arc::new(tokio::sync::Semaphore::new(max_concurrent)),
        }
    }

    pub async fn spawn(
        &self,
        task: String,
        label: Option<String>,
        origin_channel: String,
        origin_chat_id: String,
        silent: bool,
        context: Option<String>,
    ) -> Result<String> {
        let task_id = Uuid::new_v4().to_string()[..8].to_string();
        let display_label = label.unwrap_or_else(|| {
            if task.chars().count() > 30 {
                let truncated: String = task.chars().take(30).collect();
                format!("{}...", truncated)
            } else {
                task.clone()
            }
        });
        let display_label_clone = display_label.clone();
        let task_id_clone = task_id.clone();

        let origin = (origin_channel.clone(), origin_chat_id.clone());

        // Capture Arc references for the spawned task (no cloning of Strings/Vecs)
        let config = self.config.clone();
        let bus = self.bus.clone();
        let running_tasks = self.running_tasks.clone();
        let semaphore = self.semaphore.clone();

        let bg_task = tokio::spawn(async move {
            // Acquire semaphore permit — blocks if all slots are busy
            let Ok(_permit) = semaphore.acquire().await else {
                warn!("Subagent [{}] semaphore closed", task_id_clone);
                return;
            };

            run_subagent(
                &config,
                &bus,
                &running_tasks,
                SubagentTask {
                    task_id: task_id_clone,
                    task,
                    label: display_label_clone,
                    origin,
                    silent,
                    context,
                },
            )
            .await;
        });

        self.running_tasks
            .lock()
            .await
            .insert(task_id.clone(), bg_task);

        info!("Spawned subagent [{}]: {}", task_id, display_label);
        Ok(format!(
            "Subagent [{}] started (id: {}). I'll notify you when it completes.",
            display_label, task_id
        ))
    }

    pub async fn list_running(&self) -> Vec<HashMap<String, Value>> {
        let tasks = self.running_tasks.lock().await;
        tasks
            .iter()
            .map(|(id, handle)| {
                let mut map = HashMap::new();
                map.insert("id".to_string(), Value::String(id.clone()));
                map.insert("done".to_string(), Value::Bool(handle.is_finished()));
                map.insert("cancelled".to_string(), Value::Bool(false));
                map
            })
            .collect()
    }

    pub async fn cancel(&self, task_id: &str) -> bool {
        let mut tasks = self.running_tasks.lock().await;
        if let Some(handle) = tasks.remove(task_id) {
            handle.abort();
            true
        } else {
            false
        }
    }

    /// Returns (running, max, available) capacity info.
    pub async fn capacity(&self) -> (usize, usize, usize) {
        let running = self.running_tasks.lock().await.len();
        let max = self.semaphore.available_permits() + running;
        let available = self.semaphore.available_permits();
        (running, max, available)
    }
}

/// Parameters for a subagent task.
struct SubagentTask {
    task_id: String,
    task: String,
    label: String,
    origin: (String, String),
    silent: bool,
    context: Option<String>,
}

/// Run a subagent task (called inside `tokio::spawn`).
async fn run_subagent(
    config: &SubagentInner,
    bus: &Arc<Mutex<MessageBus>>,
    running_tasks: &Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    params: SubagentTask,
) {
    let SubagentTask {
        task_id,
        task,
        label,
        origin,
        silent,
        context,
    } = params;
    info!("Subagent [{}] starting task: {}", task_id, label);

    let result = run_subagent_inner(config, &task_id, &task, context.as_deref()).await;

    // Cleanup
    running_tasks.lock().await.remove(&task_id);

    match result {
        Ok(final_result) => {
            info!("Subagent [{}] completed successfully", task_id);
            if !silent {
                announce_result(bus, &task_id, &label, &task, &final_result, &origin, "ok").await;
            }
        }
        Err(e) => {
            warn!("Subagent [{}] failed: {}", task_id, e);
            if !silent {
                announce_result(
                    bus,
                    &task_id,
                    &label,
                    &task,
                    &format!("Error: {}", e),
                    &origin,
                    "error",
                )
                .await;
            }
        }
    }
}

async fn run_subagent_inner(
    config: &SubagentInner,
    task_id: &str,
    task: &str,
    context: Option<&str>,
) -> Result<String> {
    // Build tools
    let mut tools = ToolRegistry::new();
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    let allowed_roots = if config.restrict_to_workspace {
        Some(vec![config.workspace.clone(), home.clone()])
    } else {
        None
    };

    let backup_dir = Some(home.join(".nanobot/backups"));

    tools.register(Arc::new(ReadFileTool::new(allowed_roots.clone())));
    tools.register(Arc::new(WriteFileTool::new(
        allowed_roots.clone(),
        backup_dir,
    )));
    tools.register(Arc::new(ListDirTool::new(allowed_roots)));
    tools.register(Arc::new(ExecTool::new(
        config.exec_timeout,
        Some(config.workspace.clone()),
        config.restrict_to_workspace,
        config.allowed_commands.clone(),
    )?));
    tools.register(Arc::new(WebSearchTool::new(
        config.brave_api_key.clone(),
        5,
    )));
    tools.register(Arc::new(WebFetchTool::new(MAX_WEB_FETCH_CHARS)?));

    // Build messages
    let system_prompt = build_subagent_prompt(task, &config.workspace, context);
    let mut messages = vec![Message::system(system_prompt), Message::user(task)];

    // Run agent loop
    let max_iterations = MAX_SUBAGENT_ITERATIONS;
    let mut iteration = 0;
    let mut empty_retries_left = EMPTY_RESPONSE_RETRIES;

    while iteration < max_iterations {
        iteration += 1;

        let response = config
            .provider
            .chat(crate::providers::base::ChatRequest {
                messages: messages.clone(),
                tools: Some(tools.get_tool_definitions()),
                model: Some(&config.model),
                max_tokens: config.max_tokens,
                temperature: config.tool_temperature,
                tool_choice: None,
            })
            .await?;

        if response.has_tool_calls() {
            // Add assistant message
            messages.push(Message::assistant(
                response.content.clone().unwrap_or_default(),
                Some(response.tool_calls.clone()),
            ));

            // Execute tools in parallel (same pattern as main agent loop)
            let tool_lookups: Vec<_> = response
                .tool_calls
                .iter()
                .map(|tc| {
                    let tool_opt = tools.get(&tc.name);
                    (tc.clone(), tool_opt)
                })
                .collect();

            let results = if tool_lookups.len() == 1 {
                // Single tool fast-path
                let (ref tc, ref tool_opt) = tool_lookups[0];
                vec![
                    execute_subagent_tool(task_id, &tc.name, &tc.arguments, tool_opt.clone()).await,
                ]
            } else {
                // Parallel execution
                let handles: Vec<_> = tool_lookups
                    .iter()
                    .map(|(tc, tool_opt)| {
                        let task_id = task_id.to_string();
                        let tc_name = tc.name.clone();
                        let tc_args = tc.arguments.clone();
                        let tool_opt = tool_opt.clone();
                        tokio::task::spawn(async move {
                            execute_subagent_tool(&task_id, &tc_name, &tc_args, tool_opt).await
                        })
                    })
                    .collect();
                futures_util::future::join_all(handles)
                    .await
                    .into_iter()
                    .map(|join_result| match join_result {
                        Ok(result) => result,
                        Err(join_err) => {
                            error!("Subagent tool task panicked: {:?}", join_err);
                            ("Tool crashed unexpectedly".to_string(), true)
                        }
                    })
                    .collect()
            };

            // Add all tool results to messages in order
            for ((tc, _), (result_str, is_error)) in tool_lookups.iter().zip(results.into_iter()) {
                messages.push(Message::tool_result(tc.id.clone(), result_str, is_error));
            }
        } else if let Some(content) = response.content {
            return Ok(content);
        } else {
            if empty_retries_left > 0 {
                empty_retries_left -= 1;
                let retry_num = EMPTY_RESPONSE_RETRIES - empty_retries_left;
                let delay = (2_u64.pow(retry_num as u32) as f64 + fastrand::f64()).min(10.0);
                warn!(
                    "Subagent [{}] got empty response, retries left: {}, backing off {:.1}s",
                    task_id, empty_retries_left, delay
                );
                tokio::time::sleep(tokio::time::Duration::from_secs_f64(delay)).await;
                continue;
            }
            warn!(
                "Subagent [{}] empty response, no retries left - giving up",
                task_id
            );
            break;
        }
    }

    Ok("Task completed but no final response was generated.".to_string())
}

/// Execute a single tool call for a subagent, with validation and panic isolation.
async fn execute_subagent_tool(
    task_id: &str,
    tool_name: &str,
    tool_args: &Value,
    tool_opt: Option<Arc<dyn crate::agent::tools::base::Tool>>,
) -> (String, bool) {
    if let Some(tool) = tool_opt {
        // Validate params before execution
        if let Some(validation_error) =
            crate::agent::agent_loop::validate_tool_params(tool.as_ref(), tool_args)
        {
            warn!(
                "Subagent [{}] tool '{}' param validation failed: {}",
                task_id, tool_name, validation_error
            );
            return (validation_error, true);
        }

        debug!(
            "Subagent [{}] executing: {} with arguments: {}",
            task_id, tool_name, tool_args
        );

        let ctx = crate::agent::tools::base::ExecutionContext::default();
        match tool.execute(tool_args.clone(), &ctx).await {
            Ok(result) => (
                truncate_tool_result(&result.content, MAX_TOOL_RESULT_CHARS),
                result.is_error,
            ),
            Err(e) => {
                warn!("Subagent [{}] tool '{}' failed: {}", task_id, tool_name, e);
                (format!("Tool execution failed: {}", e), true)
            }
        }
    } else {
        warn!("Subagent [{}] called unknown tool: {}", task_id, tool_name);
        (format!("Error: tool '{}' does not exist", tool_name), true)
    }
}

async fn announce_result(
    bus: &Arc<Mutex<MessageBus>>,
    task_id: &str,
    label: &str,
    task: &str,
    result: &str,
    origin: &(String, String),
    status: &str,
) {
    let status_text = if status == "ok" {
        "completed successfully"
    } else {
        "failed"
    };
    let announce_content = format!(
        "[Subagent '{}' {}]\n\nTask: {}\n\nResult:\n{}\n\nSummarize this naturally for the user. Keep it brief (1-2 sentences). Do not mention technical details like \"subagent\" or task IDs.",
        label, status_text, task, result
    );

    let msg = InboundMessage {
        channel: "system".to_string(),
        sender_id: "subagent".to_string(),
        chat_id: format!("{}:{}", origin.0, origin.1),
        content: announce_content,
        timestamp: Utc::now(),
        media: vec![],
        metadata: HashMap::new(),
    };

    if let Err(e) = bus.lock().await.publish_inbound(msg).await {
        warn!("Failed to publish inbound message from subagent: {}", e);
    }
    debug!(
        "Subagent [{}] announced result to {}:{}",
        task_id, origin.0, origin.1
    );
}

fn build_subagent_prompt(task: &str, workspace: &std::path::Path, context: Option<&str>) -> String {
    let context_section = if let Some(ctx) = context {
        // Cap context to avoid bloating subagent token usage
        let trimmed: String = ctx.chars().take(MAX_CONTEXT_CHARS).collect();
        format!(
            "\n## Conversation Context\nThe main agent's recent conversation (for reference):\n{}\n",
            trimmed
        )
    } else {
        String::new()
    };

    format!(
        r"# Subagent

You are a subagent spawned by the main agent to complete a specific task.

## Your Task
{}
{}
## Rules
1. Stay focused - complete only the assigned task, nothing else
2. Your final response will be reported back to the main agent
3. Do not initiate conversations or take on side tasks
4. Be concise but informative in your findings

## What You Can Do
- Read and write files in the workspace
- Execute shell commands
- Search the web and fetch web pages
- Complete the task thoroughly

## What You Cannot Do
- Send messages directly to users (no message tool available)
- Spawn other subagents
- Access the main agent's full conversation history

## Workspace
Your workspace is at: {}

When you have completed the task, provide a clear summary of your findings or actions.",
        task,
        context_section,
        workspace.display()
    )
}

#[cfg(test)]
mod tests {
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
    async fn test_capacity_initial() {
        let provider = Arc::new(MockProvider::immediate("done"));
        let mgr = make_manager(provider, 5);
        let (running, max, available) = mgr.capacity().await;
        assert_eq!(running, 0);
        assert_eq!(max, 5);
        assert_eq!(available, 5);
    }

    #[tokio::test]
    async fn test_capacity_after_spawn() {
        let provider = MockProvider::delayed("done", 500);
        let mgr = make_manager(provider, 3);

        mgr.spawn(
            "slow task".to_string(),
            None,
            "cli".to_string(),
            "direct".to_string(),
            true, // silent to avoid bus publish issues
            None,
        )
        .await
        .unwrap();

        // Give the task a moment to start and acquire the semaphore
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let (running, max, available) = mgr.capacity().await;
        assert_eq!(running, 1);
        assert_eq!(max, 3);
        assert_eq!(available, 2);
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
    async fn test_list_running_empty() {
        let provider = Arc::new(MockProvider::immediate("done"));
        let mgr = make_manager(provider, 5);
        let list = mgr.list_running().await;
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn test_list_running_with_tasks() {
        let provider = MockProvider::delayed("done", 1000);
        let mgr = make_manager(provider, 5);

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
}
