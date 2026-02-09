use crate::agent::compaction::{estimate_messages_tokens, MessageCompactor};
use crate::agent::context::ContextBuilder;
use crate::agent::memory::MemoryStore;
use crate::agent::subagent::{SubagentConfig, SubagentManager};
use crate::agent::tools::{
    cron::CronTool,
    filesystem::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool},
    github::GitHubTool,
    google_calendar::GoogleCalendarTool,
    google_mail::GoogleMailTool,
    http::HttpTool,
    message::MessageTool,
    shell::ExecTool,
    spawn::SpawnTool,
    subagent_control::SubagentControlTool,
    tmux::TmuxTool,
    todoist::TodoistTool,
    weather::WeatherTool,
    web::{WebFetchTool, WebSearchTool},
    ToolRegistry,
};
use crate::agent::truncation::truncate_tool_result;
use crate::bus::{InboundMessage, MessageBus, OutboundMessage};
use crate::cron::service::CronService;
use crate::providers::base::{LLMProvider, Message};
use crate::session::{Session, SessionManager, SessionStore};
use crate::utils::task_tracker::TaskTracker;
use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

const EMPTY_RESPONSE_RETRIES: usize = 2;

/// Core logic for executing a single tool call and producing (result_string, is_error).
async fn execute_tool_call_inner(
    _tc_id: &str,
    tc_name: &str,
    tc_args: &Value,
    tool_opt: Option<Arc<dyn crate::agent::tools::base::Tool>>,
) -> (String, bool) {
    if let Some(tool) = tool_opt {
        debug!("Executing tool: {} with arguments: {}", tc_name, tc_args);
        let tool_name = tc_name.to_string();
        let params = tc_args.clone();
        match tool.execute(params).await {
            Ok(result) => {
                if result.is_error {
                    warn!("Tool '{}' returned error: {}", tool_name, result.content);
                }
                (truncate_tool_result(&result.content, 3000), result.is_error)
            }
            Err(e) => {
                warn!("Tool '{}' failed: {}", tool_name, e);
                (format!("Tool execution failed: {}", e), true)
            }
        }
    } else {
        warn!("LLM called unknown tool: {}", tc_name);
        (format!("Error: unknown tool '{}'", tc_name), true)
    }
}

/// Execute a tool call with panic isolation (single-tool fast-path).
async fn execute_tool_call(
    tc: &crate::providers::base::ToolCallRequest,
    tool_opt: Option<Arc<dyn crate::agent::tools::base::Tool>>,
) -> (String, bool) {
    let tc_id = tc.id.clone();
    let tc_name = tc.name.clone();
    let tc_args = tc.arguments.clone();
    let handle = tokio::task::spawn(async move {
        execute_tool_call_inner(&tc_id, &tc_name, &tc_args, tool_opt).await
    });
    match handle.await {
        Ok(result) => result,
        Err(join_err) => {
            error!("Tool '{}' panicked: {:?}", tc.name, join_err);
            (format!("Tool '{}' crashed unexpectedly", tc.name), true)
        }
    }
}

/// Regex that matches phrases where the LLM claims to have performed an action.
/// Used to detect hallucinated actions when no tools were actually called.
static ACTION_CLAIM_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:I(?:'ve| have) (?:updated|written|created|set up|configured|saved|deleted|removed|added|modified|changed|installed|fixed|applied|edited|committed|deployed|sent|scheduled|enabled|disabled)|I (?:updated|wrote|created|set up|configured|saved|deleted|removed|added|modified|changed|installed|fixed|applied|edited|committed|deployed|sent|scheduled|enabled|disabled)|(?:Changes|Updates|Modifications) (?:have been|were) (?:made|applied|saved|committed)|(?:File|Config|Settings?) (?:has been|was) (?:updated|written|created|modified|saved|deleted))\b"
    )
    .expect("Invalid action claim regex")
});

/// Returns `true` if the text contains phrases claiming actions were performed.
pub fn contains_action_claims(text: &str) -> bool {
    ACTION_CLAIM_RE.is_match(text)
}

/// Configuration for creating an [`AgentLoop`] instance.
pub struct AgentLoopConfig {
    pub bus: Arc<Mutex<MessageBus>>,
    pub provider: Arc<dyn LLMProvider>,
    pub workspace: PathBuf,
    pub model: Option<String>,
    pub max_iterations: usize,
    pub brave_api_key: Option<String>,
    pub exec_timeout: u64,
    pub restrict_to_workspace: bool,
    pub allowed_commands: Vec<String>,
    pub compaction_config: crate::config::CompactionConfig,
    pub outbound_tx: Arc<tokio::sync::mpsc::Sender<OutboundMessage>>,
    pub cron_service: Option<Arc<CronService>>,
    pub google_config: Option<crate::config::GoogleConfig>,
    pub github_config: Option<crate::config::GitHubConfig>,
    pub weather_config: Option<crate::config::WeatherConfig>,
    pub todoist_config: Option<crate::config::TodoistConfig>,
    /// Temperature for response generation (default 0.7)
    pub temperature: f32,
    /// Temperature for tool-calling iterations (default 0.0 for determinism)
    pub tool_temperature: f32,
    /// Session TTL in days for cleanup (default 30)
    pub session_ttl_days: u32,
    /// Sender for typing indicator events (channel, chat_id)
    pub typing_tx: Option<Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
    /// Channel configurations for multi-channel cron target resolution
    pub channels_config: Option<crate::config::ChannelsConfig>,
}

pub struct AgentLoop {
    inbound_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<InboundMessage>>>,
    provider: Arc<dyn LLMProvider>,
    _workspace: PathBuf, // Used in constructor for context/session/memory initialization
    model: String,
    max_iterations: usize,
    context: Arc<Mutex<ContextBuilder>>,
    sessions: Arc<dyn SessionStore>,
    memory: Arc<MemoryStore>,
    tools: Arc<Mutex<ToolRegistry>>,
    compactor: Option<Arc<MessageCompactor>>,
    compaction_config: crate::config::CompactionConfig,
    _subagents: Option<Arc<SubagentManager>>,
    _processing_lock: Arc<tokio::sync::Mutex<()>>,
    running: Arc<tokio::sync::Mutex<bool>>,
    outbound_tx: Arc<tokio::sync::mpsc::Sender<OutboundMessage>>,
    task_tracker: Arc<TaskTracker>,
    temperature: f32,
    tool_temperature: f32,
    typing_tx: Option<Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
}

impl AgentLoop {
    pub async fn new(config: AgentLoopConfig) -> Result<Self> {
        let AgentLoopConfig {
            bus,
            provider,
            workspace,
            model,
            max_iterations,
            brave_api_key,
            exec_timeout,
            restrict_to_workspace,
            allowed_commands,
            compaction_config,
            outbound_tx,
            cron_service,
            google_config,
            github_config,
            weather_config,
            todoist_config,
            temperature,
            tool_temperature,
            session_ttl_days,
            typing_tx,
            channels_config,
        } = config;

        // Extract receiver to avoid lock contention
        // Receivers are !Sync, so we wrap in Arc<Mutex> for sharing
        let inbound_rx = Arc::new(tokio::sync::Mutex::new({
            let mut bus_guard = bus.lock().await;
            bus_guard
                .take_inbound_rx()
                .ok_or_else(|| anyhow::anyhow!("Inbound receiver already taken"))?
        }));
        let model = model.unwrap_or_else(|| provider.default_model().to_string());
        let context = Arc::new(Mutex::new(ContextBuilder::new(&workspace)?));
        let session_mgr = SessionManager::new(workspace.clone())?;

        // Clean up expired sessions in background
        if session_ttl_days > 0 {
            let ttl = session_ttl_days;
            let mgr_for_cleanup = SessionManager::new(workspace.clone())?;
            tokio::spawn(async move {
                if let Err(e) = mgr_for_cleanup.cleanup_old_sessions(ttl) {
                    tracing::warn!("Session cleanup failed: {}", e);
                }
            });
        }

        let sessions: Arc<dyn SessionStore> = Arc::new(session_mgr);
        let memory = Arc::new(MemoryStore::new(&workspace)?);
        // Start background memory indexer
        memory.start_indexer().await?;

        let mut tools = ToolRegistry::new();

        // Register filesystem tools
        // When restricted, allow workspace + specific config dirs (not entire home)
        let allowed_roots = if restrict_to_workspace {
            let mut roots = vec![workspace.clone()];
            if let Some(home) = dirs::home_dir() {
                roots.push(home.join(".nanobot"));
            }
            Some(roots)
        } else {
            None
        };

        let backup_dir = dirs::home_dir().map(|h| h.join(".nanobot/backups"));

        tools.register(Arc::new(ReadFileTool::new(allowed_roots.clone())));
        tools.register(Arc::new(WriteFileTool::new(
            allowed_roots.clone(),
            backup_dir.clone(),
        )));
        tools.register(Arc::new(EditFileTool::new(
            allowed_roots.clone(),
            backup_dir,
        )));
        tools.register(Arc::new(ListDirTool::new(allowed_roots)));

        // Register shell tool
        tools.register(Arc::new(ExecTool::new(
            exec_timeout,
            Some(workspace.clone()),
            restrict_to_workspace,
            allowed_commands.clone(),
        )?));

        // Register web tools
        tools.register(Arc::new(WebSearchTool::new(brave_api_key.clone(), 5)));
        tools.register(Arc::new(WebFetchTool::new(50000)?));

        // Register message tool with outbound sender
        let outbound_tx_for_tool = outbound_tx.clone();
        tools.register(Arc::new(MessageTool::new(Some(outbound_tx_for_tool))));

        // Create subagent manager
        let subagents = Arc::new(SubagentManager::new(SubagentConfig {
            provider: provider.clone(),
            workspace: workspace.clone(),
            bus: bus.clone(),
            model: Some(model.clone()),
            brave_api_key: brave_api_key.clone(),
            exec_timeout,
            restrict_to_workspace,
            allowed_commands,
        }));

        // Register spawn and subagent control tools
        let spawn_tool = Arc::new(SpawnTool::new(subagents.clone()));
        tools.register(spawn_tool.clone());
        tools.register(Arc::new(SubagentControlTool::new(subagents.clone())));

        // Register tmux tool
        tools.register(Arc::new(TmuxTool::new()));

        // Register cron tool if service provided
        if let Some(ref cron_svc) = cron_service {
            tools.register(Arc::new(CronTool::new(cron_svc.clone(), channels_config)));
        }

        // Register Google tools if configured
        if let Some(ref google_cfg) = google_config {
            if google_cfg.enabled
                && !google_cfg.client_id.is_empty()
                && !google_cfg.client_secret.is_empty()
            {
                match crate::auth::google::get_credentials(
                    &google_cfg.client_id,
                    &google_cfg.client_secret,
                    Some(&google_cfg.scopes),
                    None,
                )
                .await
                {
                    Ok(creds) => {
                        tools.register(Arc::new(GoogleMailTool::new(creds.clone())));
                        tools.register(Arc::new(GoogleCalendarTool::new(creds)));
                        info!("Google tools registered (gmail, calendar)");
                    }
                    Err(e) => {
                        warn!("Google tools not available: {}", e);
                    }
                }
            }
        }

        // Register GitHub tool if configured
        if let Some(ref gh_cfg) = github_config {
            if gh_cfg.enabled && !gh_cfg.token.is_empty() {
                tools.register(Arc::new(GitHubTool::new(gh_cfg.token.clone())));
                info!("GitHub tool registered");
            }
        }

        // Register Weather tool if configured
        if let Some(ref weather_cfg) = weather_config {
            if weather_cfg.enabled && !weather_cfg.api_key.is_empty() {
                tools.register(Arc::new(WeatherTool::new(weather_cfg.api_key.clone())));
                info!("Weather tool registered");
            }
        }

        // Register Todoist tool if configured
        if let Some(ref todoist_cfg) = todoist_config {
            if todoist_cfg.enabled && !todoist_cfg.token.is_empty() {
                tools.register(Arc::new(TodoistTool::new(todoist_cfg.token.clone())));
                info!("Todoist tool registered");
            }
        }

        // Register HTTP tool (always available, no config needed)
        tools.register(Arc::new(HttpTool::new()));

        let tools = Arc::new(Mutex::new(tools));

        let compactor = if compaction_config.enabled {
            Some(Arc::new(MessageCompactor::new(
                provider.clone() as Arc<dyn LLMProvider>,
                compaction_config.model.clone(),
            )))
        } else {
            None
        };

        Ok(Self {
            inbound_rx,
            provider,
            _workspace: workspace,
            model,
            max_iterations,
            context,
            sessions,
            memory,
            tools,
            compactor,
            compaction_config,
            _subagents: Some(subagents),
            _processing_lock: Arc::new(tokio::sync::Mutex::new(())),
            running: Arc::new(tokio::sync::Mutex::new(false)),
            outbound_tx,
            task_tracker: Arc::new(TaskTracker::new()),
            temperature,
            tool_temperature,
            typing_tx,
        })
    }

    pub async fn run(&self) -> Result<()> {
        tracing::info!("Agent loop started, waiting for messages...");
        *self.running.lock().await = true;
        info!("Agent loop started");

        loop {
            let running = {
                let guard = self.running.lock().await;
                *guard
            };
            if !running {
                break;
            }

            // Check for messages - lock receiver only for recv()
            // Note: This is necessary because receivers are !Sync
            let msg_opt = {
                let mut rx = self.inbound_rx.lock().await;
                rx.recv().await
            };

            if let Some(msg) = msg_opt {
                info!("Agent received inbound message: channel={}, sender_id={}, chat_id={}, content_len={}", 
                    msg.channel, msg.sender_id, msg.chat_id, msg.content.len());
                match self.process_message(msg).await {
                    Ok(Some(outbound_msg)) => {
                        // Send response back through the bus
                        info!("Agent generated outbound message: channel={}, chat_id={}, content_len={}", 
                            outbound_msg.channel, outbound_msg.chat_id, outbound_msg.content.len());
                        if let Err(e) = self.outbound_tx.send(outbound_msg).await {
                            error!("Failed to send outbound message: {}", e);
                        } else {
                            info!("Successfully sent outbound message to bus");
                        }
                    }
                    Ok(None) => {
                        // No response (e.g., empty after delivery tool)
                        warn!("No response generated for message (process_message returned None)");
                    }
                    Err(e) => {
                        error!("Error processing message: {}", e);
                    }
                }
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }

        info!("Agent loop stopped");
        Ok(())
    }

    pub fn stop(&self) {
        // Signal stop - use blocking call since this is called from sync context
        // If called from async context, consider making this async
        let running = self.running.clone();
        let task_tracker = self.task_tracker.clone();
        let memory = self.memory.clone();
        tokio::spawn(async move {
            {
                let mut guard = running.lock().await;
                *guard = false;
            }
            // Cancel all tracked background tasks
            task_tracker.cancel_all().await;
            // Stop the background memory indexer
            memory.stop_indexer().await;
        });
    }

    async fn process_message(&self, msg: InboundMessage) -> Result<Option<OutboundMessage>> {
        let _lock = self._processing_lock.lock().await;
        self.process_message_unlocked(msg).await
    }

    async fn process_message_unlocked(
        &self,
        msg: InboundMessage,
    ) -> Result<Option<OutboundMessage>> {
        if msg.channel == "system" {
            return self.process_system_message(msg).await;
        }

        // Send typing indicator before processing
        if let Some(ref tx) = self.typing_tx {
            let _ = tx.send((msg.channel.clone(), msg.chat_id.clone())).await;
        }

        info!("Processing message from {}:{}", msg.channel, msg.sender_id);

        // Set tool contexts
        debug!("Setting tool contexts");
        self.set_tool_contexts(&msg.channel, &msg.chat_id).await;

        let session_key = msg.session_key();
        // Reuse session to avoid repeated lookups
        debug!("Loading session: {}", session_key);
        let mut session = self.sessions.get_or_create(&session_key).await?;

        debug!("Getting compacted history");
        let history = self.get_compacted_history(&session).await?;
        debug!("Got {} history messages", history.len());

        debug!("Acquiring context lock");
        let messages = {
            let mut context = self.context.lock().await;
            context.build_messages(
                &history,
                &msg.content,
                Some(&msg.channel),
                Some(&msg.chat_id),
            )?
        };
        debug!("Built {} messages, starting agent loop", messages.len());

        let final_content = self.run_agent_loop(messages).await?;

        // Save conversation (reuse session variable)
        let extra = HashMap::new();
        session.add_message("user".to_string(), msg.content.clone(), extra.clone());
        if let Some(ref content) = final_content {
            session.add_message("assistant".to_string(), content.clone(), extra);
        }
        self.sessions.save(&session).await?;

        // Background fact extraction
        if let (Some(ref compactor), Some(ref content)) = (&self.compactor, &final_content) {
            if self.compaction_config.extraction_enabled && msg.channel != "system" {
                let compactor = compactor.clone();
                let memory = self.memory.clone();
                let user_msg = msg.content.clone();
                let assistant_msg = content.clone();
                let task_tracker = self.task_tracker.clone();
                let task_name = format!("fact_extraction_{}", chrono::Utc::now().timestamp());
                // Use spawn_auto_cleanup since this is a one-off task that should remove itself
                task_tracker
                    .spawn_auto_cleanup(task_name, async move {
                        match compactor.extract_facts(&user_msg, &assistant_msg).await {
                            Ok(facts) => {
                                if !facts.is_empty() {
                                    if let Err(e) =
                                        memory.append_today(&format!("\n## Facts\n\n{}\n", facts))
                                    {
                                        warn!("Failed to save facts to daily note: {}", e);
                                    } else {
                                        debug!(
                                            "Saved extracted facts to daily note ({} bytes)",
                                            facts.len()
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Failed to extract facts from conversation: {}", e);
                            }
                        }
                    })
                    .await;
            }
        }

        if let Some(content) = final_content {
            Ok(Some(OutboundMessage {
                channel: msg.channel,
                chat_id: msg.chat_id,
                content,
                reply_to: None,
                media: vec![],
                metadata: HashMap::new(),
            }))
        } else {
            Ok(None)
        }
    }

    async fn run_agent_loop(&self, mut messages: Vec<Message>) -> Result<Option<String>> {
        let mut empty_retries_left = EMPTY_RESPONSE_RETRIES;
        let mut last_used_delivery_tool = false;
        let mut any_tools_called = false;

        // Cache tool definitions to avoid repeated lock acquisition
        let tools_defs = {
            let tools_guard = self.tools.lock().await;
            tools_guard.get_tool_definitions()
        };

        for iteration in 1..=self.max_iterations {
            // Use retry logic for provider calls
            // Use low temperature for tool-calling iterations (determinism),
            // normal temperature for final text responses
            let current_temp = if tools_defs.is_empty() {
                self.temperature
            } else {
                self.tool_temperature
            };
            let response = self
                .provider
                .chat_with_retry(
                    crate::providers::base::ChatRequest {
                        messages: messages.clone(),
                        tools: Some(tools_defs.clone()),
                        model: Some(&self.model),
                        max_tokens: 4096,
                        temperature: current_temp,
                    },
                    Some(crate::providers::base::RetryConfig::default()),
                )
                .await?;

            if response.has_tool_calls() {
                any_tools_called = true;
                let tool_names: Vec<&str> = response
                    .tool_calls
                    .iter()
                    .map(|tc| tc.name.as_str())
                    .collect();
                last_used_delivery_tool =
                    tool_names.iter().any(|n| *n == "message" || *n == "spawn");

                ContextBuilder::add_assistant_message(
                    &mut messages,
                    response.content.as_deref(),
                    Some(response.tool_calls.clone()),
                    response.reasoning_content.as_deref(),
                );

                // Execute tools with validation
                // NOTE: We must NOT hold the tools lock across tool execution,
                // because tools like cron `run` can re-enter the agent loop
                // (via process_direct), which needs to acquire the tools lock.

                // Phase 1: Look up all tools with a single lock acquisition
                let tool_lookups: Vec<_> = {
                    let tools_guard = self.tools.lock().await;
                    response
                        .tool_calls
                        .iter()
                        .map(|tc| (tc, tools_guard.get(&tc.name)))
                        .collect()
                };
                // Lock is dropped here — safe for tools that re-enter the agent loop

                // Phase 2+3: Execute tools and collect results
                let results = if tool_lookups.len() == 1 {
                    // Single tool fast-path: avoid join_all overhead
                    let (tc, tool_opt) = &tool_lookups[0];
                    vec![execute_tool_call(tc, tool_opt.clone()).await]
                } else {
                    // Parallel execution: spawn all, await all
                    let handles: Vec<_> = tool_lookups
                        .iter()
                        .map(|(tc, tool_opt)| {
                            let tc_id = tc.id.clone();
                            let tc_name = tc.name.clone();
                            let tc_args = tc.arguments.clone();
                            let tool_opt = tool_opt.clone();
                            tokio::task::spawn(async move {
                                execute_tool_call_inner(&tc_id, &tc_name, &tc_args, tool_opt).await
                            })
                        })
                        .collect();
                    futures_util::future::join_all(handles)
                        .await
                        .into_iter()
                        .map(|join_result| match join_result {
                            Ok(result) => result,
                            Err(join_err) => {
                                error!("Tool task panicked: {:?}", join_err);
                                ("Tool crashed unexpectedly".to_string(), true)
                            }
                        })
                        .collect()
                };

                // Phase 4: Add all results to messages in order
                for ((tc, _), (result_str, is_error)) in
                    tool_lookups.iter().zip(results.into_iter())
                {
                    ContextBuilder::add_tool_result(
                        &mut messages,
                        &tc.id,
                        &tc.name,
                        &result_str,
                        is_error,
                    );
                }
            } else if let Some(content) = response.content {
                // Detect hallucinated actions: LLM claims it did something but never called tools
                if !any_tools_called && contains_action_claims(&content) {
                    warn!(
                        "Action hallucination detected: LLM claims actions but no tools were called"
                    );
                    // Add the hallucinated response then inject a correction
                    ContextBuilder::add_assistant_message(
                        &mut messages,
                        Some(&content),
                        None,
                        response.reasoning_content.as_deref(),
                    );
                    messages.push(Message::user(
                        "You claimed to have performed actions, but you did not use any tools. \
                         Do not claim to have done something you haven't. Either use the \
                         appropriate tools to actually perform the action, or explain what \
                         you would need to do."
                            .to_string(),
                    ));
                    // Allow one more iteration to self-correct
                    any_tools_called = true; // Prevent infinite correction loop
                    continue;
                }
                return Ok(Some(content));
            } else {
                // Empty response
                if last_used_delivery_tool {
                    debug!("LLM returned empty after delivery tool — treating as successful completion");
                    return Ok(None);
                }
                if empty_retries_left > 0 {
                    empty_retries_left -= 1;
                    let retry_num = EMPTY_RESPONSE_RETRIES - empty_retries_left;
                    let delay = (2_u64.pow(retry_num as u32) as f64 + fastrand::f64()).min(10.0);
                    warn!(
                        "LLM returned empty on iteration {}, retries left: {}, backing off {:.1}s",
                        iteration, empty_retries_left, delay
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs_f64(delay)).await;
                    continue;
                }
                warn!("LLM returned empty, no retries left - giving up");
                break;
            }
        }

        Ok(None)
    }

    async fn set_tool_contexts(&self, channel: &str, chat_id: &str) {
        let tools_guard = self.tools.lock().await;
        // Set context on tools that support it
        if let Some(msg_tool) = tools_guard.get("message") {
            msg_tool.set_context(channel, chat_id).await;
        }
        if let Some(cron_tool) = tools_guard.get("cron") {
            cron_tool.set_context(channel, chat_id).await;
        }
        if let Some(spawn_tool) = tools_guard.get("spawn") {
            spawn_tool.set_context(channel, chat_id).await;
        }
    }

    async fn get_compacted_history(
        &self,
        session: &Session,
    ) -> Result<Vec<HashMap<String, Value>>> {
        if self.compactor.is_none() || !self.compaction_config.enabled {
            return Ok(session.get_history(50));
        }

        let full_history = session.get_full_history();
        if full_history.is_empty() {
            return Ok(vec![]);
        }

        let keep_recent = self.compaction_config.keep_recent;
        let threshold = self.compaction_config.threshold_tokens;
        let token_est = estimate_messages_tokens(&full_history);

        if token_est < threshold as usize {
            return Ok(session.get_history(50));
        }

        if full_history.len() <= keep_recent {
            return Ok(full_history);
        }

        let old_messages = &full_history[..full_history.len() - keep_recent];
        let recent_messages = &full_history[full_history.len() - keep_recent..];

        // Get existing summary from metadata
        let previous_summary = session
            .metadata
            .get("compaction_summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Compact old messages
        if let Some(ref compactor) = self.compactor {
            match compactor.compact(old_messages, &previous_summary).await {
                Ok(summary) => {
                    // Update session metadata with new summary
                    let session_key = session.key.clone();
                    let mut updated_session = self.sessions.get_or_create(&session_key).await?;
                    updated_session.metadata.insert(
                        "compaction_summary".to_string(),
                        Value::String(summary.clone()),
                    );
                    self.sessions.save(&updated_session).await?;

                    // Return summary + recent messages
                    let mut result = vec![HashMap::from([
                        ("role".to_string(), Value::String("system".to_string())),
                        (
                            "content".to_string(),
                            Value::String(format!("[Previous conversation summary: {}]", summary)),
                        ),
                    ])];
                    result.extend(recent_messages.iter().cloned());
                    Ok(result)
                }
                Err(e) => {
                    warn!("Compaction failed: {}, returning recent messages only", e);
                    Ok(recent_messages.to_vec())
                }
            }
        } else {
            Ok(recent_messages.to_vec())
        }
    }

    async fn process_system_message(&self, msg: InboundMessage) -> Result<Option<OutboundMessage>> {
        info!("Processing system message from {}", msg.sender_id);

        let parts: Vec<&str> = msg.chat_id.splitn(2, ':').collect();
        let (origin_channel, origin_chat_id) = if parts.len() == 2 {
            (parts[0].to_string(), parts[1].to_string())
        } else {
            ("cli".to_string(), msg.chat_id.clone())
        };

        let session_key = format!("{}:{}", origin_channel, origin_chat_id);
        let session = self.sessions.get_or_create(&session_key).await?;

        let history = self.get_compacted_history(&session).await?;

        let mut context = self.context.lock().await;
        let messages = context.build_messages(
            &history,
            &msg.content,
            Some(origin_channel.as_str()),
            Some(origin_chat_id.as_str()),
        )?;

        let final_content = self
            .run_agent_loop(messages)
            .await?
            .unwrap_or_else(|| "Background task completed.".to_string());

        let mut session = self.sessions.get_or_create(&session_key).await?;
        let extra = HashMap::new();
        session.add_message(
            "user".to_string(),
            format!("[System: {}] {}", msg.sender_id, msg.content),
            extra.clone(),
        );
        session.add_message("assistant".to_string(), final_content.clone(), extra);
        self.sessions.save(&session).await?;

        Ok(Some(OutboundMessage {
            channel: origin_channel.to_string(),
            chat_id: origin_chat_id.to_string(),
            content: final_content,
            reply_to: None,
            media: vec![],
            metadata: HashMap::new(),
        }))
    }

    pub async fn process_direct(
        &self,
        content: &str,
        session_key: &str,
        channel: &str,
        chat_id: &str,
    ) -> Result<String> {
        let session = self.sessions.get_or_create(session_key).await?;
        let history = self.get_compacted_history(&session).await?;

        let messages = {
            let mut ctx = self.context.lock().await;
            ctx.build_messages(&history, content, Some(channel), Some(chat_id))?
        };

        let response = self
            .run_agent_loop(messages)
            .await?
            .unwrap_or_else(|| "No response generated.".to_string());

        let mut session = self.sessions.get_or_create(session_key).await?;
        let extra = HashMap::new();
        session.add_message("user".to_string(), content.to_string(), extra.clone());
        session.add_message("assistant".to_string(), response.clone(), extra);
        self.sessions.save(&session).await?;

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detects_ive_updated() {
        assert!(contains_action_claims(
            "I've updated the configuration file."
        ));
    }

    #[test]
    fn test_detects_i_have_created() {
        assert!(contains_action_claims(
            "I have created the new module for you."
        ));
    }

    #[test]
    fn test_detects_i_wrote() {
        assert!(contains_action_claims("I wrote the function as requested."));
    }

    #[test]
    fn test_detects_i_deleted() {
        assert!(contains_action_claims("I deleted the old config."));
    }

    #[test]
    fn test_detects_ive_configured() {
        assert!(contains_action_claims("I've configured the settings."));
    }

    #[test]
    fn test_detects_ive_saved() {
        assert!(contains_action_claims("I've saved the changes to disk."));
    }

    #[test]
    fn test_detects_ive_scheduled() {
        assert!(contains_action_claims("I've scheduled the cron job."));
    }

    #[test]
    fn test_detects_passive_changes_applied() {
        assert!(contains_action_claims(
            "Changes have been applied to the project."
        ));
    }

    #[test]
    fn test_detects_passive_file_updated() {
        assert!(contains_action_claims(
            "File has been updated successfully."
        ));
    }

    #[test]
    fn test_detects_passive_config_was_modified() {
        assert!(contains_action_claims("Config was modified as requested."));
    }

    #[test]
    fn test_no_match_informational() {
        assert!(!contains_action_claims(
            "Here's how you can update the file."
        ));
    }

    #[test]
    fn test_no_match_question() {
        assert!(!contains_action_claims(
            "Would you like me to create a new file?"
        ));
    }

    #[test]
    fn test_no_match_explanation() {
        assert!(!contains_action_claims(
            "The function returns a string value."
        ));
    }

    #[test]
    fn test_no_match_plan() {
        assert!(!contains_action_claims(
            "To update the config, you need to edit settings.json."
        ));
    }

    #[test]
    fn test_no_match_greeting() {
        assert!(!contains_action_claims("Hello! How can I help you today?"));
    }

    #[test]
    fn test_no_match_partial() {
        // "I updated" should match, but "you updated" should not
        assert!(!contains_action_claims("You updated the file yesterday."));
    }

    #[test]
    fn test_case_insensitive() {
        assert!(contains_action_claims("I'VE UPDATED THE FILE."));
        assert!(contains_action_claims("i've written the code."));
    }

    #[test]
    fn test_mixed_content_with_claim() {
        assert!(contains_action_claims(
            "Sure, here's what I did:\n\nI've updated the configuration to use the new API endpoint.\nLet me know if you need anything else."
        ));
    }

    #[test]
    fn test_detects_i_enabled() {
        assert!(contains_action_claims("I enabled the feature flag."));
    }

    #[test]
    fn test_detects_ive_deployed() {
        assert!(contains_action_claims("I've deployed the changes."));
    }

    #[test]
    fn test_detects_updates_were_made() {
        assert!(contains_action_claims(
            "Updates were made to the database schema."
        ));
    }

    // --- Parallel tool execution tests ---

    use crate::agent::tools::base::{Tool, ToolResult};
    use crate::providers::base::ToolCallRequest;
    use async_trait::async_trait;
    use std::sync::Arc;

    /// A mock tool that sleeps for a duration then returns a result.
    struct MockTool {
        tool_name: String,
        delay_ms: u64,
        response: String,
    }

    #[async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            &self.tool_name
        }
        fn description(&self) -> &str {
            "mock"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(&self, _params: serde_json::Value) -> anyhow::Result<ToolResult> {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            Ok(ToolResult::new(self.response.clone()))
        }
    }

    /// A mock tool that returns an error.
    struct ErrorTool;

    #[async_trait]
    impl Tool for ErrorTool {
        fn name(&self) -> &str {
            "error_tool"
        }
        fn description(&self) -> &str {
            "mock"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(&self, _params: serde_json::Value) -> anyhow::Result<ToolResult> {
            Err(anyhow::anyhow!("intentional error"))
        }
    }

    /// A mock tool that panics.
    struct PanicTool;

    #[async_trait]
    impl Tool for PanicTool {
        fn name(&self) -> &str {
            "panic_tool"
        }
        fn description(&self) -> &str {
            "mock"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(&self, _params: serde_json::Value) -> anyhow::Result<ToolResult> {
            panic!("intentional panic");
        }
    }

    fn make_tool_call(id: &str, name: &str) -> ToolCallRequest {
        ToolCallRequest {
            id: id.to_string(),
            name: name.to_string(),
            arguments: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn test_parallel_tool_execution_ordering() {
        // 3 tools with different delays — results must come back in call order
        let tools: Vec<Option<Arc<dyn Tool>>> = vec![
            Some(Arc::new(MockTool {
                tool_name: "slow".into(),
                delay_ms: 80,
                response: "slow_result".into(),
            })),
            Some(Arc::new(MockTool {
                tool_name: "fast".into(),
                delay_ms: 10,
                response: "fast_result".into(),
            })),
            Some(Arc::new(MockTool {
                tool_name: "medium".into(),
                delay_ms: 40,
                response: "medium_result".into(),
            })),
        ];

        let calls = [
            make_tool_call("1", "slow"),
            make_tool_call("2", "fast"),
            make_tool_call("3", "medium"),
        ];

        // Spawn in parallel (same pattern as the production code)
        let handles: Vec<_> = calls
            .iter()
            .zip(tools.iter())
            .map(|(tc, tool_opt)| {
                let tc_id = tc.id.clone();
                let tc_name = tc.name.clone();
                let tc_args = tc.arguments.clone();
                let tool_opt = tool_opt.clone();
                tokio::task::spawn(async move {
                    execute_tool_call_inner(&tc_id, &tc_name, &tc_args, tool_opt).await
                })
            })
            .collect();

        let results: Vec<_> = futures_util::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // Results must be in the same order as the calls, not execution completion order
        assert_eq!(results[0].0, "slow_result");
        assert_eq!(results[1].0, "fast_result");
        assert_eq!(results[2].0, "medium_result");
        assert!(!results[0].1);
        assert!(!results[1].1);
        assert!(!results[2].1);
    }

    #[tokio::test]
    async fn test_single_tool_no_parallel_overhead() {
        let tool: Option<Arc<dyn Tool>> = Some(Arc::new(MockTool {
            tool_name: "only".into(),
            delay_ms: 0,
            response: "only_result".into(),
        }));

        let tc = make_tool_call("1", "only");
        let (result, is_error) = execute_tool_call(&tc, tool).await;

        assert_eq!(result, "only_result");
        assert!(!is_error);
    }

    #[tokio::test]
    async fn test_parallel_tool_one_panics() {
        let tools: Vec<Option<Arc<dyn Tool>>> = vec![
            Some(Arc::new(MockTool {
                tool_name: "good1".into(),
                delay_ms: 0,
                response: "result1".into(),
            })),
            Some(Arc::new(PanicTool)),
            Some(Arc::new(MockTool {
                tool_name: "good2".into(),
                delay_ms: 0,
                response: "result2".into(),
            })),
        ];

        let calls = [
            make_tool_call("1", "good1"),
            make_tool_call("2", "panic_tool"),
            make_tool_call("3", "good2"),
        ];

        let handles: Vec<_> = calls
            .iter()
            .zip(tools.iter())
            .map(|(tc, tool_opt)| {
                let tc_id = tc.id.clone();
                let tc_name = tc.name.clone();
                let tc_args = tc.arguments.clone();
                let tool_opt = tool_opt.clone();
                tokio::task::spawn(async move {
                    execute_tool_call_inner(&tc_id, &tc_name, &tc_args, tool_opt).await
                })
            })
            .collect();

        let results: Vec<_> = futures_util::future::join_all(handles)
            .await
            .into_iter()
            .map(|join_result| match join_result {
                Ok(result) => result,
                Err(_) => ("Tool crashed unexpectedly".to_string(), true),
            })
            .collect();

        // Good tools succeed
        assert_eq!(results[0].0, "result1");
        assert!(!results[0].1);
        assert_eq!(results[2].0, "result2");
        assert!(!results[2].1);
        // Panicked tool gets error
        assert!(results[1].0.contains("crashed unexpectedly"));
        assert!(results[1].1);
    }

    #[tokio::test]
    async fn test_parallel_tool_one_errors() {
        let tools: Vec<Option<Arc<dyn Tool>>> = vec![
            Some(Arc::new(MockTool {
                tool_name: "good".into(),
                delay_ms: 0,
                response: "good_result".into(),
            })),
            Some(Arc::new(ErrorTool)),
            Some(Arc::new(MockTool {
                tool_name: "also_good".into(),
                delay_ms: 0,
                response: "also_good_result".into(),
            })),
        ];

        let calls = [
            make_tool_call("1", "good"),
            make_tool_call("2", "error_tool"),
            make_tool_call("3", "also_good"),
        ];

        let handles: Vec<_> = calls
            .iter()
            .zip(tools.iter())
            .map(|(tc, tool_opt)| {
                let tc_id = tc.id.clone();
                let tc_name = tc.name.clone();
                let tc_args = tc.arguments.clone();
                let tool_opt = tool_opt.clone();
                tokio::task::spawn(async move {
                    execute_tool_call_inner(&tc_id, &tc_name, &tc_args, tool_opt).await
                })
            })
            .collect();

        let results: Vec<_> = futures_util::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // Good tools unaffected
        assert_eq!(results[0].0, "good_result");
        assert!(!results[0].1);
        assert_eq!(results[2].0, "also_good_result");
        assert!(!results[2].1);
        // Error tool marked as error
        assert!(results[1].0.contains("Tool execution failed"));
        assert!(results[1].1);
    }
}
