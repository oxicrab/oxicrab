mod compaction_history;
mod complexity;
pub mod config;
mod helpers;
mod iteration;
mod metadata;
mod model_gateway;
mod processing;
mod reflection;
mod replay;

#[cfg(test)]
use crate::agent::tools::base::ExecutionContext;
#[cfg(test)]
use helpers::ACTION_CLAIM_PATTERNS;
#[cfg(test)]
use helpers::MAX_IMAGES;
use helpers::cleanup_old_media;
pub use helpers::contains_action_claims;
pub(crate) use helpers::validate_tool_params;
#[cfg(test)]
use helpers::{
    execute_tool_call, extract_media_paths, load_and_encode_images, strip_document_tags,
    strip_think_tags,
};
#[cfg(test)]
use iteration::{classify_tool_call_concurrency, partition_into_waves};

pub use config::{
    AgentLoopConfig, AgentLoopResult, AgentLoopRuntimeParams, AgentRunOverrides, DirectResult,
    LifecycleConfig, SafetyConfig, ToolConfigs,
};

use crate::agent::compaction::MessageCompactor;
use crate::agent::context::ContextBuilder;
use crate::agent::memory::MemoryStore;
use crate::agent::subagent::{SubagentConfig, SubagentManager};
use crate::agent::tools::ToolRegistry;
use crate::agent::tools::setup::ToolBuildContext;
use crate::bus::{InboundMessage, OutboundMessage};
use crate::cron::event_matcher::EventMatcher;
use crate::cron::service::CronService;
use crate::providers::base::LLMProvider;
use crate::safety::LeakDetector;
use crate::session::{SessionManager, SessionStore};
use crate::utils::task_tracker::TaskTracker;
use anyhow::Result;
use dashmap::DashMap;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};
use tracing::{debug, error, info, warn};

const EMPTY_RESPONSE_RETRIES: usize = 2;
const WRAPUP_THRESHOLD_RATIO: f64 = 0.7;
const MIN_WRAPUP_ITERATION: usize = 2;
const RETRY_BACKOFF_BASE: u64 = 2;
const MAX_RETRY_DELAY_SECS: f64 = 10.0;
const DEFAULT_HISTORY_SIZE: usize = 50;
const RECOVERY_CONTEXT_MAX_CHARS: usize = 200;
const MAX_COMPACTION_STATE_SESSIONS: usize = 1024;
/// Pre-flight token estimation: chars-per-token ratio (conservative)
pub(crate) const CHARS_PER_TOKEN_ESTIMATE: usize = 4;
/// Pre-flight compaction threshold as fraction of context limit (80%)
const PREFLIGHT_COMPACTION_RATIO: usize = 4; // numerator of 4/5
/// Maximum pending messages per session before new arrivals are dropped.
const MAX_PENDING_MESSAGES_PER_SESSION: usize = 10;

/// Per-session state: a processing lock plus a queue for messages that arrive
/// while the lock is held. Messages in the queue are coalesced and processed
/// as a single turn once the current run completes.
struct SessionState {
    processing: tokio::sync::Mutex<()>,
    pending: std::sync::Mutex<Vec<InboundMessage>>,
}

struct CachedSemanticIndex {
    signature: u64,
    index: crate::router::semantic::SemanticToolIndex,
}

#[derive(Default)]
struct SessionCompactionState {
    last_checkpoint: Option<String>,
    cognitive_breadcrumb: Option<String>,
    checkpoint_handle: Option<tokio::task::JoinHandle<()>>,
}

pub struct AgentLoop {
    inbound_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<InboundMessage>>>,
    bus: Arc<crate::bus::MessageBus>,
    provider: Arc<dyn LLMProvider>,
    workspace: PathBuf,
    model: String,
    max_iterations: usize,
    context: Arc<Mutex<ContextBuilder>>,
    sessions: Arc<dyn SessionStore>,
    memory: Arc<MemoryStore>,
    tools: Arc<ToolRegistry>,
    compactor: Option<Arc<MessageCompactor>>,
    compaction_config: crate::config::CompactionConfig,
    _subagents: Option<Arc<SubagentManager>>,
    /// Per-session state: a processing lock plus a pending-message queue.
    /// Serializes message processing per session while allowing independent
    /// sessions to be processed concurrently. Messages arriving during an
    /// active run are queued and coalesced into the next turn.
    session_states: Arc<DashMap<String, Arc<SessionState>>>,
    running: Arc<tokio::sync::Mutex<bool>>,
    shutdown_notify: Arc<Notify>,
    task_tracker: Arc<TaskTracker>,
    temperature: Option<f32>,
    tool_temperature: Option<f32>,
    max_tokens: u32,
    typing_tx: Option<Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
    transcriber: Option<Arc<crate::utils::transcription::LazyTranscriptionService>>,
    event_matcher: Option<std::sync::Mutex<EventMatcher>>,
    /// Epoch-seconds timestamp of last event matcher rebuild (atomic to avoid
    /// blocking the async runtime with a `std::sync::Mutex`)
    event_matcher_last_rebuild: Arc<std::sync::atomic::AtomicU64>,
    cron_service: Option<Arc<CronService>>,
    /// Per-session checkpoint state used for compaction recovery.
    compaction_state: Arc<Mutex<LruCache<String, SessionCompactionState>>>,
    cognitive_config: crate::config::CognitiveConfig,
    /// Exfiltration guard: hides outbound tools from the LLM
    exfiltration_guard: crate::config::ExfiltrationGuardConfig,
    /// Prompt injection detection guard
    prompt_guard: Option<crate::safety::prompt_guard::PromptGuard>,
    prompt_guard_config: crate::config::PromptGuardConfig,
    /// Inbound secret leak detector — scans user messages before they reach the LLM
    leak_detector: Arc<LeakDetector>,
    /// MCP manager — kept for graceful child process shutdown via `stop()`
    mcp_manager: Arc<tokio::sync::Mutex<Option<crate::agent::tools::mcp::McpManager>>>,
    /// Pre-resolved model routing for task-specific provider selection
    routing: Option<Arc<crate::config::routing::ResolvedRouting>>,
    /// Complexity scorer for per-message model routing (None when disabled)
    complexity_scorer: Option<complexity::ComplexityScorer>,
    /// Request-scoped activation set for deferred tools discovered via `tool_search`
    tool_search_activated: crate::agent::tools::tool_search::ActivatedTools,
    /// Request-scoped state for interactive buttons (written by `add_buttons`, read after loop)
    pending_buttons: crate::agent::tools::interactive::PendingButtons,
    /// Priority-ordered message router for direct dispatch and guided LLM paths
    router: std::sync::Arc<crate::router::MessageRouter>,
    /// Semantic filter size (top-k tools) for no-context LLM turns.
    semantic_top_k: usize,
    /// Lexical prefilter size before semantic rerank.
    semantic_prefilter_k: usize,
    /// Minimum semantic score for retaining a candidate tool.
    semantic_threshold: f32,
    /// Cached semantic index rebuilt when visible tool definitions change.
    semantic_index_cache: Arc<tokio::sync::Mutex<Option<CachedSemanticIndex>>>,
    /// Shared approval store for pending operator approval requests.
    approval_store: Arc<crate::agent::approval::ApprovalStore>,
    /// Operator approval workflow configuration.
    approval_config: crate::config::ApprovalConfig,
    /// Reflexion-style failure reflection configuration.
    reflection_config: crate::config::ReflectionConfig,
    /// Sender for outbound messages (approval requests, user feedback).
    outbound_tx: Arc<tokio::sync::mpsc::Sender<crate::bus::OutboundMessage>>,
}

impl AgentLoop {
    pub async fn new(config: AgentLoopConfig) -> Result<Self> {
        let AgentLoopConfig {
            bus,
            provider,
            workspace,
            model,
            max_iterations,
            compaction_config,
            outbound_tx,
            cron_service,
            temperature,
            tool_temperature,
            per_provider_temperature,
            max_tokens,
            typing_tx,
            max_concurrent_subagents,
            voice_config,
            memory_config,
            cognitive_config,
            context_providers,
            tool_configs,
            routing,
            lifecycle:
                LifecycleConfig {
                    session_ttl_days,
                    media_ttl_days,
                },
            safety:
                SafetyConfig {
                    exfiltration_guard,
                    prompt_guard: prompt_guard_config,
                },
            memory_db: shared_db,
            leak_detector: shared_leak_detector,
            router_config,
            approval_config,
            reflection_config,
        } = config;

        // Extract receiver from the bus (called once at startup).
        // Receivers are !Sync, so we wrap in Arc<Mutex> for sharing.
        let inbound_rx = Arc::new(tokio::sync::Mutex::new(
            bus.take_inbound_rx()
                .ok_or_else(|| anyhow::anyhow!("Inbound receiver already taken"))?,
        ));
        let model = model.unwrap_or_else(|| provider.default_model().to_string());

        // Reuse a pre-opened MemoryDB when available (avoids duplicate connections)
        let memory = Arc::new(if let Some(db) = shared_db {
            if let Some(ref mem_cfg) = memory_config {
                MemoryStore::with_db_and_config(db, mem_cfg)
            } else {
                MemoryStore::with_db(db)
            }
        } else if let Some(ref mem_cfg) = memory_config {
            MemoryStore::with_config(&workspace, mem_cfg)?
        } else {
            MemoryStore::new(&workspace)?
        });

        // Reuse the same MemoryDB for session management (avoids opening a third connection)
        let sessions: Arc<dyn SessionStore> = Arc::new(SessionManager::with_db(memory.db()));

        // Share the (embedding-configured) memory store with context builder
        let mut context_builder = ContextBuilder::with_memory(&workspace, memory.clone())?;
        if !context_providers.is_empty() {
            use crate::agent::context::providers::ContextProviderRunner;
            let runner = Arc::new(ContextProviderRunner::new(context_providers));
            context_builder.set_providers(runner);
        }
        let context = Arc::new(Mutex::new(context_builder));

        // Clean up expired sessions in background (reuse shared DB)
        if session_ttl_days > 0 {
            let ttl = session_ttl_days;
            let mgr_for_cleanup = SessionManager::with_db(memory.db());
            tokio::spawn(async move {
                if let Err(e) = mgr_for_cleanup.cleanup_old_sessions(ttl).await {
                    warn!("Session cleanup failed: {}", e);
                }
            });
        }

        // Clean up old media files in background (blocking I/O, not on reactor)
        if media_ttl_days > 0 {
            let ttl = media_ttl_days;
            tokio::task::spawn_blocking(move || {
                if let Err(e) = cleanup_old_media(ttl) {
                    warn!("Media cleanup failed: {}", e);
                }
            });
        }

        // Run memory hygiene in background (search log purge, workspace file cleanup)
        {
            let db = memory.db();
            let ws = workspace.clone();
            let ttl_map = tool_configs.workspace_ttl.to_map();
            let mem_retention_days = memory_config.as_ref().map_or(180, |c| c.retention_days);
            tokio::task::spawn_blocking(move || {
                crate::agent::memory::hygiene::run_hygiene(&db, 90, mem_retention_days);
                if let Err(e) =
                    crate::agent::memory::hygiene::cleanup_workspace_files(&db, &ws, &ttl_map)
                {
                    warn!("workspace file cleanup failed: {}", e);
                }
            });
        }

        let workspace_manager = Some(Arc::new(crate::agent::workspace::WorkspaceManager::new(
            workspace.clone(),
            Some(memory.db()),
        )));

        let pending_buttons = crate::agent::tools::interactive::new_pending_buttons();

        let leak_detector = shared_leak_detector.unwrap_or_else(|| Arc::new(LeakDetector::new()));

        let tool_ctx = ToolBuildContext {
            workspace: workspace.clone(),
            restrict_to_workspace: tool_configs.restrict_to_workspace,
            exec_timeout: tool_configs.exec_timeout,
            outbound_tx: outbound_tx.clone(),
            bus: bus.clone(),
            web_search_config: tool_configs.web_search_config,
            cron_service: cron_service.clone(),
            channels_config: tool_configs.channels_config,
            google_config: tool_configs.google_config,
            github_config: tool_configs.github_config,
            weather_config: tool_configs.weather_config,
            todoist_config: tool_configs.todoist_config,
            media_config: tool_configs.media_config,
            obsidian_config: tool_configs.obsidian_config,
            browser_config: tool_configs.browser_config,
            image_gen_config: tool_configs.image_gen_config,
            memory: memory.clone(),
            subagent_config: {
                let (sa_provider, sa_model) = if let Some(ref r) = routing {
                    let o = r.resolve_overrides("subagent");
                    if let Some(p) = o.provider {
                        (p, o.model.or_else(|| Some(model.clone())))
                    } else {
                        (provider.clone(), Some(model.clone()))
                    }
                } else {
                    (provider.clone(), Some(model.clone()))
                };
                SubagentConfig {
                    provider: sa_provider,
                    workspace: workspace.clone(),
                    model: sa_model,
                    max_tokens,
                    tool_temperature,
                    max_concurrent: max_concurrent_subagents,
                    prompt_guard_config: prompt_guard_config.clone(),
                    exfil_guard: exfiltration_guard.clone(),
                    main_tools: None, // set after register_all_tools()
                    memory_db: Some(memory.db()),
                    leak_detector: leak_detector.clone(),
                }
            },
            allowed_commands: tool_configs.allowed_commands,
            mcp_config: tool_configs.mcp_config,
            sandbox_config: tool_configs.sandbox_config,
            memory_db: Some(memory.db()),
            workspace_manager,
            workspace_ttl: tool_configs.workspace_ttl,
            pending_buttons: pending_buttons.clone(),
            rss_config: tool_configs.rss_config,
            leak_detector: leak_detector.clone(),
        };

        let (
            tools,
            subagents,
            mcp_manager,
            tool_search_activated,
            _shared_tool_index,
            collections_registry_handle,
        ) = crate::agent::tools::setup::register_all_tools(&tool_ctx).await?;
        let tools = Arc::new(tools);
        subagents.set_main_tools(tools.clone());

        // Wire up the collections tool's registry handle so it can register
        // per-collection data tools at runtime.
        if let Some(handle) = collections_registry_handle {
            let _ = handle.set(tools.clone());
        }

        // Warn about built-in tools with mutating actions that have no approval gate.
        // Only runs when the interactive approval workflow is disabled.
        if !approval_config.enabled {
            for tool_name in tools.tool_names() {
                if let Some(tool) = tools.get(&tool_name) {
                    let caps = tool.capabilities();
                    // Skip MCP tools (separately gated by trust level)
                    if !caps.built_in {
                        continue;
                    }
                    // Skip tools that have requires_approval_for_action overrides
                    let has_legacy_gate = caps
                        .actions
                        .iter()
                        .any(|a| !a.read_only && tool.requires_approval_for_action(a.name));
                    if has_legacy_gate {
                        continue;
                    }
                    // Warn about unprotected mutating actions
                    let mutating: Vec<&str> = caps
                        .actions
                        .iter()
                        .filter(|a| !a.read_only)
                        .map(|a| a.name)
                        .collect();
                    if !mutating.is_empty() {
                        warn!(
                            "tool '{}' has mutating actions ({}) without approval gating",
                            tool_name,
                            mutating.join(", ")
                        );
                    }
                }
            }
        }

        let transcriber = voice_config
            .as_ref()
            .filter(|vc| vc.transcription.enabled)
            .map(|vc| {
                Arc::new(crate::utils::transcription::LazyTranscriptionService::new(
                    vc.transcription.clone(),
                ))
            });

        let compactor = if compaction_config.enabled {
            let (comp_provider, comp_model, comp_temp_override) = if let Some(ref r) = routing {
                let o = r.resolve_overrides("compaction");
                if let Some(p) = o.provider {
                    // Routing overrides the compaction provider. Still apply
                    // per_provider_temperature — the compaction model may
                    // require a fixed temperature (e.g. Moonshot kimi-k2.5
                    // requires temperature=1).
                    (p, o.model, per_provider_temperature)
                } else {
                    (
                        provider.clone() as Arc<dyn LLMProvider>,
                        compaction_config.model.clone(),
                        per_provider_temperature,
                    )
                }
            } else {
                (
                    provider.clone() as Arc<dyn LLMProvider>,
                    compaction_config.model.clone(),
                    per_provider_temperature,
                )
            };
            Some(Arc::new(MessageCompactor::with_temperature_override(
                comp_provider,
                comp_model,
                comp_temp_override,
            )))
        } else {
            None
        };

        // Build event matcher from cron jobs. Always create the matcher when
        // cron_service exists so that new event-triggered jobs added after
        // startup can be picked up by the periodic rebuild.
        let event_matcher = if let Some(ref cron_svc) = cron_service {
            let matcher = match cron_svc.list_jobs(true) {
                Ok(jobs) => {
                    let m = EventMatcher::from_jobs(&jobs);
                    if !m.is_empty() {
                        info!(
                            "Event matcher initialized with {} event-triggered job(s)",
                            jobs.iter()
                                .filter(|j| matches!(
                                    j.schedule,
                                    crate::cron::types::CronSchedule::Event { .. }
                                ))
                                .count()
                        );
                    }
                    m
                }
                Err(e) => {
                    warn!("Failed to load cron jobs for event matcher: {}", e);
                    EventMatcher::from_jobs(&[])
                }
            };
            Some(std::sync::Mutex::new(matcher))
        } else {
            None
        };

        // Build message router from tool-declared static rules and config rules
        let config_rules: Vec<crate::router::rules::ConfigRule> = router_config
            .rules
            .into_iter()
            .map(|r| crate::router::rules::ConfigRule {
                trigger: r.trigger,
                tool: r.tool,
                params: r.params,
            })
            .collect();
        let semantic_top_k = router_config.semantic_top_k.max(1);
        let semantic_prefilter_k = router_config.semantic_prefilter_k.max(semantic_top_k);
        let semantic_threshold = router_config.semantic_threshold.clamp(-1.0, 1.0);
        let router = std::sync::Arc::new(crate::router::MessageRouter::with_remember_checker(
            tools.routing_rules().to_vec(),
            config_rules,
            router_config.prefix,
            Some(Box::new(|msg: &str| {
                crate::agent::memory::remember::extract_remember_content(msg).is_some()
            })),
        ));

        let complexity_scorer = if let Some(ref r) = routing
            && let Some(weights) = r.chat_weights()
        {
            info!("complexity-aware message routing enabled");
            Some(complexity::ComplexityScorer::new(weights))
        } else {
            None
        };

        Ok(Self {
            inbound_rx,
            bus,
            provider,
            workspace: workspace.clone(),
            model,
            max_iterations,
            context,
            sessions,
            memory,
            tools,
            compactor,
            compaction_config,
            _subagents: Some(subagents),
            session_states: Arc::new(DashMap::new()),
            running: Arc::new(tokio::sync::Mutex::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
            task_tracker: Arc::new(TaskTracker::new()),
            temperature,
            tool_temperature,
            max_tokens,
            typing_tx,
            transcriber,
            event_matcher,
            event_matcher_last_rebuild: Arc::new(std::sync::atomic::AtomicU64::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs()),
            )),
            cron_service,
            compaction_state: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(MAX_COMPACTION_STATE_SESSIONS)
                    .expect("MAX_COMPACTION_STATE_SESSIONS must be > 0"),
            ))),
            cognitive_config,
            exfiltration_guard,
            prompt_guard: if prompt_guard_config.enabled {
                Some(crate::safety::prompt_guard::PromptGuard::new())
            } else {
                None
            },
            prompt_guard_config,
            leak_detector,
            mcp_manager: Arc::new(tokio::sync::Mutex::new(mcp_manager)),
            routing,
            complexity_scorer,
            tool_search_activated,
            pending_buttons,
            router,
            semantic_top_k,
            semantic_prefilter_k,
            semantic_threshold,
            semantic_index_cache: Arc::new(tokio::sync::Mutex::new(None)),
            approval_store: Arc::new(crate::agent::approval::ApprovalStore::new()),
            approval_config,
            reflection_config,
            outbound_tx,
        })
    }

    async fn take_session_checkpoint_handle(
        &self,
        session_key: &str,
    ) -> Option<tokio::task::JoinHandle<()>> {
        self.compaction_state
            .lock()
            .await
            .get_mut(session_key)
            .and_then(|state| state.checkpoint_handle.take())
    }

    async fn session_checkpoint_snapshot(
        &self,
        session_key: &str,
    ) -> (Option<String>, Option<String>) {
        let mut guard = self.compaction_state.lock().await;
        let state = guard.get(session_key);
        (
            state.and_then(|s| s.last_checkpoint.clone()),
            state.and_then(|s| s.cognitive_breadcrumb.clone()),
        )
    }

    async fn set_session_checkpoint(&self, session_key: &str, checkpoint: String) {
        let mut guard = self.compaction_state.lock().await;
        if let Some(state) = guard.get_mut(session_key) {
            state.last_checkpoint = Some(checkpoint);
        } else {
            let state = SessionCompactionState {
                last_checkpoint: Some(checkpoint),
                ..Default::default()
            };
            guard.put(session_key.to_string(), state);
        }
    }

    async fn set_session_cognitive_breadcrumb(&self, session_key: &str, breadcrumb: String) {
        let mut guard = self.compaction_state.lock().await;
        if let Some(state) = guard.get_mut(session_key) {
            state.cognitive_breadcrumb = Some(breadcrumb);
        } else {
            let state = SessionCompactionState {
                cognitive_breadcrumb: Some(breadcrumb),
                ..Default::default()
            };
            guard.put(session_key.to_string(), state);
        }
    }

    /// Run the agent loop, processing inbound messages until the channel closes
    /// or [`stop()`](Self::stop) is called.
    ///
    /// **Shutdown:** Calling `stop()` signals the shutdown notify, which wakes the
    /// blocked `recv()` via `tokio::select!`.
    pub async fn run(&self) -> Result<()> {
        *self.running.lock().await = true;
        info!("agent loop started, waiting for messages");

        loop {
            let running = {
                let guard = self.running.lock().await;
                *guard
            };
            if !running {
                break;
            }

            // Race inbound recv against shutdown signal so stop() wakes the loop.
            let msg_opt = {
                let mut rx = self.inbound_rx.lock().await;
                tokio::select! {
                    msg = rx.recv() => msg,
                    () = self.shutdown_notify.notified() => {
                        info!("agent loop received shutdown signal");
                        break;
                    }
                }
            };

            if let Some(msg) = msg_opt {
                info!(
                    "Agent received inbound message: channel={}, sender_id={}, chat_id={}, content_len={}",
                    msg.channel,
                    msg.sender_id,
                    msg.chat_id,
                    msg.content.len()
                );
                // Capture fields before moving msg into process_message
                let msg_channel = msg.channel.clone();
                let msg_chat_id = msg.chat_id.clone();
                let msg_metadata = msg.metadata.clone();
                match self.process_message(msg).await {
                    Ok(Some(outbound_msg)) => {
                        // Send response back through the bus
                        info!(
                            "Agent generated outbound message: channel={}, chat_id={}, content_len={}",
                            outbound_msg.channel,
                            outbound_msg.chat_id,
                            outbound_msg.content.len()
                        );
                        if let Err(e) = self.bus.publish_outbound(outbound_msg).await {
                            error!("Failed to send outbound message: {}", e);
                        } else {
                            info!("Successfully sent outbound message to bus");
                        }
                    }
                    Ok(None) => {
                        // No response (e.g., empty after delivery tool)
                        debug!(
                            "No outbound message needed (content delivered via tool or suppressed)"
                        );
                    }
                    Err(e) => {
                        error!("Error processing message: {}", e);
                        // Surface actionable errors to the user instead of a generic message
                        let err_str = e.to_string();
                        let user_message = if err_str.contains("credits")
                            || err_str.contains("quota")
                            || err_str.contains("billing")
                        {
                            format!("Provider billing error: {err_str}")
                        } else if err_str.contains("rate limit") {
                            "Rate limited by the LLM provider — please try again in a moment."
                                .to_string()
                        } else if err_str.contains("model") && err_str.contains("not found") {
                            format!("Model configuration error: {err_str}")
                        } else {
                            "Sorry, I encountered an error processing your message.".to_string()
                        };
                        // Send an error outbound so channels can clean up
                        // (e.g. Slack removes the thinking emoji on any outbound)
                        let error_outbound =
                            OutboundMessage::builder(msg_channel, msg_chat_id, &user_message)
                                .metadata(msg_metadata)
                                .build();
                        if let Err(send_err) = self.bus.publish_outbound(error_outbound).await {
                            error!("Failed to send error outbound message: {}", send_err);
                        }
                    }
                }
            } else {
                // Channel closed — all senders dropped
                info!("Inbound channel closed, stopping agent loop");
                break;
            }
        }

        info!("Agent loop stopped");
        Ok(())
    }

    pub fn memory_db(&self) -> Arc<crate::agent::memory::memory_db::MemoryDB> {
        self.memory.db()
    }

    pub fn tool_registry(&self) -> Arc<ToolRegistry> {
        self.tools.clone()
    }

    pub fn approval_store(&self) -> Arc<crate::agent::approval::ApprovalStore> {
        self.approval_store.clone()
    }

    /// Resolve per-task overrides from the model routing configuration.
    /// Returns default overrides when routing is not configured or the task
    /// type has no matching rule.
    pub fn resolve_overrides(&self, task_type: &str) -> AgentRunOverrides {
        if let Some(ref routing) = self.routing {
            let resolved = routing.resolve_overrides(task_type);
            if resolved.provider.is_some() {
                return resolved;
            }
        }
        AgentRunOverrides::default()
    }

    pub async fn stop(&self) {
        {
            let mut guard = self.running.lock().await;
            *guard = false;
        }
        self.shutdown_notify.notify_waiters();
        self.task_tracker.cancel_all().await;

        // Gracefully shut down MCP child processes
        if let Some(manager) = self.mcp_manager.lock().await.take() {
            tokio::spawn(async move {
                manager.shutdown().await;
            });
        }
    }

    /// Get or create per-session state (processing lock + pending queue).
    fn session_state(&self, session_key: &str) -> Arc<SessionState> {
        self.session_states
            .entry(session_key.to_string())
            .or_insert_with(|| {
                Arc::new(SessionState {
                    processing: tokio::sync::Mutex::new(()),
                    pending: std::sync::Mutex::new(Vec::new()),
                })
            })
            .clone()
    }

    /// Remove session states that are only held by the map (strong count == 1).
    /// This prevents the `session_states` map from growing unboundedly.
    ///
    /// Note on `Arc::strong_count`: With `DashMap`, concurrent `session_state()`
    /// calls may briefly race with eviction. The worst case is a stale entry
    /// survives one extra eviction cycle — acceptable for a cleanup heuristic.
    fn evict_stale_session_states(&self) {
        let before = self.session_states.len();
        self.session_states
            .retain(|_, arc| Arc::strong_count(arc) > 1);
        let evicted = before - self.session_states.len();
        if evicted > 0 {
            debug!("evicted {evicted} stale session state(s)");
        }
    }

    /// Coalesce multiple pending messages into a single `InboundMessage`.
    /// Joins content with newlines, merges media, and merges metadata with
    /// explicit policy per key:
    /// * Identity keys (`is_group`, `session_id`): first-message wins. These
    ///   describe the session scope and must not drift.
    /// * `ts` / timestamp: latest wins so threading refs the newest message.
    /// * Other keys: last wins, with a debug log when values conflict so
    ///   drift is visible in traces.
    ///
    /// Preserves the first non-None action dispatch.
    fn coalesce_messages(messages: Vec<InboundMessage>) -> InboundMessage {
        const IDENTITY_KEYS: &[&str] = &[crate::bus::meta::IS_GROUP, crate::bus::meta::SESSION_ID];

        debug_assert!(!messages.is_empty());
        let content = messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let mut merged_media: Vec<String> = Vec::new();
        let mut merged_metadata: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        let mut first_action = None;

        for msg in &messages {
            merged_media.extend(msg.media.iter().cloned());
            for (k, v) in &msg.metadata {
                if IDENTITY_KEYS.contains(&k.as_str()) {
                    // First-message wins: session scope should not drift.
                    merged_metadata
                        .entry(k.clone())
                        .or_insert_with(|| v.clone());
                    continue;
                }
                if let Some(prev) = merged_metadata.get(k)
                    && prev != v
                {
                    debug!(
                        "coalesce: metadata key '{}' changed across queued messages ({:?} -> {:?})",
                        k, prev, v
                    );
                }
                merged_metadata.insert(k.clone(), v.clone());
            }
            match (&first_action, &msg.action) {
                (None, Some(_)) => first_action.clone_from(&msg.action),
                (Some(first), Some(later)) => {
                    // Multiple queued messages carry action dispatches. Only
                    // the first can be honoured (actions are not commutative
                    // — running `shell:rm -rf a` after `shell:cp a b` would
                    // execute two shell invocations for a single coalesced
                    // turn). Warn so operators can see non-first drops.
                    warn!(
                        "coalesce: dropping subsequent action (tool={} source={:?}); \
                         first action (tool={} source={:?}) wins",
                        later.tool, later.source, first.tool, first.source
                    );
                }
                _ => {}
            }
        }

        let mut coalesced = messages.into_iter().last().expect("non-empty vec");
        coalesced.content = content;
        coalesced.media = merged_media;
        coalesced.metadata = merged_metadata;
        if first_action.is_some() {
            coalesced.action = first_action;
        }
        coalesced
    }

    async fn process_message(&self, msg: InboundMessage) -> Result<Option<OutboundMessage>> {
        // Periodically evict stale session states to prevent unbounded growth.
        // Only run every 100 messages to avoid the overhead on every call.
        static EVICT_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        if EVICT_COUNTER
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .is_multiple_of(100)
        {
            self.evict_stale_session_states();
        }

        // Approval callbacks bypass the session lock to prevent deadlock
        // in self-approval mode (same channel as user). The resolve_approval
        // method only touches the ApprovalStore (its own mutex) — it doesn't
        // need session state.
        if let Some(ref action) = msg.action
            && action.tool == "__approval"
            && matches!(action.source, crate::dispatch::ActionSource::Button { .. })
        {
            return Ok(Some(self.resolve_approval(&msg, action)));
        }

        let session_key = msg.session_key();
        let state = self.session_state(&session_key);

        // Try to acquire the processing lock without blocking. If the
        // session is already being processed, queue this message for
        // coalesced processing after the current run completes.
        let Ok(guard) = state.processing.try_lock() else {
            // Session is busy — queue the message
            let queued = {
                let mut pending = state
                    .pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if pending.len() >= MAX_PENDING_MESSAGES_PER_SESSION {
                    warn!(
                        "pending queue full for session {} (max={}), dropping message \
                         from {}:{} ({} bytes); user will see no response for this message",
                        session_key,
                        MAX_PENDING_MESSAGES_PER_SESSION,
                        msg.channel,
                        msg.sender_id,
                        msg.content.len()
                    );
                    metrics::counter!("oxicrab_agent_pending_queue_dropped_total",
                        "channel" => msg.channel.clone(),
                    )
                    .increment(1);
                    return Ok(None);
                }
                pending.push(msg);
                pending.len()
            };
            info!(
                "queued message for busy session {} ({} pending)",
                session_key, queued
            );
            return Ok(None);
        };

        // Process the initial message
        let result = self.process_message_unlocked(msg).await;

        // Drain and process any messages that queued while we held the lock.
        // Stay inside the processing guard so new arrivals continue to queue.
        loop {
            let pending = {
                let mut queue = state
                    .pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if queue.is_empty() {
                    break;
                }
                std::mem::take(&mut *queue)
            };

            let count = pending.len();
            info!(
                "processing {} coalesced pending message(s) for session {}",
                count, session_key
            );
            let coalesced = Self::coalesce_messages(pending);

            // Process the coalesced turn. Errors are logged but do not
            // prevent draining the remaining queue — we still hold the
            // guard and want to eventually release it.
            match self.process_message_unlocked(coalesced).await {
                Ok(Some(outbound)) => {
                    info!(
                        "coalesced turn produced response for session {}",
                        session_key
                    );
                    if let Err(e) = self.bus.publish_outbound(outbound).await {
                        error!(
                            "failed to send coalesced outbound for session {}: {}",
                            session_key, e
                        );
                    }
                }
                Ok(None) => {
                    debug!(
                        "coalesced turn produced no response for session {}",
                        session_key
                    );
                }
                Err(e) => {
                    error!(
                        "error processing coalesced turn for session {}: {}",
                        session_key, e
                    );
                }
            }
        }

        drop(guard);
        result
    }

    /// Resolve an operator approval callback without acquiring the session lock.
    fn resolve_approval(
        &self,
        msg: &InboundMessage,
        action: &crate::dispatch::ActionDispatch,
    ) -> OutboundMessage {
        use crate::agent::approval::ApprovalDecision;

        let approval_id = action
            .params
            .get("approval_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let decision_str = action
            .params
            .get("decision")
            .and_then(|v| v.as_str())
            .unwrap_or("denied");

        let decision = if decision_str == "approved" {
            ApprovalDecision::Approved
        } else {
            ApprovalDecision::Denied { reason: None }
        };

        let source_channel = format!("{}:{}", msg.channel, msg.chat_id);

        match self
            .approval_store
            .resolve(approval_id, &source_channel, decision)
        {
            Ok((tool_name, action_name, requested_by)) => {
                let status = if decision_str == "approved" {
                    "Approved"
                } else {
                    "Denied"
                };
                let response = format!(
                    "{status} {tool_name}.{action_name} for {requested_by} (by {})",
                    msg.sender_id
                );
                OutboundMessage::from_inbound(msg.clone(), response).build()
            }
            Err(err_msg) => OutboundMessage::from_inbound(msg.clone(), err_msg).build(),
        }
    }
}

#[cfg(test)]
mod tests;
