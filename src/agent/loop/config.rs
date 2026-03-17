use crate::bus::{MessageBus, OutboundMessage};
use crate::cron::service::CronService;
use crate::providers::base::LLMProvider;
use crate::safety::LeakDetector;
use std::path::PathBuf;
use std::sync::Arc;

/// Per-invocation overrides for the agent loop. Allows callers (e.g. cron jobs)
/// to use a different model or iteration cap without constructing a separate
/// `AgentLoop`.
#[derive(Default, Clone)]
pub struct AgentRunOverrides {
    /// Override the model used for LLM calls.
    pub model: Option<String>,
    /// Override the maximum number of iterations.
    pub max_iterations: Option<usize>,
    /// Override the LLM provider for cross-provider routing.
    pub provider: Option<Arc<dyn LLMProvider>>,
    /// Request structured output format from the LLM (JSON mode or JSON schema).
    pub response_format: Option<crate::providers::base::ResponseFormat>,
    /// Correlation ID for tracing a single request across cost, intent, and
    /// complexity records.
    pub request_id: Option<String>,
    /// Extra metadata to inject into the tool [`ExecutionContext`].
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
    /// Structured action dispatch — bypasses LLM when Some.
    pub action: Option<crate::dispatch::ActionDispatch>,
    /// Tool name filter for `GuidedLLM` path — only include these tools.
    pub tool_filter: Option<Vec<String>>,
    /// Context hint injected into system prompt for `GuidedLLM` path.
    pub context_hint: Option<String>,
}

/// Tool-specific configurations bundled together. These fields are only used
/// to construct [`ToolBuildContext`] during [`AgentLoop::new`] — grouping them
/// reduces `AgentLoopConfig` field count and makes adding new tools cheaper
/// (only touch this struct + `from_config` + `ToolBuildContext`).
pub struct ToolConfigs {
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
    pub workspace_ttl: crate::config::WorkspaceTtlConfig,
    pub rss_config: Option<crate::config::RssConfig>,
}

/// Result of a single agent loop run.
pub struct AgentLoopResult {
    /// Final text response from the agent (if any).
    pub content: Option<String>,
    /// Input token count from the last LLM call (for compaction threshold checks).
    pub input_tokens: Option<u64>,
    /// Names of tools invoked during the loop.
    pub tools_used: Vec<String>,
    /// Filesystem paths of media produced by tools (screenshots, generated images, etc.).
    pub media: Vec<String>,
    /// Reasoning content (extended thinking) from the final LLM response.
    pub reasoning_content: Option<String>,
    /// Reasoning signature for verifying reasoning block continuity.
    pub reasoning_signature: Option<String>,
    /// Extra metadata to merge into the outbound message (e.g. interactive buttons).
    pub response_metadata: std::collections::HashMap<String, serde_json::Value>,
    /// Metadata from tool results (for directive extraction by caller).
    pub tool_metadata: Vec<(String, std::collections::HashMap<String, serde_json::Value>)>,
}

/// Result of a direct (non-channel) agent invocation.
///
/// Wraps the response text with metadata so callers like cron can
/// forward interactive buttons and other structured data to channels.
pub struct DirectResult {
    /// Agent response text.
    pub content: String,
    /// Extra metadata (e.g. interactive buttons) to merge into outbound messages.
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

/// Lifecycle-related configuration (TTLs, intervals).
pub struct LifecycleConfig {
    /// Session TTL in days for cleanup (default 30)
    pub session_ttl_days: u32,
    /// Media file TTL in days for cleanup (default 7)
    pub media_ttl_days: u32,
}

/// Safety and guardrail configuration.
pub struct SafetyConfig {
    /// Exfiltration guard configuration for hiding outbound tools from LLM
    pub exfiltration_guard: crate::config::ExfiltrationGuardConfig,
    /// Prompt injection detection configuration
    pub prompt_guard: crate::config::PromptGuardConfig,
}

/// Configuration for creating an [`AgentLoop`] instance.
pub struct AgentLoopConfig {
    pub bus: Arc<MessageBus>,
    pub provider: Arc<dyn LLMProvider>,
    pub workspace: PathBuf,
    pub model: Option<String>,
    pub max_iterations: usize,
    pub compaction_config: crate::config::CompactionConfig,
    pub outbound_tx: Arc<tokio::sync::mpsc::Sender<OutboundMessage>>,
    pub cron_service: Option<Arc<CronService>>,
    /// Temperature for response generation (default Some(0.7), None = provider default)
    pub temperature: Option<f32>,
    /// Temperature for tool-calling iterations (default Some(0.0) for determinism)
    pub tool_temperature: Option<f32>,
    /// Per-provider temperature override (e.g. Moonshot requires temperature=1).
    /// When set, passed to the compactor to override its hardcoded internal temps.
    pub per_provider_temperature: Option<f32>,
    /// Max tokens for LLM responses (default 8192)
    pub max_tokens: u32,
    /// Sender for typing indicator events (channel, `chat_id`)
    pub typing_tx: Option<Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
    /// Maximum concurrent subagents (default 5)
    pub max_concurrent_subagents: usize,
    /// Voice transcription configuration
    pub voice_config: Option<crate::config::VoiceConfig>,
    /// Memory configuration (archive/purge days)
    pub memory_config: Option<crate::config::MemoryConfig>,
    /// Cognitive routines configuration for checkpoint pressure signals
    pub cognitive_config: crate::config::CognitiveConfig,
    /// External context providers that inject dynamic content into the system prompt
    pub context_providers: Vec<crate::config::ContextProviderConfig>,
    /// Tool-specific configurations (forwarded to [`ToolBuildContext`])
    pub tool_configs: ToolConfigs,
    /// Pre-resolved model routing (maps task types to providers/models).
    pub routing: Option<Arc<crate::config::routing::ResolvedRouting>>,
    /// Lifecycle TTLs and intervals
    pub lifecycle: LifecycleConfig,
    /// Safety guardrails
    pub safety: SafetyConfig,
    /// Pre-opened `MemoryDB` to share with the agent (avoids duplicate connections).
    /// When `Some`, the agent reuses this DB instead of opening its own.
    pub memory_db: Option<Arc<crate::agent::memory::memory_db::MemoryDB>>,
    /// Shared leak detector with known secrets pre-registered.
    /// When `None`, a default detector (base patterns only) is created.
    pub leak_detector: Option<Arc<LeakDetector>>,
    /// Router configuration (prefix commands, user-defined rules).
    pub router_config: crate::config::RouterConfig,
}

/// Temperature used for tool-calling iterations (low for determinism)
pub(super) const TOOL_TEMPERATURE: Option<f32> = Some(0.0);

/// Runtime parameters for [`AgentLoopConfig::from_config`] that vary per
/// invocation (as opposed to values read from the config file).
pub struct AgentLoopRuntimeParams {
    pub bus: Arc<MessageBus>,
    pub provider: Arc<dyn LLMProvider>,
    pub model: Option<String>,
    pub outbound_tx: Arc<tokio::sync::mpsc::Sender<OutboundMessage>>,
    pub cron_service: Option<Arc<CronService>>,
    pub typing_tx: Option<Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
    pub channels_config: Option<crate::config::ChannelsConfig>,
    /// Pre-opened `MemoryDB` to share (avoids duplicate connections to same file).
    pub memory_db: Option<Arc<crate::agent::memory::memory_db::MemoryDB>>,
    /// Shared leak detector with known secrets pre-registered.
    pub leak_detector: Option<Arc<LeakDetector>>,
}

impl AgentLoopConfig {
    /// Build an `AgentLoopConfig` from the application [`Config`](crate::config::Config)
    /// and runtime parameters that vary per invocation.
    pub fn from_config(
        config: &crate::config::Config,
        params: AgentLoopRuntimeParams,
        routing: Option<Arc<crate::config::routing::ResolvedRouting>>,
    ) -> Self {
        let mut image_gen = config.tools.image_gen.clone();
        if image_gen.enabled {
            if !config.providers.openai.api_key.is_empty() {
                image_gen.openai_api_key = Some(config.providers.openai.api_key.clone());
            }
            if !config.providers.gemini.api_key.is_empty() {
                image_gen.google_api_key = Some(config.providers.gemini.api_key.clone());
            }
        }

        // Resolve per-provider temperature before consuming params.model.
        // When a provider sets a per-provider temperature, use it for both
        // normal and tool iterations — some models (e.g. kimi-k2.5) reject
        // any temperature other than their configured value.
        let per_provider_temp = config.providers.get_temperature_for_model(
            params
                .model
                .as_deref()
                .unwrap_or(&config.agents.defaults.model_routing.default),
        );
        let resolved_temperature =
            per_provider_temp.map_or(config.agents.defaults.temperature, Some);
        let resolved_tool_temperature = if per_provider_temp.is_some() {
            // Per-provider override takes precedence over hardcoded 0.0
            resolved_temperature
        } else {
            TOOL_TEMPERATURE
        };

        Self {
            bus: params.bus,
            provider: params.provider,
            workspace: config.workspace_path(),
            model: params.model,
            max_iterations: config.agents.defaults.max_tool_iterations,
            compaction_config: config.agents.defaults.compaction.clone(),
            outbound_tx: params.outbound_tx,
            cron_service: params.cron_service,
            temperature: resolved_temperature,
            tool_temperature: resolved_tool_temperature,
            per_provider_temperature: per_provider_temp,
            max_tokens: config.agents.defaults.max_tokens,
            typing_tx: params.typing_tx,
            max_concurrent_subagents: config.agents.defaults.max_concurrent_subagents,
            voice_config: Some(config.voice.clone()),
            memory_config: Some(config.agents.defaults.memory.clone()),
            cognitive_config: config.agents.defaults.cognitive.clone(),
            context_providers: config.agents.defaults.context_providers.clone(),
            tool_configs: ToolConfigs {
                web_search_config: Some(config.tools.web_search.clone()),
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
                workspace_ttl: config.agents.defaults.workspace_ttl.clone(),
                rss_config: Some(config.tools.rss.clone()),
            },
            routing,
            lifecycle: LifecycleConfig {
                session_ttl_days: config.agents.defaults.session_ttl_days,
                media_ttl_days: config.agents.defaults.media_ttl_days,
            },
            safety: SafetyConfig {
                exfiltration_guard: config.tools.exfiltration_guard.clone(),
                prompt_guard: config.agents.defaults.prompt_guard.clone(),
            },
            memory_db: params.memory_db,
            leak_detector: params.leak_detector,
            router_config: config.router.clone(),
        }
    }

    /// Create a config with sensible test defaults. Only `bus`, `provider`,
    /// `workspace`, and `outbound_tx` are required; everything else gets
    /// minimal/disabled defaults.
    #[doc(hidden)]
    pub fn test_defaults(
        bus: Arc<MessageBus>,
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
                pre_flush_enabled: false,
            },
            outbound_tx,
            cron_service: None,
            temperature: Some(0.7),
            tool_temperature: Some(0.0),
            per_provider_temperature: None,
            max_tokens: 8192,
            typing_tx: None,
            max_concurrent_subagents: 5,
            voice_config: None,
            memory_config: None,
            cognitive_config: crate::config::CognitiveConfig::default(),
            context_providers: vec![],
            tool_configs: ToolConfigs {
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
                workspace_ttl: crate::config::WorkspaceTtlConfig::default(),
                rss_config: None,
            },
            routing: None,
            lifecycle: LifecycleConfig {
                session_ttl_days: 0,
                media_ttl_days: 0,
            },
            safety: SafetyConfig {
                exfiltration_guard: crate::config::ExfiltrationGuardConfig::default(),
                prompt_guard: crate::config::PromptGuardConfig::default(),
            },
            memory_db: None,
            leak_detector: None,
            router_config: crate::config::RouterConfig::default(),
        }
    }
}
