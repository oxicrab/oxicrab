use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(default = "default_threshold_tokens", rename = "thresholdTokens")]
    pub threshold_tokens: u32,
    #[serde(default = "default_keep_recent", rename = "keepRecent")]
    pub keep_recent: usize,
    #[serde(default, rename = "keepRecentTurns")]
    pub keep_recent_turns: Option<usize>,
    #[serde(default = "super::default_true", rename = "extractionEnabled")]
    pub extraction_enabled: bool,
    #[serde(default)]
    pub model: Option<String>,
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
            keep_recent_turns: None,
            extraction_enabled: true,
            model: None,
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
    #[serde(default = "super::default_true")]
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
            enabled: true,
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

fn default_search_result_limit() -> usize {
    8
}

fn default_retention_days() -> u32 {
    180
}

fn default_max_context_chars() -> usize {
    4000
}

fn default_embeddings_enabled() -> bool {
    true
}

fn default_recency_half_life_days() -> u32 {
    90
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_embeddings_enabled", rename = "embeddingsEnabled")]
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
    /// Half-life in days for BM25 recency decay. Older entries get lower keyword scores.
    /// 0 = no decay (default: 90).
    #[serde(
        default = "default_recency_half_life_days",
        rename = "recencyHalfLifeDays"
    )]
    pub recency_half_life_days: u32,
    /// Maximum number of search results returned for memory context (default: 8).
    #[serde(default = "default_search_result_limit", rename = "searchResultLimit")]
    pub search_result_limit: usize,
    /// Days to retain memory entries before purging (default: 180).
    /// Knowledge entries (`knowledge:` prefix) are never purged regardless of this setting.
    #[serde(default = "default_retention_days", rename = "retentionDays")]
    pub retention_days: u32,
    /// Maximum total characters of memory context injected into the system prompt
    /// (default: 4000, ~1000 tokens). Prevents memory from consuming too much of
    /// the context window.
    #[serde(default = "default_max_context_chars", rename = "maxContextChars")]
    pub max_context_chars: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            embeddings_enabled: default_embeddings_enabled(),
            embeddings_model: default_embeddings_model(),
            hybrid_weight: default_hybrid_weight(),
            fusion_strategy: FusionStrategy::default(),
            rrf_k: default_rrf_k(),
            embedding_cache_size: default_embedding_cache_size(),
            recency_half_life_days: default_recency_half_life_days(),
            search_result_limit: default_search_result_limit(),
            retention_days: default_retention_days(),
            max_context_chars: default_max_context_chars(),
        }
    }
}

fn default_media_ttl_days() -> u32 {
    7
}

// Serde default functions must match the field type (`Option<u64>`).
#[allow(clippy::unnecessary_wraps)]
fn default_ttl_temp() -> Option<u64> {
    Some(7)
}
#[allow(clippy::unnecessary_wraps)]
fn default_ttl_downloads() -> Option<u64> {
    Some(30)
}
#[allow(clippy::unnecessary_wraps)]
fn default_ttl_images() -> Option<u64> {
    Some(90)
}

/// Per-category TTL defaults for workspace file expiration.
///
/// Each field is `Option<u64>` where `Some(days)` means files expire after
/// that many days, and `None` means files never expire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceTtlConfig {
    /// Days before temp files expire (default: 7). Null = never.
    #[serde(default = "default_ttl_temp")]
    pub temp: Option<u64>,
    /// Days before downloads expire (default: 30). Null = never.
    #[serde(default = "default_ttl_downloads")]
    pub downloads: Option<u64>,
    /// Days before images expire (default: 90). Null = never.
    #[serde(default = "default_ttl_images")]
    pub images: Option<u64>,
    /// Days before code files expire. Null = never (default).
    #[serde(default)]
    pub code: Option<u64>,
    /// Days before document files expire. Null = never (default).
    #[serde(default)]
    pub documents: Option<u64>,
    /// Days before data files expire. Null = never (default).
    #[serde(default)]
    pub data: Option<u64>,
}

impl Default for WorkspaceTtlConfig {
    fn default() -> Self {
        Self {
            temp: default_ttl_temp(),
            downloads: default_ttl_downloads(),
            images: default_ttl_images(),
            code: None,
            documents: None,
            data: None,
        }
    }
}

impl WorkspaceTtlConfig {
    pub fn to_map(&self) -> std::collections::HashMap<String, Option<u64>> {
        let mut map = std::collections::HashMap::new();
        map.insert("temp".into(), self.temp);
        map.insert("downloads".into(), self.downloads);
        map.insert("images".into(), self.images);
        map.insert("code".into(), self.code);
        map.insert("documents".into(), self.documents);
        map.insert("data".into(), self.data);
        map
    }
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

#[allow(clippy::unnecessary_wraps)]
fn default_temperature() -> Option<f32> {
    Some(0.7)
}

fn default_max_tool_iterations() -> usize {
    20
}

/// Simplified model routing: `default` model, per-task overrides, fallback chain.
///
/// - `default`: base `provider/model` string used for all tasks unless overridden
/// - `tasks`: maps task types to models (strings) or complex chat routing (object)
/// - `fallbacks`: ordered `provider/model` chain for provider resilience
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRoutingConfig {
    /// Base model used when no task-specific override matches.
    #[serde(default = "default_model")]
    pub default: String,
    /// Ordered fallback chain of `provider/model` strings for resilience.
    #[serde(default)]
    pub fallbacks: Vec<String>,
    /// Per-task model overrides. Simple tasks (cron, compaction, subagent)
    /// use a plain model string. The `chat` key accepts an object with complexity
    /// escalation thresholds.
    #[serde(default)]
    pub tasks: std::collections::HashMap<String, TaskRouting>,
}

impl Default for ModelRoutingConfig {
    fn default() -> Self {
        Self {
            default: default_model(),
            fallbacks: Vec::new(),
            tasks: std::collections::HashMap::new(),
        }
    }
}

/// Per-task routing: either a simple model string or a chat routing config
/// with complexity escalation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TaskRouting {
    /// Simple model override: `"cron": "anthropic/claude-haiku-4-5-20251001"`
    Model(String),
    /// Chat routing with complexity-based model escalation.
    Chat(ChatRoutingConfig),
}

/// Chat routing with complexity-based model escalation.
/// When complexity scoring crosses thresholds, the request is routed to a
/// more capable (and typically more expensive) model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRoutingConfig {
    /// Score thresholds for model escalation.
    #[serde(default)]
    pub thresholds: ChatThresholds,
    /// Models to use at each complexity tier above the default.
    pub models: ChatModels,
    /// Per-dimension scoring weights for the complexity scorer.
    #[serde(default)]
    pub weights: ComplexityWeights,
}

/// Score thresholds for chat complexity escalation.
/// Below `standard` → use the default model.
/// Between `standard` and `heavy` → use `models.standard`.
/// At or above `heavy` → use `models.heavy`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatThresholds {
    /// Below this score → default model (default 0.3).
    #[serde(default = "default_standard_threshold")]
    pub standard: f64,
    /// At or above this score → heavy model (default 0.65).
    #[serde(default = "default_heavy_threshold")]
    pub heavy: f64,
}

fn default_standard_threshold() -> f64 {
    0.3
}

fn default_heavy_threshold() -> f64 {
    0.65
}

impl Default for ChatThresholds {
    fn default() -> Self {
        Self {
            standard: default_standard_threshold(),
            heavy: default_heavy_threshold(),
        }
    }
}

/// Models for chat complexity escalation tiers (above the default model).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatModels {
    /// Model used for medium-complexity messages.
    pub standard: String,
    /// Model used for high-complexity messages.
    pub heavy: String,
}

/// Per-dimension scoring weights. Negative weight on conversational simplicity
/// pulls the score down for greetings/filler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityWeights {
    #[serde(default = "default_message_length_weight", rename = "messageLength")]
    pub message_length: f64,
    #[serde(
        default = "default_reasoning_keywords_weight",
        rename = "reasoningKeywords"
    )]
    pub reasoning_keywords: f64,
    #[serde(
        default = "default_technical_vocabulary_weight",
        rename = "technicalVocabulary"
    )]
    pub technical_vocabulary: f64,
    #[serde(
        default = "default_question_complexity_weight",
        rename = "questionComplexity"
    )]
    pub question_complexity: f64,
    #[serde(default = "default_code_presence_weight", rename = "codePresence")]
    pub code_presence: f64,
    #[serde(
        default = "default_instruction_complexity_weight",
        rename = "instructionComplexity"
    )]
    pub instruction_complexity: f64,
    #[serde(
        default = "default_conversational_simplicity_weight",
        rename = "conversationalSimplicity"
    )]
    pub conversational_simplicity: f64,
}

fn default_message_length_weight() -> f64 {
    0.10
}
fn default_reasoning_keywords_weight() -> f64 {
    0.30
}
fn default_technical_vocabulary_weight() -> f64 {
    0.15
}
fn default_question_complexity_weight() -> f64 {
    0.15
}
fn default_code_presence_weight() -> f64 {
    0.10
}
fn default_instruction_complexity_weight() -> f64 {
    0.15
}
fn default_conversational_simplicity_weight() -> f64 {
    -0.20
}

impl Default for ComplexityWeights {
    fn default() -> Self {
        Self {
            message_length: default_message_length_weight(),
            reasoning_keywords: default_reasoning_keywords_weight(),
            technical_vocabulary: default_technical_vocabulary_weight(),
            question_complexity: default_question_complexity_weight(),
            code_presence: default_code_presence_weight(),
            instruction_complexity: default_instruction_complexity_weight(),
            conversational_simplicity: default_conversational_simplicity_weight(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefaults {
    #[serde(default = "default_workspace")]
    pub workspace: String,
    #[serde(default = "default_max_tokens", rename = "maxTokens")]
    pub max_tokens: u32,
    #[serde(default = "default_temperature")]
    pub temperature: Option<f32>,
    #[serde(default = "default_max_tool_iterations", rename = "maxToolIterations")]
    pub max_tool_iterations: usize,
    #[serde(default)]
    pub compaction: CompactionConfig,
    #[serde(default = "default_session_ttl_days", rename = "sessionTtlDays")]
    pub session_ttl_days: u32,
    #[serde(default = "default_media_ttl_days", rename = "mediaTtlDays")]
    pub media_ttl_days: u32,
    #[serde(
        default = "default_max_concurrent_subagents",
        rename = "maxConcurrentSubagents"
    )]
    pub max_concurrent_subagents: usize,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub cognitive: CognitiveConfig,
    #[serde(default, rename = "promptGuard")]
    pub prompt_guard: PromptGuardConfig,
    #[serde(default, rename = "contextProviders")]
    pub context_providers: Vec<ContextProviderConfig>,
    #[serde(default, rename = "workspaceTtl")]
    pub workspace_ttl: WorkspaceTtlConfig,
    #[serde(default, rename = "modelRouting")]
    pub model_routing: ModelRoutingConfig,
}

impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            workspace: default_workspace(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            max_tool_iterations: default_max_tool_iterations(),
            compaction: CompactionConfig::default(),
            session_ttl_days: default_session_ttl_days(),
            media_ttl_days: default_media_ttl_days(),
            max_concurrent_subagents: default_max_concurrent_subagents(),
            memory: MemoryConfig::default(),
            cognitive: CognitiveConfig::default(),
            prompt_guard: PromptGuardConfig::default(),
            context_providers: vec![],
            workspace_ttl: WorkspaceTtlConfig::default(),
            model_routing: ModelRoutingConfig::default(),
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
