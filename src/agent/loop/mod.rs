mod helpers;
mod intent;

#[cfg(test)]
use helpers::ACTION_CLAIM_PATTERNS;
#[cfg(test)]
use helpers::MAX_IMAGES;
pub(crate) use helpers::validate_tool_params;
use helpers::{
    cleanup_old_media, execute_tool_call, extract_media_paths, load_and_encode_images,
    start_typing, strip_audio_tags, strip_document_tags, strip_image_tags, transcribe_audio_tags,
};
pub use helpers::{contains_action_claims, is_false_no_tools_claim, mentions_multiple_tools};

use crate::agent::cognitive::CheckpointTracker;
use crate::agent::compaction::{
    MessageCompactor, estimate_messages_tokens, strip_orphaned_tool_messages,
};
use crate::agent::context::ContextBuilder;
use crate::agent::cost_guard::CostGuard;
use crate::agent::memory::MemoryStore;
use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::subagent::{SubagentConfig, SubagentManager};
use crate::agent::tools::ToolRegistry;
use crate::agent::tools::base::ExecutionContext;
use crate::agent::tools::setup::ToolBuildContext;
use crate::bus::{InboundMessage, MessageBus, OutboundMessage};
use crate::cron::event_matcher::EventMatcher;
use crate::cron::service::CronService;
use crate::providers::base::{LLMProvider, Message, ToolCallRequest};
use crate::session::{Session, SessionManager, SessionStore};
use crate::utils::task_tracker::TaskTracker;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write as _;
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

/// Per-invocation overrides for the agent loop. Allows callers (e.g. the daemon
/// heartbeat) to use a different model or iteration cap without constructing a
/// separate `AgentLoop`.
#[derive(Default)]
pub struct AgentRunOverrides {
    /// Override the model used for LLM calls.
    pub model: Option<String>,
    /// Override the maximum number of iterations.
    pub max_iterations: Option<usize>,
}

/// Tool-specific configurations bundled together. These fields are only used
/// to construct [`ToolBuildContext`] during [`AgentLoop::new`] — grouping them
/// reduces `AgentLoopConfig` field count and makes adding new tools cheaper
/// (only touch this struct + `from_config` + `ToolBuildContext`).
pub struct ToolConfigs {
    pub brave_api_key: Option<String>,
    pub web_search_config: Option<crate::config::WebSearchConfig>,
    pub exec_timeout: u64,
    pub restrict_to_workspace: bool,
    pub allowed_commands: Vec<String>,
    pub sandbox_config: crate::config::SandboxConfig,
    pub channels_config: Option<crate::config::ChannelsConfig>,
    pub google_config: Option<crate::config::GoogleConfig>,
    pub github_config: Option<crate::config::GitHubConfig>,
    pub weather_config: Option<crate::config::WeatherConfig>,
    pub todoist_config: Option<crate::config::TodoistConfig>,
    pub media_config: Option<crate::config::MediaConfig>,
    pub obsidian_config: Option<crate::config::ObsidianConfig>,
    pub browser_config: Option<crate::config::BrowserConfig>,
    pub image_gen_config: Option<crate::config::ImageGenConfig>,
    pub mcp_config: Option<crate::config::McpConfig>,
}

/// Configuration for creating an [`AgentLoop`] instance.
pub struct AgentLoopConfig {
    pub bus: Arc<Mutex<MessageBus>>,
    pub provider: Arc<dyn LLMProvider>,
    pub workspace: PathBuf,
    pub model: Option<String>,
    pub max_iterations: usize,
    pub compaction_config: crate::config::CompactionConfig,
    pub outbound_tx: Arc<tokio::sync::mpsc::Sender<OutboundMessage>>,
    pub cron_service: Option<Arc<CronService>>,
    /// Temperature for response generation (default 0.7)
    pub temperature: f32,
    /// Temperature for tool-calling iterations (default 0.0 for determinism)
    pub tool_temperature: f32,
    /// Session TTL in days for cleanup (default 30)
    pub session_ttl_days: u32,
    /// Max tokens for LLM responses (default 8192)
    pub max_tokens: u32,
    /// Sender for typing indicator events (channel, `chat_id`)
    pub typing_tx: Option<Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
    /// Memory indexer interval in seconds (default 300)
    pub memory_indexer_interval: u64,
    /// Media file TTL in days for cleanup (default 7)
    pub media_ttl_days: u32,
    /// Maximum concurrent subagents (default 5)
    pub max_concurrent_subagents: usize,
    /// Voice transcription configuration
    pub voice_config: Option<crate::config::VoiceConfig>,
    /// Memory configuration (archive/purge days)
    pub memory_config: Option<crate::config::MemoryConfig>,
    /// Cost guard configuration for budget and rate limiting
    pub cost_guard_config: crate::config::CostGuardConfig,
    /// Cognitive routines configuration for checkpoint pressure signals
    pub cognitive_config: crate::config::CognitiveConfig,
    /// Exfiltration guard configuration for hiding outbound tools from LLM
    pub exfiltration_guard: crate::config::ExfiltrationGuardConfig,
    /// Prompt injection detection configuration
    pub prompt_guard_config: crate::config::PromptGuardConfig,
    /// External context providers that inject dynamic content into the system prompt
    pub context_providers: Vec<crate::config::ContextProviderConfig>,
    /// Tool-specific configurations (forwarded to [`ToolBuildContext`])
    pub tool_configs: ToolConfigs,
}

/// Temperature used for tool-calling iterations (low for determinism)
const TOOL_TEMPERATURE: f32 = 0.0;

/// Runtime parameters for [`AgentLoopConfig::from_config`] that vary per
/// invocation (as opposed to values read from the config file).
pub struct AgentLoopRuntimeParams {
    pub bus: Arc<Mutex<MessageBus>>,
    pub provider: Arc<dyn LLMProvider>,
    pub model: Option<String>,
    pub outbound_tx: Arc<tokio::sync::mpsc::Sender<OutboundMessage>>,
    pub cron_service: Option<Arc<CronService>>,
    pub typing_tx: Option<Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
    pub channels_config: Option<crate::config::ChannelsConfig>,
}

impl AgentLoopConfig {
    /// Build an `AgentLoopConfig` from the application [`Config`](crate::config::Config)
    /// and runtime parameters that vary per invocation.
    pub fn from_config(config: &crate::config::Config, params: AgentLoopRuntimeParams) -> Self {
        let mut image_gen = config.tools.image_gen.clone();
        if image_gen.enabled {
            if !config.providers.openai.api_key.is_empty() {
                image_gen.openai_api_key = Some(config.providers.openai.api_key.clone());
            }
            if !config.providers.gemini.api_key.is_empty() {
                image_gen.google_api_key = Some(config.providers.gemini.api_key.clone());
            }
        }

        Self {
            bus: params.bus,
            provider: params.provider,
            workspace: config.workspace_path(),
            model: params.model,
            max_iterations: config.agents.defaults.max_tool_iterations,
            compaction_config: config.agents.defaults.compaction.clone(),
            outbound_tx: params.outbound_tx,
            cron_service: params.cron_service,
            temperature: config.agents.defaults.temperature,
            tool_temperature: TOOL_TEMPERATURE,
            session_ttl_days: config.agents.defaults.session_ttl_days,
            max_tokens: config.agents.defaults.max_tokens,
            typing_tx: params.typing_tx,
            memory_indexer_interval: config.agents.defaults.memory_indexer_interval,
            media_ttl_days: config.agents.defaults.media_ttl_days,
            max_concurrent_subagents: config.agents.defaults.max_concurrent_subagents,
            voice_config: Some(config.voice.clone()),
            memory_config: Some(config.agents.defaults.memory.clone()),
            cost_guard_config: config.agents.defaults.cost_guard.clone(),
            cognitive_config: config.agents.defaults.cognitive.clone(),
            exfiltration_guard: config.tools.exfiltration_guard.clone(),
            prompt_guard_config: config.agents.defaults.prompt_guard.clone(),
            context_providers: config.agents.defaults.context_providers.clone(),
            tool_configs: ToolConfigs {
                brave_api_key: Some(config.tools.web.search.api_key.clone()),
                web_search_config: Some(config.tools.web.search.clone()),
                exec_timeout: config.tools.exec.timeout,
                restrict_to_workspace: config.tools.restrict_to_workspace,
                allowed_commands: config.tools.exec.allowed_commands.clone(),
                sandbox_config: config.tools.exec.sandbox.clone(),
                channels_config: params.channels_config,
                google_config: Some(config.tools.google.clone()),
                github_config: Some(config.tools.github.clone()),
                weather_config: Some(config.tools.weather.clone()),
                todoist_config: Some(config.tools.todoist.clone()),
                media_config: Some(config.tools.media.clone()),
                obsidian_config: Some(config.tools.obsidian.clone()),
                browser_config: Some(config.tools.browser.clone()),
                image_gen_config: Some(image_gen),
                mcp_config: Some(config.tools.mcp.clone()),
            },
        }
    }

    /// Create a config with sensible test defaults. Only `bus`, `provider`,
    /// `workspace`, and `outbound_tx` are required; everything else gets
    /// minimal/disabled defaults.
    #[doc(hidden)]
    pub fn test_defaults(
        bus: Arc<Mutex<MessageBus>>,
        provider: Arc<dyn LLMProvider>,
        workspace: PathBuf,
        outbound_tx: Arc<tokio::sync::mpsc::Sender<OutboundMessage>>,
    ) -> Self {
        Self {
            bus,
            provider,
            workspace,
            model: Some("mock-model".to_string()),
            max_iterations: 10,
            compaction_config: crate::config::CompactionConfig {
                enabled: false,
                threshold_tokens: 40000,
                keep_recent: 10,
                extraction_enabled: false,
                model: None,
                checkpoint: crate::config::CheckpointConfig::default(),
                pre_flush_enabled: false,
            },
            outbound_tx,
            cron_service: None,
            temperature: 0.7,
            tool_temperature: 0.0,
            session_ttl_days: 0,
            max_tokens: 8192,
            typing_tx: None,
            memory_indexer_interval: 300,
            media_ttl_days: 0,
            max_concurrent_subagents: 5,
            voice_config: None,
            memory_config: None,
            cost_guard_config: crate::config::CostGuardConfig::default(),
            cognitive_config: crate::config::CognitiveConfig::default(),
            exfiltration_guard: crate::config::ExfiltrationGuardConfig::default(),
            prompt_guard_config: crate::config::PromptGuardConfig::default(),
            context_providers: vec![],
            tool_configs: ToolConfigs {
                brave_api_key: None,
                web_search_config: None,
                exec_timeout: 30,
                restrict_to_workspace: true,
                allowed_commands: vec![],
                sandbox_config: crate::config::SandboxConfig {
                    enabled: false,
                    ..crate::config::SandboxConfig::default()
                },
                channels_config: None,
                google_config: None,
                github_config: None,
                weather_config: None,
                todoist_config: None,
                media_config: None,
                obsidian_config: None,
                browser_config: None,
                image_gen_config: None,
                mcp_config: None,
            },
        }
    }
}

/// Result of [`AgentLoop::handle_text_response`] — either continue the loop
/// (a nudge/correction was injected) or return the final text to the caller.
enum TextAction {
    /// A nudge or correction was injected; the loop should `continue`.
    Continue,
    /// The response is final; the caller should return it.
    Return,
}

/// Tracks how many corrections each hallucination detection layer has sent,
/// preventing infinite correction loops while allowing each layer its own budget.
struct CorrectionState {
    /// Layer 0 (false no-tools claim) correction count. Capped at
    /// `MAX_LAYER0_CORRECTIONS` — if the LLM insists it has no tools after
    /// that many corrections, give up.
    layer0_count: u8,
    /// Whether Layer 1 (regex action claims) has fired. Fires once — a second
    /// hallucination after correction is accepted as the LLM's final answer.
    layer1_fired: bool,
    /// Whether Layer 2 (intent mismatch) has fired. Independent of Layer 1,
    /// so if L1 corrects first and fails, L2 still gets its own attempt.
    layer2_fired: bool,
}

impl CorrectionState {
    fn new() -> Self {
        Self {
            layer0_count: 0,
            layer1_fired: false,
            layer2_fired: false,
        }
    }
}

/// Maximum corrections for Layer 0 (false no-tools claims).
const MAX_LAYER0_CORRECTIONS: u8 = 2;

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
    processing_lock: Arc<tokio::sync::Mutex<()>>,
    running: Arc<tokio::sync::Mutex<bool>>,
    outbound_tx: Arc<tokio::sync::mpsc::Sender<OutboundMessage>>,
    task_tracker: Arc<TaskTracker>,
    temperature: f32,
    tool_temperature: f32,
    max_tokens: u32,
    typing_tx: Option<Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
    transcriber: Option<Arc<crate::utils::transcription::LazyTranscriptionService>>,
    event_matcher: Option<std::sync::Mutex<EventMatcher>>,
    /// Epoch-seconds timestamp of last event matcher rebuild (atomic to avoid
    /// blocking the async runtime with a `std::sync::Mutex`)
    event_matcher_last_rebuild: Arc<std::sync::atomic::AtomicU64>,
    cron_service: Option<Arc<CronService>>,
    cost_guard: Option<Arc<CostGuard>>,
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
    /// MCP manager kept alive for graceful child process shutdown
    _mcp_manager: Option<crate::agent::tools::mcp::McpManager>,
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
            session_ttl_days,
            max_tokens,
            typing_tx,
            memory_indexer_interval,
            media_ttl_days,
            max_concurrent_subagents,
            voice_config,
            memory_config,
            cost_guard_config,
            cognitive_config,
            exfiltration_guard,
            prompt_guard_config,
            context_providers,
            tool_configs,
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
        let mut context_builder = ContextBuilder::new(&workspace)?;
        if !context_providers.is_empty() {
            use crate::agent::context::providers::ContextProviderRunner;
            let runner = Arc::new(ContextProviderRunner::new(context_providers));
            context_builder.set_providers(runner);
        }
        let context = Arc::new(Mutex::new(context_builder));
        let session_mgr = SessionManager::new(&workspace)?;

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

        let sessions: Arc<dyn SessionStore> = Arc::new(session_mgr);
        let memory = Arc::new(if let Some(ref mem_cfg) = memory_config {
            MemoryStore::with_config(&workspace, memory_indexer_interval, mem_cfg)?
        } else {
            MemoryStore::with_indexer_interval(&workspace, memory_indexer_interval)?
        });
        // Start background memory indexer
        memory.start_indexer().await?;

        // Create cost guard — always enabled for cost logging, optionally enforces limits
        info!(
            "cost guard active (daily_budget={:?} cents, max_actions_per_hour={:?})",
            cost_guard_config.daily_budget_cents, cost_guard_config.max_actions_per_hour
        );
        let cost_guard = Some(Arc::new(CostGuard::with_db(cost_guard_config, memory.db())));

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
            subagent_config: SubagentConfig {
                provider: provider.clone(),
                workspace: workspace.clone(),
                model: Some(model.clone()),
                max_tokens,
                tool_temperature,
                max_concurrent: max_concurrent_subagents,
                cost_guard: cost_guard.clone(),
                prompt_guard_config: prompt_guard_config.clone(),
                exfil_guard: exfiltration_guard.clone(),
                main_tools: None, // set after register_all_tools()
            },
            brave_api_key: tool_configs.brave_api_key,
            allowed_commands: tool_configs.allowed_commands,
            mcp_config: tool_configs.mcp_config,
            sandbox_config: tool_configs.sandbox_config,
            memory_db: Some(memory.db()),
        };

        let (tools, subagents, mcp_manager) =
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
            Some(Arc::new(MessageCompactor::new(
                provider.clone() as Arc<dyn LLMProvider>,
                compaction_config.model.clone(),
            )))
        } else {
            None
        };

        // Build event matcher from cron jobs (if any event-triggered jobs exist)
        let event_matcher = if let Some(ref cron_svc) = cron_service {
            match cron_svc.load_store(false).await {
                Ok(store) => {
                    let matcher = EventMatcher::from_jobs(&store.jobs);
                    if matcher.is_empty() {
                        None
                    } else {
                        info!(
                            "Event matcher initialized with {} event-triggered job(s)",
                            store
                                .jobs
                                .iter()
                                .filter(|j| matches!(
                                    j.schedule,
                                    crate::cron::types::CronSchedule::Event { .. }
                                ))
                                .count()
                        );
                        Some(std::sync::Mutex::new(matcher))
                    }
                }
                Err(e) => {
                    warn!("Failed to load cron store for event matcher: {}", e);
                    None
                }
            }
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
            processing_lock: Arc::new(tokio::sync::Mutex::new(())),
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
            cost_guard,
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
            _mcp_manager: mcp_manager,
        })
    }

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

    pub async fn stop(&self) {
        {
            let mut guard = self.running.lock().await;
            *guard = false;
        }
        self.task_tracker.cancel_all().await;
        self.memory.stop_indexer().await;
    }

    async fn process_message(&self, msg: InboundMessage) -> Result<Option<OutboundMessage>> {
        let _lock = self.processing_lock.lock().await;
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

        // Check for event-triggered cron jobs in the background.
        // Periodically rebuild the matcher from the cron store (every 60s)
        // so new/modified event jobs are picked up at runtime.
        if let Some(cron_svc) = &self.cron_service {
            // Check-and-claim: CAS on epoch-seconds timestamp to prevent
            // concurrent messages from triggering duplicate rebuilds.
            let now_epoch = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let last = self
                .event_matcher_last_rebuild
                .load(std::sync::atomic::Ordering::Relaxed);
            let needs_rebuild = now_epoch.saturating_sub(last) >= 60
                && self
                    .event_matcher_last_rebuild
                    .compare_exchange(
                        last,
                        now_epoch,
                        std::sync::atomic::Ordering::AcqRel,
                        std::sync::atomic::Ordering::Relaxed,
                    )
                    .is_ok();
            if needs_rebuild && let Ok(store) = cron_svc.load_store(true).await {
                let new_matcher = EventMatcher::from_jobs(&store.jobs);
                if let Some(ref matcher_mutex) = self.event_matcher
                    && let Ok(mut guard) = matcher_mutex.lock()
                {
                    *guard = new_matcher;
                }
            }

            if let Some(matcher_mutex) = &self.event_matcher {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64);
                let triggered = matcher_mutex
                    .lock()
                    .map(|mut matcher| matcher.check_message(&msg.content, &msg.channel, now_ms))
                    .unwrap_or_default();
                for job in triggered {
                    let cron_svc = cron_svc.clone();
                    let job_id = job.id.clone();
                    info!("Event-triggered cron job '{}' ({})", job.name, job.id);
                    tokio::spawn(async move {
                        if let Err(e) = cron_svc.run_job(&job_id, true).await {
                            warn!("Event-triggered job '{}' failed: {}", job_id, e);
                        }
                    });
                }
            }
        }

        let session_key = msg.session_key();
        // Reuse session to avoid repeated lookups
        debug!("Loading session: {}", session_key);
        let session = self.sessions.get_or_create(&session_key).await?;

        // Build execution context for tool calls
        let context_summary = session
            .metadata
            .get("compaction_summary")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        let exec_ctx = Self::build_execution_context_with_metadata(
            &msg.channel,
            &msg.chat_id,
            context_summary,
            msg.metadata.clone(),
        );

        debug!("Getting compacted history");
        let history = self.get_compacted_history(&session).await?;
        debug!("Got {} history messages", history.len());

        // Transcribe any audio files before other processing
        let msg_content = if let Some(ref lazy) = self.transcriber
            && let Some(svc) = lazy.get()
        {
            transcribe_audio_tags(&msg.content, svc).await
        } else {
            strip_audio_tags(&msg.content)
        };

        // Prompt injection preflight check
        if let Some(ref guard) = self.prompt_guard {
            let matches = guard.scan(&msg_content);
            if !matches.is_empty() {
                for m in &matches {
                    warn!(
                        "prompt injection detected ({:?}): {}",
                        m.category, m.pattern_name
                    );
                }
                if self.prompt_guard_config.should_block() {
                    return Ok(Some(OutboundMessage {
                        channel: msg.channel,
                        chat_id: msg.chat_id,
                        content: "I can't process this message as it appears to contain prompt injection patterns.".to_string(),
                        reply_to: None,
                        media: vec![],
                        metadata: msg.metadata,
                    }));
                }
            }
        }

        // Remember fast path: bypass LLM for explicit "remember that..." messages
        if let Some(content) =
            crate::agent::memory::remember::extract_remember_content(&msg_content)
        {
            let response = match self.try_remember_fast_path(&content, &session_key).await {
                Ok(resp) => resp,
                Err(e) => {
                    warn!("remember fast path failed, falling through to LLM: {}", e);
                    None
                }
            };
            if let Some(response_text) = response {
                return Ok(Some(OutboundMessage {
                    channel: msg.channel,
                    chat_id: msg.chat_id,
                    content: response_text,
                    reply_to: None,
                    media: vec![],
                    metadata: msg.metadata,
                }));
            }
        }

        // Load and encode any attached images (skip audio files)
        let audio_extensions = ["ogg", "mp3", "mp4", "m4a", "wav", "webm", "flac", "oga"];
        let image_media: Vec<String> = msg
            .media
            .iter()
            .filter(|p| {
                let ext = std::path::Path::new(p)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                !audio_extensions.contains(&ext)
            })
            .cloned()
            .collect();

        let images = if image_media.is_empty() {
            vec![]
        } else {
            info!(
                "Loading {} media files for LLM: {:?}",
                image_media.len(),
                image_media
            );
            let imgs = load_and_encode_images(&image_media);
            info!("Encoded {} images for LLM", imgs.len());
            imgs
        };

        // Strip [image: ...] and [document: ...] tags from content when media was
        // successfully encoded, since the LLM receives them as content blocks and
        // doesn't need the file paths (which can cause it to try read_file on binary data).
        let content = if images.is_empty() {
            msg_content
        } else {
            strip_document_tags(&strip_image_tags(&msg_content))
        };

        debug!("Acquiring context lock");
        let is_group = msg
            .metadata
            .get("is_group")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        // Load discourse entity register from session for reference resolution
        let mut discourse_register =
            crate::agent::discourse::DiscourseRegister::from_session_metadata(&session.metadata);
        let entity_context = discourse_register.to_context_string();

        let messages = {
            let mut ctx = self.context.lock().await;
            ctx.refresh_provider_context().await;
            ctx.build_messages(
                &history,
                &content,
                Some(&msg.channel),
                Some(&msg.chat_id),
                Some(&msg.sender_id),
                images,
                is_group,
                entity_context.as_deref(),
            )?
        };
        debug!("Built {} messages, starting agent loop", messages.len());

        let user_action_intent = self.classify_and_record_intent(&content);

        let typing_ctx = Some((msg.channel.clone(), msg.chat_id.clone()));
        let (final_content, input_tokens, tools_used, collected_media, loop_discourse) = self
            .run_agent_loop(messages, typing_ctx, &exec_ctx, user_action_intent)
            .await?;

        // Merge loop-extracted entities into the session's discourse register
        discourse_register.turn = loop_discourse.turn;
        discourse_register.register(loop_discourse.entities);

        // Reload session in case compaction updated it during the agent loop
        // (compaction saves a compaction_summary to session metadata)
        let mut session = self.sessions.get_or_create(&session_key).await?;
        let extra = HashMap::new();
        session.add_message("user".to_string(), msg.content.clone(), extra.clone());
        // Always save an assistant message to maintain user/assistant alternation.
        // Broken alternation causes the Anthropic provider to merge consecutive user
        // messages, which garbles conversation context for future turns.
        let response_text = final_content
            .as_deref()
            .unwrap_or("I wasn't able to generate a response.");
        let mut assistant_extra = HashMap::new();
        if !tools_used.is_empty() {
            assistant_extra.insert(
                "tools_used".to_string(),
                Value::Array(tools_used.into_iter().map(Value::String).collect()),
            );
        }
        session.add_message(
            "assistant".to_string(),
            response_text.to_string(),
            assistant_extra,
        );
        // Store provider-reported input tokens for precise compaction threshold checks
        if let Some(tokens) = input_tokens {
            session.metadata.insert(
                "last_input_tokens".to_string(),
                Value::Number(serde_json::Number::from(tokens)),
            );
        }
        // Persist discourse entity register for next-turn reference resolution
        discourse_register.to_session_metadata(&mut session.metadata);
        self.sessions.save(&session).await?;

        // Background fact extraction
        if let (Some(compactor), Some(assistant_content)) = (&self.compactor, &final_content)
            && self.compaction_config.extraction_enabled
            && msg.channel != "system"
        {
            let compactor = compactor.clone();
            let memory = self.memory.clone();
            let user_msg = msg.content.clone();
            let assistant_msg = assistant_content.clone();
            let task_tracker = self.task_tracker.clone();
            let task_name = format!("fact_extraction_{}", chrono::Utc::now().timestamp());
            // Use spawn_auto_cleanup since this is a one-off task that should remove itself
            task_tracker
                .spawn_auto_cleanup(task_name, async move {
                    let existing = memory.read_today_section("Facts").unwrap_or_default();
                    match compactor
                        .extract_facts(&user_msg, &assistant_msg, &existing)
                        .await
                    {
                        Ok(facts) => {
                            if !facts.is_empty() {
                                let filtered =
                                    crate::agent::memory::quality::filter_lines(&facts);
                                if filtered.trim().is_empty() {
                                    debug!("fact extraction: all lines filtered by quality gates");
                                } else if let Err(e) =
                                    memory.append_to_section("Facts", &filtered)
                                {
                                    warn!("failed to save facts to daily note: {}", e);
                                } else {
                                    debug!(
                                        "saved extracted facts to daily note ({} bytes, {} filtered)",
                                        filtered.len(),
                                        facts.len() - filtered.len()
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

        if let Some(content) = final_content {
            // Suppress sending if the LLM returned a [SILENT] response
            if content.starts_with("[SILENT]") {
                debug!("Suppressing silent response");
                return Ok(None);
            }
            Ok(Some(OutboundMessage {
                channel: msg.channel,
                chat_id: msg.chat_id,
                content,
                reply_to: None,
                media: collected_media,
                metadata: msg.metadata,
            }))
        } else {
            warn!(
                "agent loop produced no response for {}:{}",
                msg.channel, msg.chat_id
            );
            Ok(Some(OutboundMessage {
                channel: msg.channel,
                chat_id: msg.chat_id,
                content: "I wasn't able to generate a response. Please try again.".to_string(),
                reply_to: None,
                media: vec![],
                metadata: msg.metadata,
            }))
        }
    }

    /// Returns `(final_content, input_tokens, tools_used, collected_media)`.
    /// `input_tokens` is the provider-reported input token count from the most
    /// recent LLM call (if available). `tools_used` lists all tool names invoked
    /// during the loop (with duplicates). `collected_media` contains file paths
    /// of media produced by tools (screenshots, downloaded images, etc.).
    async fn run_agent_loop(
        &self,
        messages: Vec<Message>,
        typing_context: Option<(String, String)>,
        exec_ctx: &ExecutionContext,
        user_has_action_intent: bool,
    ) -> Result<(
        Option<String>,
        Option<u64>,
        Vec<String>,
        Vec<String>,
        crate::agent::discourse::DiscourseRegister,
    )> {
        self.run_agent_loop_with_overrides(
            messages,
            typing_context,
            exec_ctx,
            &AgentRunOverrides::default(),
            user_has_action_intent,
        )
        .await
    }

    /// Core agent loop implementation with per-invocation overrides.
    ///
    /// Iterates up to `max_iterations` rounds of: LLM call → parallel tool execution → append results.
    /// Uses `tool_choice=None` (auto) on all iterations — hallucination detection in
    /// `handle_text_response()` catches false action claims. At 70% of max iterations, a wrap-up
    /// nudge is injected.
    ///
    /// Returns `(response_text, last_message_id, collected_media, tool_names_used, discourse_register)`.
    async fn run_agent_loop_with_overrides(
        &self,
        mut messages: Vec<Message>,
        typing_context: Option<(String, String)>,
        exec_ctx: &ExecutionContext,
        overrides: &AgentRunOverrides,
        user_has_action_intent: bool,
    ) -> Result<(
        Option<String>,
        Option<u64>,
        Vec<String>,
        Vec<String>,
        crate::agent::discourse::DiscourseRegister,
    )> {
        let effective_model = overrides.model.as_deref().unwrap_or(&self.model);
        let effective_max_iterations = overrides.max_iterations.unwrap_or(self.max_iterations);
        let mut empty_retries_left = EMPTY_RESPONSE_RETRIES;
        let mut any_tools_called = false;
        let mut correction_state = CorrectionState::new();
        let mut last_input_tokens: Option<u64> = None;
        let mut tools_used: Vec<String> = Vec::new();
        let mut collected_media: Vec<String> = Vec::new();
        let mut checkpoint_tracker = CheckpointTracker::new(self.cognitive_config.clone());
        let mut discourse_register = crate::agent::discourse::DiscourseRegister::default();

        let tools_defs = self.tools.get_tool_definitions();

        // Exfiltration guard: hide network-outbound tools from the LLM
        let tools_defs = if self.exfiltration_guard.enabled {
            let allowed = &self.exfiltration_guard.allow_tools;
            tools_defs
                .into_iter()
                .filter(|td| {
                    let is_network = self
                        .tools
                        .get(&td.name)
                        .is_some_and(|t| t.capabilities().network_outbound);
                    !is_network || allowed.contains(&td.name)
                })
                .collect()
        } else {
            tools_defs
        };

        // Extract tool names for hallucination detection (immutable snapshot for the full loop)
        let tool_names: Vec<String> = tools_defs.iter().map(|td| td.name.clone()).collect();

        // Anti-hallucination instruction in the system prompt. The tool definitions
        // sent via the API `tools` parameter already list all available tools with
        // descriptions, so we don't duplicate the name list here — just reinforce
        // that tools ARE available and should be called directly.
        if !tool_names.is_empty()
            && let Some(system_msg) = messages.first_mut()
        {
            system_msg.content.push_str(
                "\n\nYou have tools available. If a user asks for external actions, \
                 do not claim tools are unavailable — call the matching tool directly.",
            );
        }

        // Append cognitive routines to system prompt when enabled
        if self.cognitive_config.enabled
            && let Some(system_msg) = messages.first_mut()
        {
            system_msg.content.push_str(
                "\n\n## Cognitive Routines\n\n\
                 When working on complex tasks with many tool calls:\n\
                 - Periodically summarize your progress in your responses\n\
                 - If you receive a checkpoint hint, briefly note: what's done, \
                 what's in progress, what's next\n\
                 - Keep track of your overall plan and remaining steps",
            );
        }

        let wrapup_threshold =
            (effective_max_iterations as f64 * WRAPUP_THRESHOLD_RATIO).ceil() as usize;
        // Ensure wrapup doesn't fire on the very first iteration
        let wrapup_threshold = wrapup_threshold.max(MIN_WRAPUP_ITERATION);

        for iteration in 1..=effective_max_iterations {
            // Inject wrap-up hint when approaching iteration limit
            if iteration == wrapup_threshold && any_tools_called {
                messages.push(Message::system(format!(
                    "You have used {} of {} iterations. Begin wrapping up — summarize progress and deliver results.",
                    iteration, effective_max_iterations
                )));
            }

            // Start periodic typing indicator before LLM call
            let typing_handle = start_typing(self.typing_tx.as_ref(), typing_context.as_ref());

            // Temperature strategy: use low temperature after any tool calls for
            // deterministic tool sequences, normal temperature before the first tool
            // call (initial response). The post-loop summary uses self.temperature
            // separately, so the final user-facing text always gets normal temperature.
            let current_temp = if any_tools_called {
                self.tool_temperature
            } else {
                self.temperature
            };
            // Let the model decide when to use tools (auto mode). Hallucination detection
            // in handle_text_response() catches false action claims as a safety net.
            let tool_choice: Option<String> = None;

            // Cost guard pre-flight check
            if let Some(ref cg) = self.cost_guard
                && let Err(msg) = cg.check_allowed()
            {
                warn!("cost guard blocked LLM call: {}", msg);
                if let Some(h) = typing_handle {
                    h.abort();
                }
                return Ok((
                    Some(msg),
                    last_input_tokens,
                    tools_used,
                    collected_media,
                    discourse_register,
                ));
            }

            let response = self
                .provider
                .chat_with_retry(
                    crate::providers::base::ChatRequest {
                        messages: messages.clone(),
                        tools: Some(tools_defs.clone()),
                        model: Some(effective_model),
                        max_tokens: self.max_tokens,
                        temperature: current_temp,
                        tool_choice,
                        response_format: None,
                    },
                    Some(crate::providers::base::RetryConfig::default()),
                )
                .await;

            // Stop typing indicator after LLM call returns
            if let Some(h) = typing_handle {
                h.abort();
            }

            let response = response?;

            // Track provider-reported input token count for precise compaction decisions
            if response.input_tokens.is_some() {
                last_input_tokens = response.input_tokens;
            }

            // Record cost for budget tracking
            if let Some(ref cg) = self.cost_guard {
                cg.record_llm_call(
                    effective_model,
                    response.input_tokens,
                    response.output_tokens,
                    response.cache_creation_input_tokens,
                    response.cache_read_input_tokens,
                );
            }

            if response.has_tool_calls() {
                any_tools_called = true;
                tools_used.extend(response.tool_calls.iter().map(|tc| tc.name.clone()));
                ContextBuilder::add_assistant_message(
                    &mut messages,
                    response.content.as_deref(),
                    Some(response.tool_calls.clone()),
                    response.reasoning_content.as_deref(),
                );

                // Start periodic typing indicator before tool execution
                let typing_handle = start_typing(self.typing_tx.as_ref(), typing_context.as_ref());

                let exfil_ref = if self.exfiltration_guard.enabled {
                    Some(&self.exfiltration_guard)
                } else {
                    None
                };
                let results = self
                    .execute_tools(&response.tool_calls, &tool_names, exec_ctx, exfil_ref)
                    .await;

                // Stop typing indicator after tool execution
                if let Some(h) = typing_handle {
                    h.abort();
                }

                discourse_register.advance_turn();
                self.handle_tool_results(
                    &mut messages,
                    &response.tool_calls,
                    results,
                    &mut collected_media,
                    &mut discourse_register,
                    &mut checkpoint_tracker,
                    iteration,
                )
                .await;
            } else if let Some(content) = response.content {
                // Extract entities from assistant text for reference resolution.
                // This catches entities even when the LLM summarizes tool results
                // in prose or (as a safety net) hallucinates actions.
                let text_entities =
                    crate::agent::discourse::DiscourseRegister::extract_from_assistant_text(
                        &content,
                        discourse_register.turn,
                    );
                if !text_entities.is_empty() {
                    discourse_register.register(text_entities);
                }

                match Self::handle_text_response(
                    &content,
                    &mut messages,
                    response.reasoning_content.as_deref(),
                    any_tools_called,
                    &mut correction_state,
                    &tool_names,
                    &tools_used,
                    user_has_action_intent,
                    Some(&self.memory.db()),
                ) {
                    TextAction::Continue => {}
                    TextAction::Return => {
                        return Ok((
                            Some(content),
                            last_input_tokens,
                            tools_used,
                            collected_media,
                            discourse_register,
                        ));
                    }
                }
            } else {
                // Empty response
                if empty_retries_left > 0 {
                    empty_retries_left -= 1;
                    let retry_num = EMPTY_RESPONSE_RETRIES - empty_retries_left;
                    let delay = (RETRY_BACKOFF_BASE.pow(retry_num as u32) as f64 + fastrand::f64())
                        .min(MAX_RETRY_DELAY_SECS);
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

        // If tools were called but the loop ended without final content,
        // make one more LLM call with no tools to force a text summary.
        if any_tools_called
            && let Some(content) = self
                .generate_post_loop_summary(&mut messages, effective_model)
                .await?
        {
            return Ok((
                Some(content),
                last_input_tokens,
                tools_used,
                collected_media,
                discourse_register,
            ));
        }

        Ok((
            None,
            last_input_tokens,
            tools_used,
            collected_media,
            discourse_register,
        ))
    }

    /// Execute tool calls — single-tool fast-path or parallel `spawn`+`join_all`.
    async fn execute_tools(
        &self,
        tool_calls: &[ToolCallRequest],
        tool_names: &[String],
        exec_ctx: &ExecutionContext,
        exfil_guard: Option<&crate::config::ExfiltrationGuardConfig>,
    ) -> Vec<(String, bool)> {
        let allow_tools: Option<Vec<String>> = exfil_guard.map(|g| g.allow_tools.clone());
        if tool_calls.len() == 1 {
            let tc = &tool_calls[0];
            vec![
                execute_tool_call(
                    &self.tools,
                    &tc.name,
                    &tc.arguments,
                    tool_names,
                    exec_ctx,
                    allow_tools.as_deref(),
                    Some(&self.workspace),
                )
                .await,
            ]
        } else {
            let handles: Vec<_> = tool_calls
                .iter()
                .map(|tc| {
                    let registry = self.tools.clone();
                    let tc_name = tc.name.clone();
                    let tc_args = tc.arguments.clone();
                    let available = tool_names.to_vec();
                    let ctx = exec_ctx.clone();
                    let allow = allow_tools.clone();
                    let ws = self.workspace.clone();
                    tokio::task::spawn(async move {
                        execute_tool_call(
                            &registry,
                            &tc_name,
                            &tc_args,
                            &available,
                            &ctx,
                            allow.as_deref(),
                            Some(&ws),
                        )
                        .await
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
        }
    }

    /// Collect media from tool results, extract discourse entities, scan for
    /// prompt injection, update cognitive tracking, and fire periodic checkpoints.
    #[allow(clippy::too_many_arguments)]
    async fn handle_tool_results(
        &self,
        messages: &mut Vec<Message>,
        tool_calls: &[ToolCallRequest],
        results: Vec<(String, bool)>,
        collected_media: &mut Vec<String>,
        discourse_register: &mut crate::agent::discourse::DiscourseRegister,
        checkpoint_tracker: &mut CheckpointTracker,
        iteration: usize,
    ) {
        // Add all results to messages in order and collect media
        if tool_calls.len() != results.len() {
            error!(
                "tool_calls and results length mismatch: {} vs {} — adding error results for missing entries",
                tool_calls.len(),
                results.len()
            );
            // Pad results to match tool_calls length so every tool call gets a response
            let mut results = results;
            while results.len() < tool_calls.len() {
                results.push(("Tool execution result was lost".to_string(), true));
            }
            for (tc, (result_str, is_error)) in tool_calls.iter().zip(results.into_iter()) {
                ContextBuilder::add_tool_result(messages, &tc.id, &tc.name, &result_str, is_error);
            }
            return;
        }
        for (tc, (result_str, is_error)) in tool_calls.iter().zip(results.into_iter()) {
            if !is_error {
                collected_media.extend(extract_media_paths(&result_str));
                // Extract discourse entities from successful tool results
                let entities = crate::agent::discourse::DiscourseRegister::extract_from_tool_result(
                    &tc.name,
                    &result_str,
                    discourse_register.turn,
                );
                if !entities.is_empty() {
                    discourse_register.register(entities);
                }
            }
            ContextBuilder::add_tool_result(messages, &tc.id, &tc.name, &result_str, is_error);
        }

        // Scan tool results for prompt injection (warn only)
        if let Some(ref guard) = self.prompt_guard {
            for tc in tool_calls {
                if let Some(msg) = messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some(&tc.id))
                {
                    let tool_matches = guard.scan(&msg.content);
                    for m in &tool_matches {
                        warn!(
                            "prompt injection in tool '{}' output ({:?}): {}",
                            tc.name, m.category, m.pattern_name
                        );
                    }
                }
            }
        }

        // Record tool calls for cognitive checkpoint tracking
        let called_tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
        checkpoint_tracker.record_tool_calls(&called_tool_names);

        // Inject cognitive pressure message if a new threshold was crossed
        if let Some(pressure_msg) = checkpoint_tracker.pressure_message() {
            messages.push(Message::system(pressure_msg));
        }

        // Update cognitive breadcrumb for compaction recovery
        if self.cognitive_config.enabled {
            *self.cognitive_breadcrumb.lock().await = Some(checkpoint_tracker.breadcrumb());
        }

        // Periodic checkpoint: summarize progress via compactor
        if self.compaction_config.checkpoint.enabled
            && iteration > 1
            && self.compaction_config.checkpoint.interval_iterations > 0
            && (iteration as u32)
                .is_multiple_of(self.compaction_config.checkpoint.interval_iterations)
            && let Some(ref compactor) = self.compactor
        {
            // Abort any in-flight checkpoint before spawning a new one to prevent
            // stale data from a slow old task overwriting the newer summary
            if let Some(old) = self.checkpoint_handle.lock().await.take() {
                old.abort();
            }

            let compactor = compactor.clone();
            let msgs_snapshot = messages.clone();
            let last_cp = self.last_checkpoint.clone();
            let handle = tokio::spawn(async move {
                let history: Vec<std::collections::HashMap<String, Value>> = msgs_snapshot
                    .iter()
                    .map(|m| {
                        let mut map = std::collections::HashMap::new();
                        map.insert("role".to_string(), Value::String(m.role.clone()));
                        // Annotate content with tool names so the checkpoint
                        // summary reflects tool usage in long agentic loops.
                        let content = if let Some(ref tcs) = m.tool_calls
                            && !tcs.is_empty()
                        {
                            let names: Vec<&str> = tcs.iter().map(|tc| tc.name.as_str()).collect();
                            format!("{}\n[tools used: {}]", m.content, names.join(", "))
                        } else {
                            m.content.clone()
                        };
                        map.insert("content".to_string(), Value::String(content));
                        map
                    })
                    .collect();
                match compactor.compact(&history, "").await {
                    Ok(summary) => {
                        debug!(
                            "checkpoint at iteration {}: {} chars",
                            iteration,
                            summary.len()
                        );
                        *last_cp.lock().await = Some(summary);
                    }
                    Err(e) => {
                        warn!("checkpoint generation failed: {}", e);
                    }
                }
            });
            *self.checkpoint_handle.lock().await = Some(handle);
            // Reset tracker only after spawning — the checkpoint task captures
            // the current message snapshot, so the tracker should start fresh
            // for the next interval regardless of whether compaction succeeds
            // (a failed checkpoint will be retried at the next interval anyway)
            checkpoint_tracker.reset();
        }
    }

    /// Classify user message intent and record the metric to the database.
    /// Returns `true` if the message has action intent (should trigger tool use).
    fn classify_and_record_intent(&self, content: &str) -> bool {
        let regex_intent = intent::classify_action_intent(content);
        let (semantic_result, semantic_score) = if regex_intent {
            (None, None)
        } else {
            self.memory
                .embedding_service()
                .and_then(|svc| intent::classify_action_intent_semantic(content, svc))
                .map_or((None, None), |(result, score)| (Some(result), Some(score)))
        };
        let user_action_intent = regex_intent || semantic_result.unwrap_or(false);

        let intent_method = if regex_intent {
            "regex"
        } else if semantic_result == Some(true) {
            "semantic"
        } else {
            "none"
        };
        if let Err(e) = self.memory.db().record_intent_event(
            "classification",
            Some(intent_method),
            semantic_score,
            None,
            content,
        ) {
            debug!("failed to record intent metric: {}", e);
        }

        user_action_intent
    }

    /// Handle a text-only LLM response: false no-tools correction or
    /// hallucination detection. Returns [`TextAction::Continue`] if a
    /// correction was injected, or [`TextAction::Return`] if the response is final.
    ///
    /// Detection is layered:
    /// 1. False "no tools" claim detection (LLM says it has no tools)
    /// 2. Regex-based action claim detection (fast-path for obvious hallucinations)
    /// 3. Intent-based structural detection (backstop: user asked for action + no tools called)
    #[allow(clippy::too_many_arguments)]
    fn handle_text_response(
        content: &str,
        messages: &mut Vec<Message>,
        reasoning_content: Option<&str>,
        any_tools_called: bool,
        state: &mut CorrectionState,
        tool_names: &[String],
        tools_used: &[String],
        user_has_action_intent: bool,
        db: Option<&MemoryDB>,
    ) -> TextAction {
        // Layer 0: Detect false "no tools" claims and retry with correction.
        // The LLM is factually wrong about not having tools — correct up to
        // MAX_LAYER0_CORRECTIONS times before giving up.
        if !tool_names.is_empty() && is_false_no_tools_claim(content) {
            if state.layer0_count >= MAX_LAYER0_CORRECTIONS {
                warn!(
                    "False no-tools claim persists after {} corrections, giving up",
                    MAX_LAYER0_CORRECTIONS
                );
                return TextAction::Return;
            }
            warn!(
                "False no-tools claim detected: LLM claims tools unavailable but {} tools are registered (correction {}/{})",
                tool_names.len(),
                state.layer0_count + 1,
                MAX_LAYER0_CORRECTIONS
            );
            if let Some(db) = db
                && let Err(e) = db.record_intent_event(
                    "hallucination",
                    None,
                    None,
                    Some("layer0_false_no_tools"),
                    content,
                )
            {
                debug!("failed to record hallucination metric: {}", e);
            }
            ContextBuilder::add_assistant_message(messages, Some(content), None, reasoning_content);
            let tool_list = tool_names.join(", ");
            messages.push(Message::user(format!(
                "[Internal: Your previous response was not delivered. \
                 You DO have tools available: {}. \
                 Call the appropriate tool now. Do NOT apologize or reference this correction.]",
                tool_list
            )));
            state.layer0_count += 1;
            return TextAction::Continue;
        }

        // Layer 1: Regex-based action claim detection (fast path)
        //
        // When no tools have been called, check for action claims and multi-tool
        // mentions. When tools HAVE been called, action claims (e.g. "I've updated
        // the config") are likely legitimate summaries, so skip that check — but
        // still catch mentions of tools that were never actually called (the LLM
        // embellishing what it did).
        if !state.layer1_fired {
            let trigger = if any_tools_called {
                // Only check for mentions of uncalled tools
                let uncalled: Vec<String> = tool_names
                    .iter()
                    .filter(|name| !tools_used.iter().any(|u| u == *name))
                    .cloned()
                    .collect();
                mentions_multiple_tools(content, &uncalled)
            } else {
                contains_action_claims(content) || mentions_multiple_tools(content, tool_names)
            };
            if trigger {
                warn!(
                    "Action hallucination detected: LLM claims actions but tools were not called"
                );
                if let Some(db) = db
                    && let Err(e) = db.record_intent_event(
                        "hallucination",
                        None,
                        None,
                        Some("layer1_regex"),
                        content,
                    )
                {
                    debug!("failed to record hallucination metric: {}", e);
                }
                ContextBuilder::add_assistant_message(
                    messages,
                    Some(content),
                    None,
                    reasoning_content,
                );
                messages.push(Message::user(
                    "[Internal: Your previous response was not delivered to the user. \
                     You must call the appropriate tool to perform the requested action. \
                     Do NOT apologize or mention any previous attempt — the user has no \
                     knowledge of it. Just call the tool and respond normally.]"
                        .to_string(),
                ));
                state.layer1_fired = true;
                return TextAction::Continue;
            }
        }

        // Layer 2: Intent-based structural detection (robust backstop)
        // If the user asked for an action and the LLM returned text without
        // calling tools AND the response isn't a clarification question,
        // this is a hallucination regardless of phrasing.
        if !state.layer2_fired
            && !any_tools_called
            && !tool_names.is_empty()
            && user_has_action_intent
            && !intent::is_clarification_question(content)
        {
            warn!(
                "Intent mismatch: user requested action but LLM returned text without calling tools"
            );
            if let Some(db) = db
                && let Err(e) = db.record_intent_event(
                    "hallucination",
                    None,
                    None,
                    Some("layer2_intent"),
                    content,
                )
            {
                debug!("failed to record hallucination metric: {}", e);
            }
            ContextBuilder::add_assistant_message(messages, Some(content), None, reasoning_content);
            messages.push(Message::user(
                "[Internal: Your previous response was not delivered to the user. \
                 The user is requesting an action that requires a tool call. \
                 Call the appropriate tool now. Do NOT apologize or reference \
                 this correction — the user has no knowledge of it.]"
                    .to_string(),
            ));
            state.layer2_fired = true;
            return TextAction::Continue;
        }

        TextAction::Return
    }

    /// Post-loop LLM call with no tools to force a text summary when the loop
    /// ended after tool calls without producing a final text response.
    async fn generate_post_loop_summary(
        &self,
        messages: &mut Vec<Message>,
        effective_model: &str,
    ) -> Result<Option<String>> {
        // Cost guard pre-flight check for summary call
        if let Some(ref cg) = self.cost_guard
            && let Err(msg) = cg.check_allowed()
        {
            warn!("cost guard blocked post-loop summary: {}", msg);
            return Ok(Some(msg));
        }

        messages.push(Message::user(
            "Provide a brief summary of what you accomplished for the user.".to_string(),
        ));
        match self
            .provider
            .chat_with_retry(
                crate::providers::base::ChatRequest {
                    messages: messages.clone(),
                    tools: None,
                    model: Some(effective_model),
                    max_tokens: self.max_tokens,
                    temperature: self.temperature,
                    tool_choice: None,
                    response_format: None,
                },
                Some(crate::providers::base::RetryConfig::default()),
            )
            .await
        {
            Ok(response) => {
                if let Some(ref cg) = self.cost_guard {
                    cg.record_llm_call(
                        effective_model,
                        response.input_tokens,
                        response.output_tokens,
                        response.cache_creation_input_tokens,
                        response.cache_read_input_tokens,
                    );
                }
                Ok(response.content)
            }
            Err(e) => {
                warn!("post-loop summary LLM call failed: {}", e);
                Ok(None)
            }
        }
    }

    fn build_execution_context(
        channel: &str,
        chat_id: &str,
        context_summary: Option<String>,
    ) -> ExecutionContext {
        ExecutionContext {
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            context_summary,
            metadata: HashMap::new(),
        }
    }

    fn build_execution_context_with_metadata(
        channel: &str,
        chat_id: &str,
        context_summary: Option<String>,
        metadata: HashMap<String, Value>,
    ) -> ExecutionContext {
        ExecutionContext {
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            context_summary,
            metadata,
        }
    }

    async fn get_compacted_history(
        &self,
        session: &Session,
    ) -> Result<Vec<HashMap<String, Value>>> {
        if self.compactor.is_none() || !self.compaction_config.enabled {
            return Ok(session.get_history(DEFAULT_HISTORY_SIZE));
        }

        let full_history = session.get_full_history();
        if full_history.is_empty() {
            return Ok(vec![]);
        }

        let keep_recent = self.compaction_config.keep_recent;
        let threshold = u64::from(self.compaction_config.threshold_tokens);

        // Prefer provider-reported input tokens (precise), fall back to heuristic
        let token_est = session
            .metadata
            .get("last_input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| estimate_messages_tokens(&full_history) as u64);

        if token_est < threshold {
            return Ok(session.get_history(DEFAULT_HISTORY_SIZE));
        }

        if full_history.len() <= keep_recent {
            return Ok(full_history);
        }

        let old_messages = &full_history[..full_history.len() - keep_recent];
        let recent_messages = &full_history[full_history.len() - keep_recent..];

        if old_messages.is_empty() {
            return Ok(recent_messages.to_vec());
        }

        // Get existing summary from metadata
        let previous_summary = session
            .metadata
            .get("compaction_summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Extract last user message for recovery context
        let last_user_msg = full_history
            .iter()
            .rev()
            .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
            .and_then(|m| m.get("content").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();

        // Await any in-flight checkpoint task before reading
        if let Some(handle) = self.checkpoint_handle.lock().await.take() {
            let _ = handle.await;
        }
        // Get most recent checkpoint if available
        let checkpoint = self.last_checkpoint.lock().await.clone();
        let cognitive_crumb = self.cognitive_breadcrumb.lock().await.clone();

        // Pre-compaction flush: extract important context before messages are lost
        if self.compaction_config.pre_flush_enabled
            && let Some(ref compactor) = self.compactor
        {
            // Check if we already flushed for this message count to avoid double-flush
            let old_msg_count = old_messages.len();
            let already_flushed = session
                .metadata
                .get("pre_flush_msg_count")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|c| c as usize >= old_msg_count);

            if !already_flushed {
                let mut flushed_content = false;
                match compactor.flush_to_memory(old_messages).await {
                    Ok(ref facts) if !facts.is_empty() => {
                        let filtered = crate::agent::memory::quality::filter_lines(facts);
                        if filtered.trim().is_empty() {
                            debug!("pre-compaction flush: all facts filtered by quality gates");
                        } else if let Err(e) = self
                            .memory
                            .append_to_section("Pre-compaction context", &filtered)
                        {
                            warn!("failed to write pre-compaction flush: {}", e);
                        } else {
                            debug!(
                                "pre-compaction flush: saved {} bytes to daily notes ({} filtered)",
                                filtered.len(),
                                facts.len() - filtered.len()
                            );
                            flushed_content = true;
                        }
                    }
                    Err(e) => {
                        warn!("pre-compaction flush failed (non-fatal): {}", e);
                    }
                    _ => {}
                }
                // Only mark flushed when content was actually persisted, so a
                // retry can attempt extraction again if nothing was saved.
                if flushed_content {
                    match self.sessions.get_or_create(&session.key).await {
                        Ok(mut latest) => {
                            latest.metadata.insert(
                                "pre_flush_msg_count".to_string(),
                                Value::Number(serde_json::Number::from(old_msg_count as u64)),
                            );
                            if let Err(e) = self.sessions.save(&latest).await {
                                warn!("failed to save pre-flush marker: {}", e);
                            }
                        }
                        Err(e) => warn!("failed to reload session for pre-flush marker: {}", e),
                    }
                }
            }
        }

        // Compact old messages
        if let Some(ref compactor) = self.compactor {
            match compactor.compact(old_messages, &previous_summary).await {
                Ok(summary) => {
                    // Build recovery-enriched summary
                    let mut recovery_summary = summary.clone();
                    if let Some(ref cp) = checkpoint {
                        let _ = write!(recovery_summary, "\n\n[Checkpoint] {}", cp);
                    }
                    if let Some(ref crumb) = cognitive_crumb {
                        let _ = write!(recovery_summary, "\n\n{}", crumb);
                    }
                    if !last_user_msg.is_empty() {
                        // Truncate last user message to avoid bloating the summary
                        let truncated_msg: String = last_user_msg
                            .chars()
                            .take(RECOVERY_CONTEXT_MAX_CHARS)
                            .collect();
                        let _ = write!(
                            recovery_summary,
                            "\n\n[Recovery] The conversation was compacted. \
                             Continue from where you left off. Last user request: {}",
                            truncated_msg
                        );
                    }

                    // Cache summary locally so it survives save failures
                    *self.last_checkpoint.lock().await = Some(recovery_summary.clone());

                    // Persist the enriched summary so the next compaction cycle
                    // builds incrementally on the same context the LLM actually saw
                    // (including checkpoint/recovery annotations).
                    match self.sessions.get_or_create(&session.key).await {
                        Ok(mut latest) => {
                            latest.metadata.insert(
                                "compaction_summary".to_string(),
                                Value::String(recovery_summary.clone()),
                            );
                            if let Err(e) = self.sessions.save(&latest).await {
                                warn!(
                                    "failed to persist compaction summary: {}, will retry next cycle",
                                    e
                                );
                            }
                        }
                        Err(e) => {
                            warn!("failed to reload session for compaction summary: {}", e);
                        }
                    }

                    // Return recovery-enriched summary + recent messages
                    let mut result = vec![HashMap::from([
                        ("role".to_string(), Value::String("system".to_string())),
                        (
                            "content".to_string(),
                            Value::String(format!(
                                "[Previous conversation summary: {}]",
                                recovery_summary
                            )),
                        ),
                    ])];
                    result.extend(recent_messages.iter().cloned());

                    // Strip orphaned tool messages that lost their pair during compaction
                    strip_orphaned_tool_messages(&mut result);

                    Ok(result)
                }
                Err(e) => {
                    if previous_summary.is_empty() {
                        // No previous summary — return full history (oversized but not lost)
                        warn!(
                            "compaction failed with no previous summary: {}, returning full history",
                            e
                        );
                        Ok(full_history)
                    } else {
                        // Reuse the last successful summary rather than losing all context
                        warn!("compaction failed: {}, falling back to previous summary", e);
                        let mut result = vec![HashMap::from([
                            ("role".to_string(), Value::String("system".to_string())),
                            (
                                "content".to_string(),
                                Value::String(format!(
                                    "[Previous conversation summary: {}]",
                                    previous_summary
                                )),
                            ),
                        ])];
                        result.extend(recent_messages.iter().cloned());
                        strip_orphaned_tool_messages(&mut result);
                        Ok(result)
                    }
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

        let messages = {
            let mut context = self.context.lock().await;
            context.refresh_provider_context().await;
            context.build_messages(
                &history,
                &msg.content,
                Some(origin_channel.as_str()),
                Some(origin_chat_id.as_str()),
                None,
                vec![],
                false, // background tasks are not group-scoped
                None,  // no entity context for background tasks
            )?
        };

        let typing_ctx = Some((origin_channel.clone(), origin_chat_id.clone()));
        let exec_ctx = Self::build_execution_context(&origin_channel, &origin_chat_id, None);
        let user_action_intent = self.classify_and_record_intent(&msg.content);
        let (final_content, _, tools_used, collected_media, _discourse) = self
            .run_agent_loop(messages, typing_ctx, &exec_ctx, user_action_intent)
            .await?;
        let final_content =
            final_content.unwrap_or_else(|| "Background task completed.".to_string());

        let mut session = self.sessions.get_or_create(&session_key).await?;
        let extra = HashMap::new();
        session.add_message(
            "user".to_string(),
            format!("[System: {}] {}", msg.sender_id, msg.content),
            extra.clone(),
        );
        let mut assistant_extra = HashMap::new();
        if !tools_used.is_empty() {
            assistant_extra.insert(
                "tools_used".to_string(),
                Value::Array(tools_used.into_iter().map(Value::String).collect()),
            );
        }
        session.add_message(
            "assistant".to_string(),
            final_content.clone(),
            assistant_extra,
        );
        self.sessions.save(&session).await?;

        Ok(Some(OutboundMessage {
            channel: origin_channel.clone(),
            chat_id: origin_chat_id.clone(),
            content: final_content,
            reply_to: None,
            media: collected_media,
            metadata: HashMap::new(),
        }))
    }

    /// Attempt to persist a "remember that..." message directly to memory,
    /// bypassing the LLM. Returns `Ok(Some(response))` if handled, `Ok(None)` if
    /// the caller should fall through to normal LLM processing.
    async fn try_remember_fast_path(
        &self,
        content: &str,
        session_key: &str,
    ) -> Result<Option<String>> {
        use crate::agent::memory::quality::{QualityVerdict, check_quality};
        use crate::agent::memory::remember::is_duplicate;

        // Quality gate: reject low-signal content
        let response = match check_quality(content) {
            QualityVerdict::Reject(reason) => {
                info!("remember fast path: rejected ({:?})", reason);
                "That doesn't seem like something worth remembering. Try being more specific."
                    .to_string()
            }
            QualityVerdict::Reframed(reframed) => {
                // Read today's notes for dedup (use reframed text)
                let today_notes = self.memory.read_today().unwrap_or_default();
                if is_duplicate(&reframed, &today_notes) {
                    info!("remember fast path: duplicate detected, skipping write");
                    "I already have that noted.".to_string()
                } else {
                    self.memory.append_today(&format!("\n- {}\n", reframed))?;
                    info!(
                        "remember fast path: wrote {} chars to daily notes (reframed)",
                        reframed.len()
                    );
                    format!("Noted (reframed for accuracy): {}", reframed)
                }
            }
            QualityVerdict::Pass => {
                let today_notes = self.memory.read_today().unwrap_or_default();
                if is_duplicate(content, &today_notes) {
                    info!("remember fast path: duplicate detected, skipping write");
                    "I already have that noted.".to_string()
                } else {
                    self.memory.append_today(&format!("\n- {}\n", content))?;
                    info!(
                        "remember fast path: wrote {} chars to daily notes",
                        content.len()
                    );
                    format!("Noted! I'll remember: {}", content)
                }
            }
        };

        // Single session load + save for all branches
        let mut session = self.sessions.get_or_create(session_key).await?;
        let extra = HashMap::new();
        session.add_message(
            "user".to_string(),
            format!("remember that {}", content),
            extra.clone(),
        );
        session.add_message("assistant".to_string(), response.clone(), extra);
        self.sessions.save(&session).await?;

        Ok(Some(response))
    }

    pub async fn process_direct(
        &self,
        content: &str,
        session_key: &str,
        channel: &str,
        chat_id: &str,
    ) -> Result<String> {
        self.process_direct_with_overrides(
            content,
            session_key,
            channel,
            chat_id,
            &AgentRunOverrides::default(),
        )
        .await
    }

    /// Like [`process_direct`](Self::process_direct) but accepts per-invocation
    /// overrides for model and `max_iterations` (used by daemon heartbeats).
    pub async fn process_direct_with_overrides(
        &self,
        content: &str,
        session_key: &str,
        channel: &str,
        chat_id: &str,
        overrides: &AgentRunOverrides,
    ) -> Result<String> {
        // Acquire processing lock to prevent concurrent processing
        let _lock = self.processing_lock.lock().await;

        // Prompt injection preflight check
        if let Some(ref guard) = self.prompt_guard {
            let matches = guard.scan(content);
            if !matches.is_empty() {
                for m in &matches {
                    warn!(
                        "prompt injection detected in direct call ({:?}): {}",
                        m.category, m.pattern_name
                    );
                }
                if self.prompt_guard_config.should_block() {
                    return Ok(
                        "I can't process this message as it appears to contain prompt injection patterns."
                            .to_string(),
                    );
                }
            }
        }

        let session = self.sessions.get_or_create(session_key).await?;
        let history = self.get_compacted_history(&session).await?;

        let messages = {
            let mut ctx = self.context.lock().await;
            ctx.refresh_provider_context().await;
            ctx.build_messages(
                &history,
                content,
                Some(channel),
                Some(chat_id),
                None,
                vec![],
                false, // process_direct is not group-scoped
                None,  // no entity context for direct processing
            )?
        };

        let typing_ctx = Some((channel.to_string(), chat_id.to_string()));
        let exec_ctx = Self::build_execution_context(channel, chat_id, None);
        let user_action_intent = self.classify_and_record_intent(content);

        let (response, _, tools_used, _collected_media, _discourse) = self
            .run_agent_loop_with_overrides(
                messages,
                typing_ctx,
                &exec_ctx,
                overrides,
                user_action_intent,
            )
            .await?;
        let response = response.unwrap_or_else(|| "No response generated.".to_string());

        let mut session = self.sessions.get_or_create(session_key).await?;
        let extra = HashMap::new();
        session.add_message("user".to_string(), content.to_string(), extra.clone());
        let mut assistant_extra = HashMap::new();
        if !tools_used.is_empty() {
            assistant_extra.insert(
                "tools_used".to_string(),
                Value::Array(tools_used.into_iter().map(Value::String).collect()),
            );
        }
        session.add_message("assistant".to_string(), response.clone(), assistant_extra);
        self.sessions.save(&session).await?;

        Ok(response)
    }
}

#[cfg(test)]
mod tests;
