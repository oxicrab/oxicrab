mod activity_log;

use crate::agent::cost_guard::CostGuard;
use crate::agent::tools::ToolRegistry;
use crate::bus::{InboundMessage, MessageBus};
use crate::config::PromptGuardConfig;
use crate::providers::base::{LLMProvider, Message};
use crate::safety::prompt_guard::PromptGuard;
use activity_log::ActivityLog;
use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use uuid::Uuid;

const EMPTY_RESPONSE_RETRIES: usize = 2;
const MAX_SUBAGENT_ITERATIONS: usize = 15;
const MAX_CONTEXT_CHARS: usize = 2000;
/// Overall timeout for a subagent run (5 minutes)
const SUBAGENT_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(5);

/// Immutable configuration shared across all subagent tasks via `Arc`.
#[derive(Clone)]
pub struct SubagentConfig {
    pub provider: Arc<dyn LLMProvider>,
    pub workspace: PathBuf,
    pub model: Option<String>,
    pub max_tokens: u32,
    pub tool_temperature: f32,
    pub max_concurrent: usize,
    /// Shared cost guard for budget/rate enforcement across main agent and subagents.
    pub cost_guard: Option<Arc<CostGuard>>,
    /// Prompt guard config for injection scanning on subagent inputs/outputs.
    pub prompt_guard_config: PromptGuardConfig,
    /// Exfiltration guard config — network-outbound tools are excluded unless allow-listed.
    pub exfil_guard: crate::config::ExfiltrationGuardConfig,
    /// Main agent's tool registry, used to build subagent tools from capabilities.
    /// Set after `register_all_tools()` returns via `SubagentManager::set_main_tools()`.
    pub main_tools: Option<Arc<ToolRegistry>>,
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
    max_tokens: u32,
    tool_temperature: f32,
    cost_guard: Option<Arc<CostGuard>>,
    prompt_guard: Option<PromptGuard>,
    prompt_guard_config: PromptGuardConfig,
    exfil_guard: crate::config::ExfiltrationGuardConfig,
    main_tools: std::sync::OnceLock<Arc<ToolRegistry>>,
}

impl SubagentManager {
    pub fn new(config: SubagentConfig, bus: Arc<Mutex<MessageBus>>) -> Self {
        let model = config
            .model
            .unwrap_or_else(|| config.provider.default_model().to_string());
        let max_concurrent = config.max_concurrent;
        let prompt_guard = if config.prompt_guard_config.enabled {
            Some(PromptGuard::new())
        } else {
            None
        };
        let inner = Arc::new(SubagentInner {
            provider: config.provider,
            workspace: config.workspace,
            model,
            max_tokens: config.max_tokens,
            tool_temperature: config.tool_temperature,
            cost_guard: config.cost_guard,
            prompt_guard,
            prompt_guard_config: config.prompt_guard_config,
            exfil_guard: config.exfil_guard,
            main_tools: {
                let lock = std::sync::OnceLock::new();
                if let Some(tools) = config.main_tools {
                    let _ = lock.set(tools);
                }
                lock
            },
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

        // Hold the lock while spawning to prevent the race where the task
        // finishes and tries to remove itself before we insert the handle.
        let mut tasks = self.running_tasks.lock().await;
        // Prune finished tasks and enforce capacity limit
        tasks.retain(|_, handle| !handle.is_finished());
        if tasks.len() >= 100 {
            anyhow::bail!(
                "too many tracked subagent tasks ({}), try again later",
                tasks.len()
            );
        }
        let bg_task = tokio::spawn(async move {
            // Acquire semaphore permit — blocks if all slots are busy.
            // The permit is held for the duration of the task and released
            // on drop (including abort/cancellation).
            let Ok(_permit) = semaphore.acquire().await else {
                warn!("Subagent [{}] semaphore closed", task_id_clone);
                return;
            };

            // Use AssertUnwindSafe + catch_unwind pattern via select to ensure
            // cleanup runs even if the task is aborted. The permit is released
            // automatically by drop when the spawned task exits (including abort).
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
            // NOTE: If this task is aborted, the permit (_permit) is still
            // dropped correctly by tokio's task cleanup. The running_tasks
            // cleanup below won't run, but cancel() already removes the entry.
        });
        tasks.insert(task_id.clone(), bg_task);
        drop(tasks);

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

    /// Set the main agent's tool registry after `register_all_tools()` returns.
    /// This enables capability-based subagent tool filtering.
    pub fn set_main_tools(&self, tools: Arc<ToolRegistry>) {
        let _ = self.config.main_tools.set(tools);
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

    let result = if let Ok(r) = tokio::time::timeout(
        SUBAGENT_TIMEOUT,
        run_subagent_inner(config, &task_id, &task, context.as_deref(), &origin),
    )
    .await
    {
        r
    } else {
        warn!(
            "Subagent [{}] timed out after {}s",
            task_id,
            SUBAGENT_TIMEOUT.as_secs()
        );
        Ok(format!(
            "Task timed out after {} seconds",
            SUBAGENT_TIMEOUT.as_secs()
        ))
    };

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

/// Build the tool registry for a subagent by querying capabilities from the
/// main agent's tool registry. Tools with `SubagentAccess::Full` are passed
/// through (network-outbound tools respect the exfil block list), tools with
/// `SubagentAccess::ReadOnly` are wrapped to expose only read-only actions,
/// and `SubagentAccess::Denied` tools are excluded.
fn build_subagent_tools(config: &SubagentInner) -> ToolRegistry {
    use crate::agent::tools::base::SubagentAccess;
    use crate::agent::tools::read_only_wrapper::ReadOnlyToolWrapper;

    let main_tools = config
        .main_tools
        .get()
        .expect("main_tools must be set before spawning subagents");

    let mut tools = ToolRegistry::new();
    for (name, tool) in main_tools.iter() {
        let caps = tool.capabilities();
        match caps.subagent_access {
            SubagentAccess::Full => {
                if caps.network_outbound
                    && config.exfil_guard.enabled
                    && !config.exfil_guard.allow_tools.contains(&name.to_string())
                {
                    continue;
                }
                tools.register(tool.clone());
            }
            SubagentAccess::ReadOnly => {
                if let Some(wrapped) = ReadOnlyToolWrapper::new(tool.clone()) {
                    tools.register(Arc::new(wrapped));
                }
            }
            SubagentAccess::Denied => {}
        }
    }
    tools
}

async fn run_subagent_inner(
    config: &SubagentInner,
    task_id: &str,
    task: &str,
    context: Option<&str>,
    origin: &(String, String),
) -> Result<String> {
    let mut log = ActivityLog::new(task_id);
    if let Some(ref mut l) = log {
        info!(
            "Subagent [{}] activity log: {}",
            task_id,
            l.path().display()
        );
        l.log_start(task);
    }

    // Build tools from main registry capabilities
    let tools = build_subagent_tools(config);

    // Log registered tools
    let registered_names = tools.tool_names();
    info!(
        "Subagent [{}] tools registered: [{}], exfil_guard: {}",
        task_id,
        registered_names.join(", "),
        if config.exfil_guard.enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    if let Some(ref mut l) = log {
        l.log_tools(&registered_names);
    }

    // Scan task input for prompt injection if configured to block
    if let Some(ref guard) = config.prompt_guard
        && config.prompt_guard_config.should_block()
    {
        let matches = guard.scan(task);
        if !matches.is_empty() {
            for m in &matches {
                warn!(
                    "Subagent [{}] prompt injection in task input ({:?}): {}",
                    task_id, m.category, m.pattern_name
                );
            }
            if let Some(ref mut l) = log {
                l.log_end("blocked-injection");
            }
            anyhow::bail!("prompt injection detected in subagent task input");
        }
    }

    // Build messages
    let system_prompt = build_subagent_prompt(task, &config.workspace, context);
    let mut messages = vec![Message::system(system_prompt), Message::user(task)];

    // Run agent loop
    let max_iterations = MAX_SUBAGENT_ITERATIONS;
    let mut iteration = 0;
    let mut empty_retries_left = EMPTY_RESPONSE_RETRIES;

    while iteration < max_iterations {
        iteration += 1;

        // Cost guard pre-flight check
        if let Some(ref cg) = config.cost_guard
            && let Err(msg) = cg.check_allowed()
        {
            warn!(
                "Subagent [{}] cost guard blocked LLM call: {}",
                task_id, msg
            );
            if let Some(ref mut l) = log {
                l.log_cost_blocked(&msg);
                l.log_end("cost-blocked");
            }
            return Ok(format!("Budget limit reached: {}", msg));
        }

        let response = config
            .provider
            .chat(crate::providers::base::ChatRequest {
                messages: messages.clone(),
                tools: Some(tools.get_tool_definitions()),
                model: Some(&config.model),
                max_tokens: config.max_tokens,
                temperature: config.tool_temperature,
                tool_choice: None,
                response_format: None,
            })
            .await?;

        // Record cost for budget tracking
        if let Some(ref cg) = config.cost_guard {
            cg.record_llm_call(
                &config.model,
                response.input_tokens,
                response.output_tokens,
                response.cache_creation_input_tokens,
                response.cache_read_input_tokens,
            );
        }

        if response.has_tool_calls() {
            let call_count = response.tool_calls.len();
            info!(
                "Subagent [{}] iteration {}: {} tool call(s)",
                task_id, iteration, call_count
            );
            if let Some(ref mut l) = log {
                l.log_iteration_tool_calls(iteration, call_count);
            }

            // Add assistant message
            messages.push(Message::assistant_with_thinking(
                response.content.clone().unwrap_or_default(),
                Some(response.tool_calls.clone()),
                response.reasoning_content.clone(),
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

            // Log each tool call before execution
            if let Some(ref mut l) = log {
                for (tc, _) in &tool_lookups {
                    l.log_tool_call(&tc.name, &tc.arguments);
                }
            }

            // Execute tools in parallel through the registry middleware pipeline
            // (timeout, panic isolation, truncation, caching, logging).
            let futs: Vec<_> = tool_lookups
                .iter()
                .map(|(tc, tool_opt)| {
                    execute_subagent_tool(
                        task_id,
                        &tc.name,
                        &tc.arguments,
                        &tools,
                        tool_opt.clone(),
                        Some(&config.workspace),
                        origin,
                    )
                })
                .collect();
            let results = futures_util::future::join_all(futs).await;

            // Add all tool results to messages in order
            for ((tc, _), (result_str, is_error)) in tool_lookups.iter().zip(results.into_iter()) {
                // Log tool results
                if let Some(ref mut l) = log {
                    l.log_tool_result(&tc.name, &result_str, is_error);
                }

                // Scan tool output for prompt injection (warn only, matching main loop)
                if let Some(ref guard) = config.prompt_guard {
                    let tool_matches = guard.scan(&result_str);
                    for m in &tool_matches {
                        warn!(
                            "Subagent [{}] prompt injection in tool '{}' output ({:?}): {}",
                            task_id, tc.name, m.category, m.pattern_name
                        );
                    }
                }
                messages.push(Message::tool_result(tc.id.clone(), result_str, is_error));
            }
        } else if let Some(content) = response.content {
            if let Some(ref mut l) = log {
                l.log_iteration_text(iteration, content.len());
                l.log_end("ok");
            }
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
                if let Some(ref mut l) = log {
                    l.log_iteration_empty(iteration, empty_retries_left);
                }
                tokio::time::sleep(tokio::time::Duration::from_secs_f64(delay)).await;
                continue;
            }
            warn!(
                "Subagent [{}] empty response, no retries left - giving up",
                task_id
            );
            if let Some(ref mut l) = log {
                l.log_iteration_empty(iteration, 0);
            }
            break;
        }
    }

    if iteration >= max_iterations {
        warn!(
            "Subagent [{}] reached max iterations ({})",
            task_id, max_iterations
        );
    }
    if let Some(ref mut l) = log {
        if iteration >= max_iterations {
            l.log_max_iterations(max_iterations);
        }
        l.log_end("no-final-response");
    }

    Ok("Task completed but no final response was generated.".to_string())
}

/// Execute a single tool call for a subagent, routed through the `ToolRegistry`
/// middleware pipeline (caching, timeout, panic isolation, truncation, logging).
async fn execute_subagent_tool(
    task_id: &str,
    tool_name: &str,
    tool_args: &Value,
    registry: &ToolRegistry,
    tool_opt: Option<Arc<dyn crate::agent::tools::base::Tool>>,
    workspace: Option<&std::path::Path>,
    origin: &(String, String),
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

        let ctx = crate::agent::tools::base::ExecutionContext {
            channel: origin.0.clone(),
            chat_id: origin.1.clone(),
            context_summary: None,
            metadata: std::collections::HashMap::new(),
        };
        match registry.execute(tool_name, tool_args.clone(), &ctx).await {
            Ok(result) => (result.content, result.is_error),
            Err(e) => {
                warn!("Subagent [{}] tool '{}' failed: {}", task_id, tool_name, e);
                let msg = crate::utils::path_sanitize::sanitize_error_message(
                    &format!("Tool execution failed: {}", e),
                    workspace,
                );
                (msg, true)
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
- Send messages directly to users
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
mod tests;
