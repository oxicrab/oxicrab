use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_checkpoint_interval", rename = "intervalIterations")]
    pub interval_iterations: u32,
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_iterations: default_checkpoint_interval(),
        }
    }
}

fn default_checkpoint_interval() -> u32 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(default = "default_threshold_tokens", rename = "thresholdTokens")]
    pub threshold_tokens: u32,
    #[serde(default = "default_keep_recent", rename = "keepRecent")]
    pub keep_recent: usize,
    #[serde(default = "super::default_true", rename = "extractionEnabled")]
    pub extraction_enabled: bool,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub checkpoint: CheckpointConfig,
    /// Before compaction, make a silent LLM call to extract important context
    /// from about-to-be-compacted messages and persist to daily notes.
    #[serde(default, rename = "preFlushEnabled")]
    pub pre_flush_enabled: bool,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_tokens: default_threshold_tokens(),
            keep_recent: default_keep_recent(),
            extraction_enabled: true,
            model: None,
            checkpoint: CheckpointConfig::default(),
            pre_flush_enabled: false,
        }
    }
}

fn default_threshold_tokens() -> u32 {
    40000
}

fn default_keep_recent() -> usize {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default, rename = "executionModel")]
    pub execution_model: Option<String>,
    #[serde(default, rename = "executionProvider")]
    pub execution_provider: Option<String>,
    #[serde(default = "default_strategy_file", rename = "strategyFile")]
    pub strategy_file: String,
    #[serde(default = "default_max_iterations", rename = "maxIterations")]
    pub max_iterations: usize,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval: default_interval(),
            execution_model: None,
            execution_provider: None,
            strategy_file: default_strategy_file(),
            max_iterations: default_max_iterations(),
        }
    }
}

fn default_interval() -> u64 {
    300
}

fn default_strategy_file() -> String {
    "HEARTBEAT.md".to_string()
}

fn default_max_iterations() -> usize {
    25
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCost {
    #[serde(default, rename = "inputPerMillion")]
    pub input_per_million: f64,
    #[serde(default, rename = "outputPerMillion")]
    pub output_per_million: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostGuardConfig {
    #[serde(default, rename = "dailyBudgetCents")]
    pub daily_budget_cents: Option<u64>,
    #[serde(default, rename = "maxActionsPerHour")]
    pub max_actions_per_hour: Option<u64>,
    #[serde(default, rename = "modelCosts")]
    pub model_costs: std::collections::HashMap<String, ModelCost>,
}

/// Action to take when prompt injection is detected.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptGuardAction {
    /// Log a warning and continue processing (default).
    #[default]
    Warn,
    /// Reject the message entirely.
    Block,
}

impl std::fmt::Display for PromptGuardAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Warn => write!(f, "warn"),
            Self::Block => write!(f, "block"),
        }
    }
}

fn default_prompt_guard_action() -> PromptGuardAction {
    PromptGuardAction::default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptGuardConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Action on detection: `Warn` (log + continue) or `Block` (reject message)
    #[serde(default = "default_prompt_guard_action")]
    pub action: PromptGuardAction,
}

impl PromptGuardConfig {
    /// Whether detected prompt injections should block the message.
    pub fn should_block(&self) -> bool {
        self.action == PromptGuardAction::Block
    }
}

impl Default for PromptGuardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            action: default_prompt_guard_action(),
        }
    }
}

fn default_gentle_threshold() -> u32 {
    12
}
fn default_firm_threshold() -> u32 {
    20
}
fn default_urgent_threshold() -> u32 {
    30
}
fn default_recent_tools_window() -> usize {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_gentle_threshold", rename = "gentleThreshold")]
    pub gentle_threshold: u32,
    #[serde(default = "default_firm_threshold", rename = "firmThreshold")]
    pub firm_threshold: u32,
    #[serde(default = "default_urgent_threshold", rename = "urgentThreshold")]
    pub urgent_threshold: u32,
    #[serde(default = "default_recent_tools_window", rename = "recentToolsWindow")]
    pub recent_tools_window: usize,
}

impl Default for CognitiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gentle_threshold: default_gentle_threshold(),
            firm_threshold: default_firm_threshold(),
            urgent_threshold: default_urgent_threshold(),
            recent_tools_window: default_recent_tools_window(),
        }
    }
}

fn default_memory_indexer_interval() -> u64 {
    300
}

fn default_memory_archive_after_days() -> u32 {
    30
}

fn default_memory_purge_after_days() -> u32 {
    90
}

fn default_embeddings_model() -> String {
    "BAAI/bge-small-en-v1.5".to_string()
}

fn default_hybrid_weight() -> f32 {
    0.5
}

/// Fusion strategy for combining keyword and vector search results.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FusionStrategy {
    /// Weighted linear combination of normalized scores (default).
    #[default]
    WeightedScore,
    /// Reciprocal Rank Fusion — merges by rank position, ignoring raw scores.
    Rrf,
}

fn default_rrf_k() -> u32 {
    60
}

fn default_embedding_cache_size() -> usize {
    10_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(
        default = "default_memory_archive_after_days",
        rename = "archiveAfterDays"
    )]
    pub archive_after_days: u32,
    #[serde(default = "default_memory_purge_after_days", rename = "purgeAfterDays")]
    pub purge_after_days: u32,
    #[serde(default, rename = "embeddingsEnabled")]
    pub embeddings_enabled: bool,
    #[serde(default = "default_embeddings_model", rename = "embeddingsModel")]
    pub embeddings_model: String,
    /// 0.0 = keyword only, 1.0 = vector only, 0.5 = equal blend (used with `WeightedScore`)
    #[serde(default = "default_hybrid_weight", rename = "hybridWeight")]
    pub hybrid_weight: f32,
    /// Strategy for combining keyword and vector search results.
    #[serde(default, rename = "searchFusionStrategy")]
    pub fusion_strategy: FusionStrategy,
    /// Constant k for RRF (higher = less emphasis on top ranks). Default 60.
    #[serde(default = "default_rrf_k", rename = "rrfK")]
    pub rrf_k: u32,
    /// LRU cache size for query embeddings. Default 10,000.
    #[serde(
        default = "default_embedding_cache_size",
        rename = "embeddingCacheSize"
    )]
    pub embedding_cache_size: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            archive_after_days: default_memory_archive_after_days(),
            purge_after_days: default_memory_purge_after_days(),
            embeddings_enabled: false,
            embeddings_model: default_embeddings_model(),
            hybrid_weight: default_hybrid_weight(),
            fusion_strategy: FusionStrategy::default(),
            rrf_k: default_rrf_k(),
            embedding_cache_size: default_embedding_cache_size(),
        }
    }
}

fn default_media_ttl_days() -> u32 {
    7
}

fn default_max_concurrent_subagents() -> usize {
    5
}

fn default_session_ttl_days() -> u32 {
    30
}

fn default_workspace() -> String {
    "~/.oxicrab/workspace".to_string()
}

fn default_model() -> String {
    "claude-sonnet-4-5-20250929".to_string()
}

fn default_max_tokens() -> u32 {
    8192
}

fn default_temperature() -> f32 {
    0.7
}

fn default_max_tool_iterations() -> usize {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefaults {
    #[serde(default = "default_workspace")]
    pub workspace: String,
    #[serde(default = "default_model")]
    pub model: String,
    /// Explicit LLM provider override. When set, bypasses model-name inference.
    /// Examples: "anthropic", "openai", "groq", "ollama"
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default = "default_max_tokens", rename = "maxTokens")]
    pub max_tokens: u32,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_max_tool_iterations", rename = "maxToolIterations")]
    pub max_tool_iterations: usize,
    #[serde(default)]
    pub compaction: CompactionConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default = "default_session_ttl_days", rename = "sessionTtlDays")]
    pub session_ttl_days: u32,
    #[serde(
        default = "default_memory_indexer_interval",
        rename = "memoryIndexerInterval"
    )]
    pub memory_indexer_interval: u64,
    #[serde(default = "default_media_ttl_days", rename = "mediaTtlDays")]
    pub media_ttl_days: u32,
    #[serde(
        default = "default_max_concurrent_subagents",
        rename = "maxConcurrentSubagents"
    )]
    pub max_concurrent_subagents: usize,
    #[serde(default, rename = "localModel")]
    pub local_model: Option<String>,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default, rename = "costGuard")]
    pub cost_guard: CostGuardConfig,
    #[serde(default)]
    pub cognitive: CognitiveConfig,
    #[serde(default, rename = "promptGuard")]
    pub prompt_guard: PromptGuardConfig,
    #[serde(default, rename = "contextProviders")]
    pub context_providers: Vec<ContextProviderConfig>,
}

impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            workspace: default_workspace(),
            model: default_model(),
            provider: None,
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            max_tool_iterations: default_max_tool_iterations(),
            compaction: CompactionConfig::default(),
            daemon: DaemonConfig::default(),
            session_ttl_days: default_session_ttl_days(),
            memory_indexer_interval: default_memory_indexer_interval(),
            media_ttl_days: default_media_ttl_days(),
            max_concurrent_subagents: default_max_concurrent_subagents(),
            local_model: None,
            memory: MemoryConfig::default(),
            cost_guard: CostGuardConfig::default(),
            cognitive: CognitiveConfig::default(),
            prompt_guard: PromptGuardConfig::default(),
            context_providers: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextProviderConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(default = "default_context_provider_timeout")]
    pub timeout: u64,
    #[serde(default = "default_context_provider_ttl")]
    pub ttl: u64,
    #[serde(default, rename = "requiresBins")]
    pub requires_bins: Vec<String>,
    #[serde(default, rename = "requiresEnv")]
    pub requires_env: Vec<String>,
}

fn default_context_provider_timeout() -> u64 {
    5
}

fn default_context_provider_ttl() -> u64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentsConfig {
    #[serde(default)]
    pub defaults: AgentDefaults,
}
