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
use tracing::{debug, info, warn};
use uuid::Uuid;

const MAX_TOOL_RESULT_CHARS: usize = 10000;
const EMPTY_RESPONSE_RETRIES: usize = 2;
const MAX_WEB_FETCH_CHARS: usize = 50000;
const MAX_SUBAGENT_ITERATIONS: usize = 15;

pub struct SubagentConfig {
    pub provider: Arc<dyn LLMProvider>,
    pub workspace: PathBuf,
    pub bus: Arc<Mutex<MessageBus>>,
    pub model: Option<String>,
    pub brave_api_key: Option<String>,
    pub exec_timeout: u64,
    pub restrict_to_workspace: bool,
    pub allowed_commands: Vec<String>,
    pub max_tokens: u32,
    pub tool_temperature: f32,
}

pub struct SubagentManager {
    provider: Arc<dyn LLMProvider>,
    workspace: PathBuf,
    bus: Arc<Mutex<MessageBus>>,
    model: String,
    brave_api_key: Option<String>,
    exec_timeout: u64,
    restrict_to_workspace: bool,
    allowed_commands: Vec<String>,
    max_tokens: u32,
    tool_temperature: f32,
    running_tasks: Arc<tokio::sync::Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
}

impl SubagentManager {
    pub fn new(config: SubagentConfig) -> Self {
        let model = config
            .model
            .unwrap_or_else(|| config.provider.default_model().to_string());
        Self {
            provider: config.provider,
            workspace: config.workspace,
            bus: config.bus,
            model,
            brave_api_key: config.brave_api_key,
            exec_timeout: config.exec_timeout,
            restrict_to_workspace: config.restrict_to_workspace,
            allowed_commands: config.allowed_commands,
            max_tokens: config.max_tokens,
            tool_temperature: config.tool_temperature,
            running_tasks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    pub async fn spawn(
        &self,
        task: String,
        label: Option<String>,
        origin_channel: String,
        origin_chat_id: String,
    ) -> Result<String> {
        let task_id = Uuid::new_v4().to_string()[..8].to_string();
        let display_label = label.unwrap_or_else(|| {
            let truncated: String = task.chars().take(30).collect();
            if truncated.len() < task.len() {
                format!("{}...", truncated)
            } else {
                task.clone()
            }
        });
        let display_label_clone = display_label.clone();
        let task_id_clone = task_id.clone();

        let origin = (origin_channel.clone(), origin_chat_id.clone());

        let manager = SubagentManager {
            provider: self.provider.clone(),
            workspace: self.workspace.clone(),
            bus: self.bus.clone(),
            model: self.model.clone(),
            brave_api_key: self.brave_api_key.clone(),
            exec_timeout: self.exec_timeout,
            restrict_to_workspace: self.restrict_to_workspace,
            allowed_commands: self.allowed_commands.clone(),
            max_tokens: self.max_tokens,
            tool_temperature: self.tool_temperature,
            running_tasks: self.running_tasks.clone(),
        };

        let bg_task = tokio::spawn(async move {
            manager
                .run_subagent(task_id_clone, task, display_label_clone, origin)
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

    async fn run_subagent(
        &self,
        task_id: String,
        task: String,
        label: String,
        origin: (String, String),
    ) {
        info!("Subagent [{}] starting task: {}", task_id, label);

        let result = self
            .run_subagent_inner(&task_id, &task, &label, &origin)
            .await;

        // Cleanup
        self.running_tasks.lock().await.remove(&task_id);

        match result {
            Ok(final_result) => {
                info!("Subagent [{}] completed successfully", task_id);
                self.announce_result(&task_id, &label, &task, &final_result, &origin, "ok")
                    .await;
            }
            Err(e) => {
                warn!("Subagent [{}] failed: {}", task_id, e);
                self.announce_result(
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

    async fn run_subagent_inner(
        &self,
        task_id: &str,
        task: &str,
        _label: &str,
        _origin: &(String, String),
    ) -> Result<String> {
        // Build tools
        let mut tools = ToolRegistry::new();
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let allowed_roots = if self.restrict_to_workspace {
            Some(vec![self.workspace.clone(), home.clone()])
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
            self.exec_timeout,
            Some(self.workspace.clone()),
            self.restrict_to_workspace,
            self.allowed_commands.clone(),
        )?));
        tools.register(Arc::new(WebSearchTool::new(self.brave_api_key.clone(), 5)));
        tools.register(Arc::new(WebFetchTool::new(MAX_WEB_FETCH_CHARS)?));

        // Build messages
        let system_prompt = self.build_subagent_prompt(task);
        let mut messages = vec![Message::system(system_prompt), Message::user(task)];

        // Run agent loop
        let max_iterations = MAX_SUBAGENT_ITERATIONS;
        let mut iteration = 0;
        let mut empty_retries_left = EMPTY_RESPONSE_RETRIES;

        while iteration < max_iterations {
            iteration += 1;

            let response = self
                .provider
                .chat(crate::providers::base::ChatRequest {
                    messages: messages.clone(),
                    tools: Some(tools.get_tool_definitions()),
                    model: Some(&self.model),
                    max_tokens: self.max_tokens,
                    temperature: self.tool_temperature,
                    tool_choice: None,
                })
                .await?;

            if response.has_tool_calls() {
                // Add assistant message
                messages.push(Message::assistant(
                    response.content.clone().unwrap_or_default(),
                    Some(response.tool_calls.clone()),
                ));

                // Execute tools
                for tool_call in &response.tool_calls {
                    debug!(
                        "Subagent [{}] executing: {} with arguments: {}",
                        task_id, tool_call.name, tool_call.arguments
                    );

                    // Validate params before execution
                    if let Some(tool) = tools.get(&tool_call.name) {
                        if let Some(validation_error) =
                            crate::agent::agent_loop::validate_tool_params(
                                tool.as_ref(),
                                &tool_call.arguments,
                            )
                        {
                            warn!(
                                "Subagent [{}] tool '{}' param validation failed: {}",
                                task_id, tool_call.name, validation_error
                            );
                            messages.push(Message::tool_result(
                                tool_call.id.clone(),
                                validation_error,
                                true,
                            ));
                            continue;
                        }
                    }

                    let result = tools
                        .execute(&tool_call.name, tool_call.arguments.clone())
                        .await?;
                    let result_str = truncate_tool_result(&result.content, MAX_TOOL_RESULT_CHARS);
                    messages.push(Message::tool_result(
                        tool_call.id.clone(),
                        result_str,
                        result.is_error,
                    ));
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

    async fn announce_result(
        &self,
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

        if let Err(e) = self.bus.lock().await.publish_inbound(msg).await {
            warn!("Failed to publish inbound message from subagent: {}", e);
        }
        debug!(
            "Subagent [{}] announced result to {}:{}",
            task_id, origin.0, origin.1
        );
    }

    fn build_subagent_prompt(&self, task: &str) -> String {
        format!(
            r"# Subagent

You are a subagent spawned by the main agent to complete a specific task.

## Your Task
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
- Access the main agent's conversation history

## Workspace
Your workspace is at: {}

When you have completed the task, provide a clear summary of your findings or actions.",
            task,
            self.workspace.display()
        )
    }

    pub async fn list_running(&self) -> Vec<HashMap<String, Value>> {
        let tasks = self.running_tasks.lock().await;
        tasks
            .iter()
            .map(|(id, handle)| {
                let mut map = HashMap::new();
                map.insert("id".to_string(), Value::String(id.clone()));
                map.insert("done".to_string(), Value::Bool(handle.is_finished()));
                map.insert("cancelled".to_string(), Value::Bool(false)); // Can't easily check cancellation
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
}
