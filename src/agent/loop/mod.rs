mod compaction_history;
mod complexity;
pub mod config;
mod hallucination;
mod helpers;
mod intent;
mod iteration;
mod processing;
mod tool_filter;

#[cfg(test)]
use crate::agent::tools::base::ExecutionContext;
#[cfg(test)]
use helpers::ACTION_CLAIM_PATTERNS;
#[cfg(test)]
use helpers::MAX_IMAGES;
use helpers::cleanup_old_media;
pub(crate) use helpers::validate_tool_params;
pub use helpers::{contains_action_claims, is_false_no_tools_claim, mentions_multiple_tools};
#[cfg(test)]
use helpers::{
    execute_tool_call, extract_media_paths, load_and_encode_images, strip_document_tags,
    strip_think_tags,
};

pub use config::{
    AgentLoopConfig, AgentLoopResult, AgentLoopRuntimeParams, AgentRunOverrides, DirectResult,
    LifecycleConfig, SafetyConfig, ToolConfigs,
};

#[cfg(test)]
use tool_filter::infer_tool_categories;

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
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

const EMPTY_RESPONSE_RETRIES: usize = 2;
const WRAPUP_THRESHOLD_RATIO: f64 = 0.7;
const MIN_WRAPUP_ITERATION: usize = 2;
const RETRY_BACKOFF_BASE: u64 = 2;
const MAX_RETRY_DELAY_SECS: f64 = 10.0;
const DEFAULT_HISTORY_SIZE: usize = 50;
const RECOVERY_CONTEXT_MAX_CHARS: usize = 200;

#[cfg(test)]
use hallucination::MAX_LAYER0_CORRECTIONS;
#[cfg(test)]
use hallucination::{CorrectionState, TextAction};

pub struct AgentLoop {
    inbound_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<InboundMessage>>>,
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
    /// Per-session processing locks. Each session key maps to a Mutex that
    /// serializes message processing for that session while allowing independent
    /// sessions to be processed concurrently.
    session_locks: Arc<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    running: Arc<tokio::sync::Mutex<bool>>,
    outbound_tx: Arc<tokio::sync::mpsc::Sender<OutboundMessage>>,
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
    /// Most recent checkpoint summary (updated periodically during long loops)
    last_checkpoint: Arc<Mutex<Option<String>>>,
    /// Handle for the most recent background checkpoint task
    checkpoint_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    cognitive_config: crate::config::CognitiveConfig,
    /// Cognitive breadcrumb for compaction recovery (updated during long loops)
    cognitive_breadcrumb: Arc<Mutex<Option<String>>>,
    /// Exfiltration guard: hides outbound tools from the LLM
    exfiltration_guard: crate::config::ExfiltrationGuardConfig,
    /// Prompt injection detection guard
    prompt_guard: Option<crate::safety::prompt_guard::PromptGuard>,
    prompt_guard_config: crate::config::PromptGuardConfig,
    /// Inbound secret leak detector — scans user messages before they reach the LLM
    leak_detector: LeakDetector,
    /// MCP manager kept alive for graceful child process shutdown
    _mcp_manager: Option<crate::agent::tools::mcp::McpManager>,
    /// Pre-resolved model routing for task-specific provider selection
    routing: Option<Arc<crate::config::routing::ResolvedRouting>>,
    /// Complexity scorer for per-message model routing (None when disabled)
    complexity_scorer: Option<complexity::ComplexityScorer>,
    /// Shared activation set for deferred tools discovered via `tool_search`
    tool_search_activated: Arc<tokio::sync::Mutex<HashSet<String>>>,
    /// Shared state for interactive buttons (written by `add_buttons` tool, read after loop)
    pending_buttons: crate::agent::tools::interactive::PendingButtons,
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
        } = config;

        // Extract receiver from the bus (called once at startup).
        // Receivers are !Sync, so we wrap in Arc<Mutex> for sharing.
        let inbound_rx = Arc::new(tokio::sync::Mutex::new(
            bus.take_inbound_rx()
                .ok_or_else(|| anyhow::anyhow!("Inbound receiver already taken"))?,
        ));
        let model = model.unwrap_or_else(|| provider.default_model().to_string());

        let sessions: Arc<dyn SessionStore> = Arc::new(SessionManager::new(&workspace)?);
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

        // Share the (embedding-configured) memory store with context builder
        let mut context_builder = ContextBuilder::with_memory(&workspace, memory.clone())?;
        if !context_providers.is_empty() {
            use crate::agent::context::providers::ContextProviderRunner;
            let runner = Arc::new(ContextProviderRunner::new(context_providers));
            context_builder.set_providers(runner);
        }
        let context = Arc::new(Mutex::new(context_builder));

        // Clean up expired sessions in background
        if session_ttl_days > 0 {
            let ttl = session_ttl_days;
            let mgr_for_cleanup = SessionManager::new(&workspace)?;
            tokio::spawn(async move {
                if let Err(e) = mgr_for_cleanup.cleanup_old_sessions(ttl) {
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
            tokio::task::spawn_blocking(move || {
                crate::agent::memory::hygiene::run_hygiene(&db, 90);
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
                }
            },
            allowed_commands: tool_configs.allowed_commands,
            mcp_config: tool_configs.mcp_config,
            sandbox_config: tool_configs.sandbox_config,
            memory_db: Some(memory.db()),
            workspace_manager,
            workspace_ttl: tool_configs.workspace_ttl,
            pending_buttons: pending_buttons.clone(),
        };

        let (tools, subagents, mcp_manager, tool_search_activated) =
            crate::agent::tools::setup::register_all_tools(&tool_ctx).await?;
        let tools = Arc::new(tools);
        subagents.set_main_tools(tools.clone());

        let transcriber = voice_config
            .as_ref()
            .filter(|vc| vc.transcription.enabled)
            .map(|vc| {
                Arc::new(crate::utils::transcription::LazyTranscriptionService::new(
                    vc.transcription.clone(),
                ))
            });

        let compactor = if compaction_config.enabled {
            let (comp_provider, comp_model) = if let Some(ref r) = routing {
                let o = r.resolve_overrides("compaction");
                if let Some(p) = o.provider {
                    (p, o.model)
                } else {
                    (
                        provider.clone() as Arc<dyn LLMProvider>,
                        compaction_config.model.clone(),
                    )
                }
            } else {
                (
                    provider.clone() as Arc<dyn LLMProvider>,
                    compaction_config.model.clone(),
                )
            };
            Some(Arc::new(MessageCompactor::new(comp_provider, comp_model)))
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
            session_locks: Arc::new(std::sync::Mutex::new(HashMap::new())),
            running: Arc::new(tokio::sync::Mutex::new(false)),
            outbound_tx,
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
            last_checkpoint: Arc::new(Mutex::new(None)),
            checkpoint_handle: Arc::new(Mutex::new(None)),
            cognitive_config,
            cognitive_breadcrumb: Arc::new(Mutex::new(None)),
            exfiltration_guard,
            prompt_guard: if prompt_guard_config.enabled {
                Some(crate::safety::prompt_guard::PromptGuard::new())
            } else {
                None
            },
            prompt_guard_config,
            leak_detector: LeakDetector::new(),
            _mcp_manager: mcp_manager,
            routing,
            complexity_scorer,
            tool_search_activated,
            pending_buttons,
        })
    }

    /// Run the agent loop, processing inbound messages until the channel closes.
    ///
    /// **Shutdown:** The caller must cancel the spawned task (e.g. via `tokio::select!`)
    /// to stop the loop. The `stop()` method sets an advisory flag but does not
    /// wake the blocked `recv()` call.
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

            // Check for messages - lock receiver only for recv()
            // Note: This is necessary because receivers are !Sync
            let msg_opt = {
                let mut rx = self.inbound_rx.lock().await;
                rx.recv().await
            };

            if let Some(msg) = msg_opt {
                info!(
                    "Agent received inbound message: channel={}, sender_id={}, chat_id={}, content_len={}",
                    msg.channel,
                    msg.sender_id,
                    msg.chat_id,
                    msg.content.len()
                );
                match self.process_message(msg).await {
                    Ok(Some(outbound_msg)) => {
                        // Send response back through the bus
                        info!(
                            "Agent generated outbound message: channel={}, chat_id={}, content_len={}",
                            outbound_msg.channel,
                            outbound_msg.chat_id,
                            outbound_msg.content.len()
                        );
                        if let Err(e) = self.outbound_tx.send(outbound_msg).await {
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
        self.task_tracker.cancel_all().await;
    }

    /// Get or create a per-session lock, enabling concurrent processing of
    /// independent sessions while serializing within each session.
    fn session_lock(&self, session_key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .session_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        locks
            .entry(session_key.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// Remove session locks that are only held by the map (strong count == 1).
    /// This prevents the `session_locks` `HashMap` from growing unboundedly.
    ///
    /// Safety of `Arc::strong_count`: This is called under the outer
    /// `std::sync::Mutex` lock on `session_locks`, which serializes all
    /// calls to `session_lock()` and `evict_stale_session_locks()`. No
    /// other code clones the `Arc` without holding that mutex, so the
    /// strong count cannot change between the check and the retain
    /// decision — no TOCTOU race is possible.
    fn evict_stale_session_locks(&self) {
        let mut locks = self
            .session_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = locks.len();
        locks.retain(|_, arc| Arc::strong_count(arc) > 1);
        let evicted = before - locks.len();
        if evicted > 0 {
            debug!("evicted {evicted} stale session lock(s)");
        }
    }

    async fn process_message(&self, msg: InboundMessage) -> Result<Option<OutboundMessage>> {
        // Periodically evict stale session locks to prevent unbounded growth.
        // Strong count == 1 means only the map holds a reference (no active processing).
        self.evict_stale_session_locks();

        let session_key = msg.session_key();
        let lock = self.session_lock(&session_key);
        let _guard = lock.lock().await;
        self.process_message_unlocked(msg).await
    }
}

#[cfg(test)]
mod tests;
