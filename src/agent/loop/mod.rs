use crate::agent::cognitive::CheckpointTracker;
use crate::agent::compaction::{MessageCompactor, estimate_messages_tokens};
use crate::agent::context::ContextBuilder;
use crate::agent::cost_guard::CostGuard;
use crate::agent::memory::MemoryStore;
use crate::agent::subagent::{SubagentConfig, SubagentManager};
use crate::agent::tools::ToolRegistry;
use crate::agent::tools::base::ExecutionContext;
use crate::agent::tools::setup::ToolBuildContext;
use crate::bus::{InboundMessage, MessageBus, OutboundMessage};
use crate::cron::event_matcher::EventMatcher;
use crate::cron::service::CronService;
use crate::providers::base::{ImageData, LLMProvider, Message, ToolCallRequest};
use crate::session::{Session, SessionManager, SessionStore};
use crate::utils::task_tracker::TaskTracker;
use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

const EMPTY_RESPONSE_RETRIES: usize = 2;
const WRAPUP_THRESHOLD_RATIO: f64 = 0.7;
const MIN_WRAPUP_ITERATION: usize = 2;
const TOOL_MENTION_HALLUCINATION_THRESHOLD: usize = 3;
const TYPING_INDICATOR_INTERVAL_SECS: u64 = 4;
const RETRY_BACKOFF_BASE: u64 = 2;
const MAX_RETRY_DELAY_SECS: f64 = 10.0;
const DEFAULT_HISTORY_SIZE: usize = 50;
const RECOVERY_CONTEXT_MAX_CHARS: usize = 200;
const SAVED_TO_PREFIX: &str = "saved to: ";
const AUDIO_TAG_PREFIX: &str = "[audio: ";

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

/// Extract media file paths from a tool result string.
///
/// Looks for:
/// - JSON `"mediaPath"` fields (from `web_fetch` / `http` binary downloads)
/// - "Screenshot saved to: /path" or "Binary content saved to: /path" patterns
fn extract_media_paths(result: &str) -> Vec<String> {
    let mut paths = Vec::new();

    // Try JSON parsing for mediaPath
    if let Ok(json) = serde_json::from_str::<Value>(result)
        && let Some(path) = json.get("mediaPath").and_then(Value::as_str)
        && std::path::Path::new(path).exists()
    {
        paths.push(path.to_string());
    }

    // Text pattern: "saved to: /path" (browser screenshots, http binary)
    for line in result.lines() {
        if let Some(idx) = line.find(SAVED_TO_PREFIX) {
            let path = line[idx + SAVED_TO_PREFIX.len()..].trim();
            if !path.is_empty() && std::path::Path::new(path).exists() {
                paths.push(path.to_string());
            }
        }
    }

    paths.sort();
    paths.dedup();
    paths
}

/// Validate tool arguments against the tool's JSON schema.
/// Checks: (1) required fields are present, (2) field types match schema.
/// Returns None if valid, `Some(error_message)` if invalid.
pub(crate) fn validate_tool_params(
    tool: &dyn crate::agent::tools::base::Tool,
    params: &Value,
) -> Option<String> {
    let schema = tool.parameters();
    let mut errors = Vec::new();

    // Check required fields
    if let Some(required) = schema["required"].as_array() {
        for field in required {
            if let Some(field_name) = field.as_str()
                && (params.get(field_name).is_none() || params[field_name].is_null())
            {
                errors.push(format!("missing required parameter '{}'", field_name));
            }
        }
    }

    // Check types of provided fields
    if let Some(properties) = schema["properties"].as_object() {
        for (field_name, field_schema) in properties {
            if let Some(value) = params.get(field_name)
                && !value.is_null()
                && let Some(expected_type) = field_schema["type"].as_str()
            {
                let type_ok = match expected_type {
                    "string" => value.is_string(),
                    "number" | "integer" => value.is_number(),
                    "boolean" => value.is_boolean(),
                    "array" => value.is_array(),
                    "object" => value.is_object(),
                    _ => true,
                };
                if !type_ok {
                    errors.push(format!(
                        "parameter '{}' should be {} but got {}",
                        field_name,
                        expected_type,
                        value_type_name(value)
                    ));
                }
            }
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(format!(
            "Invalid arguments for tool '{}': {}",
            tool.name(),
            errors.join("; ")
        ))
    }
}

fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "boolean",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Null => "null",
    }
}

/// Execute a tool call via the registry's middleware pipeline.
///
/// The registry handles: param validation, caching, timeout, panic isolation,
/// truncation, and logging. This function is a thin wrapper that handles the
/// "tool not found" case and converts the result to `(String, bool)`.
async fn execute_tool_call(
    registry: &ToolRegistry,
    tc_name: &str,
    tc_args: &Value,
    available_tools: &[String],
    ctx: &ExecutionContext,
    exfil_blocked: &[String],
    workspace: Option<&std::path::Path>,
) -> (String, bool) {
    // Exfiltration guard: block tools that were hidden from the LLM
    if !exfil_blocked.is_empty() && exfil_blocked.iter().any(|b| b == tc_name) {
        warn!("exfiltration guard blocked tool: {}", tc_name);
        return (
            "Error: this tool is not available in the current security mode".to_string(),
            true,
        );
    }

    // Check if tool exists before delegating to registry
    let Some(tool) = registry.get(tc_name) else {
        warn!("LLM called unknown tool: {}", tc_name);
        return (
            format!(
                "Error: tool '{}' does not exist. Available tools: {}",
                tc_name,
                available_tools.join(", ")
            ),
            true,
        );
    };

    // Approval gate: block untrusted MCP tools
    if tool.requires_approval() {
        warn!("blocked untrusted MCP tool: {}", tc_name);
        return (
            format!(
                "Error: tool '{}' is from an untrusted MCP server and requires approval. \
                 Change the server's trust level to \"local\" in config to allow execution.",
                tc_name
            ),
            true,
        );
    }

    // Validate params against schema before execution
    if let Some(validation_error) = validate_tool_params(tool.as_ref(), tc_args) {
        warn!(
            "Tool '{}' param validation failed: {}",
            tc_name, validation_error
        );
        return (validation_error, true);
    }

    match registry.execute(tc_name, tc_args.clone(), ctx).await {
        Ok(result) => (result.content, result.is_error),
        Err(e) => {
            warn!("Tool '{}' failed: {}", tc_name, e);
            let msg = crate::utils::path_sanitize::sanitize_error_message(
                &format!("Tool execution failed: {}", e),
                workspace,
            );
            (msg, true)
        }
    }
}

/// Regex that matches phrases where the LLM claims to have performed an action.
/// Used to detect hallucinated actions when no tools were actually called.
static ACTION_CLAIM_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:I(?:'ve| have) (?:updated|written|created|set up|configured|saved|deleted|removed|added|modified|changed|installed|fixed|applied|edited|committed|deployed|sent|scheduled|enabled|disabled|tested|ran|executed|fetched|searched|checked|verified|completed|performed|called|started|listed|read)|I (?:updated|wrote|created|set up|configured|saved|deleted|removed|added|modified|changed|installed|fixed|applied|edited|committed|deployed|sent|scheduled|enabled|disabled|tested|ran|executed|fetched|searched|checked|verified|completed|performed|called|started|listed|read)|(?:Changes|Updates|Modifications) (?:have been|were) (?:made|applied|saved|committed)|(?:File|Config|Settings?) (?:has been|was) (?:updated|written|created|modified|saved|deleted)|All (?:tools?|tests?|checks?) (?:are |were |have been )?(?:fully )?(?:working|functional|successful|passing|passed|completed)|(?:Successfully|Already) (?:tested|executed|completed|verified|fetched|ran|performed|called|created|updated|sent|deleted))\b"
    )
    .expect("Invalid action claim regex")
});

/// Returns `true` if the text contains phrases claiming actions were performed.
pub fn contains_action_claims(text: &str) -> bool {
    ACTION_CLAIM_RE.is_match(text)
}

/// Regex that matches phrases where the LLM falsely claims it has no tools.
static FALSE_NO_TOOLS_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:I (?:don't|do not|cannot|can't) have (?:access to )?(?:any )?tools|(?:no tools|tools (?:are|aren't) (?:not )?available)|I(?:'m| am) (?:not able|unable) to (?:use|access|call) tools)"
    )
    .expect("Invalid false-no-tools regex")
});

/// Returns `true` if the text falsely claims tools are unavailable.
pub fn is_false_no_tools_claim(text: &str) -> bool {
    FALSE_NO_TOOLS_RE.is_match(text)
}

/// Returns `true` if the text mentions 3+ tool names, suggesting hallucinated tool results.
/// When the LLM lists tool names with "results" but never actually called them, this catches
/// the pattern that the action-claim regex might miss.
pub fn mentions_multiple_tools(text: &str, tool_names: &[String]) -> bool {
    let text_lower = text.to_lowercase();
    let count = tool_names
        .iter()
        .filter(|name| text_lower.contains(name.as_str()))
        .count();
    count >= TOOL_MENTION_HALLUCINATION_THRESHOLD
}

const MAX_IMAGE_SIZE: usize = 20 * 1024 * 1024; // 20MB (Anthropic limit)
const MAX_IMAGES: usize = 5;

/// Load image files from disk and base64-encode them for LLM consumption.
/// Skips files that are missing, too large, or have unsupported formats.
fn load_and_encode_images(media_paths: &[String]) -> Vec<ImageData> {
    use base64::Engine;

    let mut images = Vec::new();
    for path in media_paths.iter().take(MAX_IMAGES) {
        let file_path = std::path::Path::new(path);
        if !file_path.exists() {
            warn!("Image file not found: {}", path);
            continue;
        }
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let media_type = match ext {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            _ => {
                warn!("Unsupported image format: {}", ext);
                continue;
            }
        };
        match std::fs::read(file_path) {
            Ok(data) => {
                if data.len() > MAX_IMAGE_SIZE {
                    warn!(
                        "Image too large ({} bytes, max {}): {}",
                        data.len(),
                        MAX_IMAGE_SIZE,
                        path
                    );
                    continue;
                }
                // Validate magic bytes match claimed format
                let valid = match ext {
                    "png" => data.starts_with(&[0x89, 0x50, 0x4E, 0x47]),
                    "jpg" | "jpeg" => data.starts_with(&[0xFF, 0xD8, 0xFF]),
                    "gif" => data.starts_with(b"GIF8"),
                    "webp" => {
                        data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP"
                    }
                    _ => false,
                };
                if !valid {
                    warn!(
                        "Image file {} has invalid magic bytes for format '{}' (first bytes: {:02x?}). File may be corrupted or not an image.",
                        path,
                        ext,
                        &data[..8.min(data.len())]
                    );
                    continue;
                }
                let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
                info!(
                    "Encoded image for LLM: {} ({}, {} raw bytes, {} base64 chars)",
                    path,
                    media_type,
                    data.len(),
                    encoded.len()
                );
                images.push(ImageData {
                    media_type: media_type.to_string(),
                    data: encoded,
                });
            }
            Err(e) => {
                warn!("Failed to read image file {}: {}", path, e);
            }
        }
    }
    images
}

/// Replace `[prefix /path/to/file]` tags in content with an optional replacement string.
/// If `replacement` is `None`, the tags are removed entirely.
fn replace_bracketed_tags(content: &str, prefix: &str, replacement: Option<&str>) -> String {
    let mut result = String::with_capacity(content.len());
    let mut remaining = content;
    while let Some(start) = remaining.find(prefix) {
        result.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find(']') {
            if let Some(rep) = replacement {
                result.push_str(rep);
            }
            remaining = &remaining[start + end + 1..];
        } else {
            // No closing bracket — keep the rest as-is
            remaining = &remaining[start..];
            break;
        }
    }
    result.push_str(remaining);
    result.trim().to_string()
}

/// Strip `[image: /path/to/file]` tags from message content.
/// These tags are added by channels when images are downloaded, but become
/// redundant (and misleading) once images are base64-encoded into content blocks.
fn strip_image_tags(content: &str) -> String {
    replace_bracketed_tags(content, "[image: ", None)
}

/// Replace `[audio: /path/to/file]` tags with a notice when transcription is not configured.
/// This ensures the LLM knows a voice message was sent even without transcription.
fn strip_audio_tags(content: &str) -> String {
    replace_bracketed_tags(
        content,
        "[audio: ",
        Some("[Voice message received, but transcription is not configured]"),
    )
}

/// Replace `[audio: /path/to/file]` tags with transcribed text.
async fn transcribe_audio_tags(
    content: &str,
    transcriber: &crate::utils::transcription::TranscriptionService,
) -> String {
    use std::fmt::Write;

    let mut result = String::with_capacity(content.len());
    let mut remaining = content;
    while let Some(start) = remaining.find(AUDIO_TAG_PREFIX) {
        result.push_str(&remaining[..start]);
        let after_tag = &remaining[start + AUDIO_TAG_PREFIX.len()..];
        if let Some(end) = after_tag.find(']') {
            let path_str = &after_tag[..end];
            let path = std::path::Path::new(path_str);
            match transcriber.transcribe(path).await {
                Ok(text) if !text.is_empty() => {
                    info!("transcribed audio: {} -> {} chars", path_str, text.len());
                    let _ = write!(result, "[Voice message: \"{}\"]", text);
                }
                Ok(_) => {
                    warn!("empty transcription for {}", path_str);
                    result.push_str("[Voice message: transcription empty]");
                }
                Err(e) => {
                    warn!("transcription failed for {}: {}", path_str, e);
                    result.push_str("[Voice message: transcription failed]");
                }
            }
            remaining = &after_tag[end + 1..];
        } else {
            remaining = &remaining[start..];
            break;
        }
    }
    result.push_str(remaining);
    result
}

/// Delete media files older than the given TTL (in days).
fn cleanup_old_media(ttl_days: u32) -> Result<()> {
    let media_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
        .join(".oxicrab")
        .join("media");
    if !media_dir.exists() {
        return Ok(());
    }
    let cutoff =
        std::time::SystemTime::now() - std::time::Duration::from_secs(u64::from(ttl_days) * 86400);
    let mut removed = 0u32;
    for entry in std::fs::read_dir(&media_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && let Ok(metadata) = std::fs::metadata(&path)
            && let Ok(modified) = metadata.modified()
            && modified < cutoff
            && std::fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    if removed > 0 {
        info!("Cleaned up {} old media files", removed);
    }
    Ok(())
}

/// Periodic typing indicator: sends every 4s until the returned handle is aborted.
fn start_typing(
    typing_tx: Option<&Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
    ctx: Option<&(String, String)>,
) -> Option<tokio::task::JoinHandle<()>> {
    if let (Some(tx), Some(ctx)) = (typing_tx, ctx) {
        let tx = tx.clone();
        let ctx = ctx.clone();
        Some(tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(TYPING_INDICATOR_INTERVAL_SECS));
            loop {
                interval.tick().await;
                if tx.send(ctx.clone()).await.is_err() {
                    break;
                }
            }
        }))
    } else {
        None
    }
}

/// Configuration for creating an [`AgentLoop`] instance.
pub struct AgentLoopConfig {
    pub bus: Arc<Mutex<MessageBus>>,
    pub provider: Arc<dyn LLMProvider>,
    pub workspace: PathBuf,
    pub model: Option<String>,
    pub max_iterations: usize,
    pub brave_api_key: Option<String>,
    pub web_search_config: Option<crate::config::WebSearchConfig>,
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
    pub media_config: Option<crate::config::MediaConfig>,
    pub obsidian_config: Option<crate::config::ObsidianConfig>,
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
    /// Channel configurations for multi-channel cron target resolution
    pub channels_config: Option<crate::config::ChannelsConfig>,
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
    /// Browser tool configuration
    pub browser_config: Option<crate::config::BrowserConfig>,
    /// Image generation tool configuration
    pub image_gen_config: Option<crate::config::ImageGenConfig>,
    /// MCP (Model Context Protocol) server configuration
    pub mcp_config: Option<crate::config::McpConfig>,
    /// Cost guard configuration for budget and rate limiting
    pub cost_guard_config: crate::config::CostGuardConfig,
    /// Cognitive routines configuration for checkpoint pressure signals
    pub cognitive_config: crate::config::CognitiveConfig,
    /// Exfiltration guard configuration for hiding outbound tools from LLM
    pub exfiltration_guard: crate::config::ExfiltrationGuardConfig,
    /// Prompt injection detection configuration
    pub prompt_guard_config: crate::config::PromptGuardConfig,
    /// Landlock sandbox configuration for shell commands
    pub sandbox_config: crate::config::SandboxConfig,
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
            brave_api_key: Some(config.tools.web.search.api_key.clone()),
            web_search_config: Some(config.tools.web.search.clone()),
            exec_timeout: config.tools.exec.timeout,
            restrict_to_workspace: config.tools.restrict_to_workspace,
            allowed_commands: config.tools.exec.allowed_commands.clone(),
            compaction_config: config.agents.defaults.compaction.clone(),
            outbound_tx: params.outbound_tx,
            cron_service: params.cron_service,
            google_config: Some(config.tools.google.clone()),
            github_config: Some(config.tools.github.clone()),
            weather_config: Some(config.tools.weather.clone()),
            todoist_config: Some(config.tools.todoist.clone()),
            media_config: Some(config.tools.media.clone()),
            obsidian_config: Some(config.tools.obsidian.clone()),
            temperature: config.agents.defaults.temperature,
            tool_temperature: TOOL_TEMPERATURE,
            session_ttl_days: config.agents.defaults.session_ttl_days,
            max_tokens: config.agents.defaults.max_tokens,
            typing_tx: params.typing_tx,
            channels_config: params.channels_config,
            memory_indexer_interval: config.agents.defaults.memory_indexer_interval,
            media_ttl_days: config.agents.defaults.media_ttl_days,
            max_concurrent_subagents: config.agents.defaults.max_concurrent_subagents,
            voice_config: Some(config.voice.clone()),
            memory_config: Some(config.agents.defaults.memory.clone()),
            browser_config: Some(config.tools.browser.clone()),
            image_gen_config: Some(image_gen),
            mcp_config: Some(config.tools.mcp.clone()),
            cost_guard_config: config.agents.defaults.cost_guard.clone(),
            cognitive_config: config.agents.defaults.cognitive.clone(),
            exfiltration_guard: config.tools.exfiltration_guard.clone(),
            prompt_guard_config: config.agents.defaults.prompt_guard.clone(),
            sandbox_config: config.tools.exec.sandbox.clone(),
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
            brave_api_key: None,
            web_search_config: None,
            exec_timeout: 30,
            restrict_to_workspace: true,
            allowed_commands: vec![],
            compaction_config: crate::config::CompactionConfig {
                enabled: false,
                threshold_tokens: 40000,
                keep_recent: 10,
                extraction_enabled: false,
                model: None,
                checkpoint: crate::config::CheckpointConfig::default(),
            },
            outbound_tx,
            cron_service: None,
            google_config: None,
            github_config: None,
            weather_config: None,
            todoist_config: None,
            media_config: None,
            obsidian_config: None,
            temperature: 0.7,
            tool_temperature: 0.0,
            session_ttl_days: 0,
            max_tokens: 8192,
            typing_tx: None,
            channels_config: None,
            memory_indexer_interval: 300,
            media_ttl_days: 0,
            max_concurrent_subagents: 5,
            voice_config: None,
            memory_config: None,
            browser_config: None,
            image_gen_config: None,
            mcp_config: None,
            cost_guard_config: crate::config::CostGuardConfig::default(),
            cognitive_config: crate::config::CognitiveConfig::default(),
            exfiltration_guard: crate::config::ExfiltrationGuardConfig::default(),
            prompt_guard_config: crate::config::PromptGuardConfig::default(),
            sandbox_config: crate::config::SandboxConfig {
                enabled: false,
                ..crate::config::SandboxConfig::default()
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
    transcriber: Option<Arc<crate::utils::transcription::TranscriptionService>>,
    event_matcher: Option<std::sync::Mutex<EventMatcher>>,
    /// Last time the event matcher was rebuilt from disk
    event_matcher_last_rebuild: Arc<std::sync::Mutex<std::time::Instant>>,
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
            brave_api_key,
            web_search_config,
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
            media_config,
            obsidian_config,
            temperature,
            tool_temperature,
            session_ttl_days,
            max_tokens,
            typing_tx,
            channels_config,
            memory_indexer_interval,
            media_ttl_days,
            max_concurrent_subagents,
            voice_config,
            memory_config,
            browser_config,
            image_gen_config,
            mcp_config,
            cost_guard_config,
            cognitive_config,
            exfiltration_guard,
            prompt_guard_config,
            sandbox_config,
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
            restrict_to_workspace,
            exec_timeout,
            outbound_tx: outbound_tx.clone(),
            bus: bus.clone(),
            web_search_config,
            cron_service: cron_service.clone(),
            channels_config,
            google_config,
            github_config,
            weather_config,
            todoist_config,
            media_config,
            obsidian_config,
            browser_config,
            image_gen_config,
            memory: memory.clone(),
            subagent_config: SubagentConfig {
                provider: provider.clone(),
                workspace: workspace.clone(),
                model: Some(model.clone()),
                brave_api_key: brave_api_key.clone(),
                exec_timeout,
                restrict_to_workspace,
                allowed_commands: allowed_commands.clone(),
                max_tokens,
                tool_temperature,
                max_concurrent: max_concurrent_subagents,
                exfil_blocked_tools: if exfiltration_guard.enabled {
                    exfiltration_guard.blocked_tools.clone()
                } else {
                    vec![]
                },
                cost_guard: cost_guard.clone(),
                prompt_guard_config: prompt_guard_config.clone(),
                sandbox_config: sandbox_config.clone(),
            },
            brave_api_key,
            allowed_commands,
            mcp_config,
            sandbox_config,
        };

        let (tools, subagents, mcp_manager) =
            crate::agent::tools::setup::register_all_tools(&tool_ctx).await?;
        let tools = Arc::new(tools);

        let transcriber = voice_config
            .as_ref()
            .and_then(|vc| {
                crate::utils::transcription::TranscriptionService::new(&vc.transcription)
            })
            .map(Arc::new);

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
            event_matcher_last_rebuild: Arc::new(std::sync::Mutex::new(std::time::Instant::now())),
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
        info!("Agent loop started, waiting for messages...");
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
            let needs_rebuild = self
                .event_matcher_last_rebuild
                .lock()
                .is_ok_and(|t| t.elapsed().as_secs() >= 60);
            if needs_rebuild && let Ok(store) = cron_svc.load_store(true).await {
                let new_matcher = EventMatcher::from_jobs(&store.jobs);
                if let Some(ref matcher_mutex) = self.event_matcher
                    && let Ok(mut guard) = matcher_mutex.lock()
                {
                    *guard = new_matcher;
                }
                if let Ok(mut t) = self.event_matcher_last_rebuild.lock() {
                    *t = std::time::Instant::now();
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
        let exec_ctx = Self::build_execution_context(&msg.channel, &msg.chat_id, context_summary);

        debug!("Getting compacted history");
        let history = self.get_compacted_history(&session).await?;
        debug!("Got {} history messages", history.len());

        // Transcribe any audio files before other processing
        let msg_content = if let Some(ref transcriber) = self.transcriber {
            transcribe_audio_tags(&msg.content, transcriber).await
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

        // Strip [image: ...] tags from content when images were successfully encoded,
        // since the LLM receives them as content blocks and doesn't need the file paths
        // (which can cause it to try read_file on binary image data).
        let content = if images.is_empty() {
            msg_content
        } else {
            strip_image_tags(&msg_content)
        };

        debug!("Acquiring context lock");
        let messages = {
            let mut ctx = self.context.lock().await;
            ctx.build_messages(
                &history,
                &content,
                Some(&msg.channel),
                Some(&msg.chat_id),
                Some(&msg.sender_id),
                images,
            )?
        };
        debug!("Built {} messages, starting agent loop", messages.len());

        let typing_ctx = Some((msg.channel.clone(), msg.chat_id.clone()));
        let (final_content, input_tokens, tools_used, collected_media) =
            self.run_agent_loop(messages, typing_ctx, &exec_ctx).await?;

        // Reload session in case compaction updated it during the agent loop
        // (compaction saves a compaction_summary to session metadata)
        let mut session = self.sessions.get_or_create(&session_key).await?;
        let extra = HashMap::new();
        session.add_message("user".to_string(), msg.content.clone(), extra.clone());
        if let Some(ref content) = final_content {
            let mut assistant_extra = HashMap::new();
            if !tools_used.is_empty() {
                assistant_extra.insert(
                    "tools_used".to_string(),
                    Value::Array(tools_used.into_iter().map(Value::String).collect()),
                );
            }
            session.add_message("assistant".to_string(), content.clone(), assistant_extra);
        }
        // Store provider-reported input tokens for precise compaction threshold checks
        if let Some(tokens) = input_tokens {
            session.metadata.insert(
                "last_input_tokens".to_string(),
                Value::Number(serde_json::Number::from(tokens)),
            );
        }
        self.sessions.save(&session).await?;

        // Background fact extraction
        if let (Some(compactor), Some(content)) = (&self.compactor, &final_content)
            && self.compaction_config.extraction_enabled
            && msg.channel != "system"
        {
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
            Ok(None)
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
    ) -> Result<(Option<String>, Option<u64>, Vec<String>, Vec<String>)> {
        self.run_agent_loop_with_overrides(
            messages,
            typing_context,
            exec_ctx,
            &AgentRunOverrides::default(),
        )
        .await
    }

    /// Core agent loop implementation with per-invocation overrides.
    ///
    /// Iterates up to `max_iterations` rounds of: LLM call → parallel tool execution → append results.
    /// First iteration forces `tool_choice="any"` to prevent text-only hallucinations. At 70% of
    /// max iterations, a wrap-up nudge is injected.
    ///
    /// Returns `(response_text, last_message_id, collected_media, tool_names_used)`.
    async fn run_agent_loop_with_overrides(
        &self,
        mut messages: Vec<Message>,
        typing_context: Option<(String, String)>,
        exec_ctx: &ExecutionContext,
        overrides: &AgentRunOverrides,
    ) -> Result<(Option<String>, Option<u64>, Vec<String>, Vec<String>)> {
        let effective_model = overrides.model.as_deref().unwrap_or(&self.model);
        let effective_max_iterations = overrides.max_iterations.unwrap_or(self.max_iterations);
        let mut empty_retries_left = EMPTY_RESPONSE_RETRIES;
        let mut any_tools_called = false;
        let mut correction_sent = false;
        let mut last_input_tokens: Option<u64> = None;
        let mut tools_used: Vec<String> = Vec::new();
        let mut collected_media: Vec<String> = Vec::new();
        let mut checkpoint_tracker = CheckpointTracker::new(self.cognitive_config.clone());

        let tools_defs = self.tools.get_tool_definitions();

        // Exfiltration guard: hide outbound-capable tools from the LLM
        let exfil_blocked: Vec<String> = if self.exfiltration_guard.enabled {
            self.exfiltration_guard.blocked_tools.clone()
        } else {
            vec![]
        };
        let tools_defs = if exfil_blocked.is_empty() {
            tools_defs
        } else {
            tools_defs
                .into_iter()
                .filter(|td| !exfil_blocked.contains(&td.name))
                .collect()
        };

        // Extract tool names for hallucination detection (immutable snapshot for the full loop)
        let tool_names: Vec<String> = tools_defs.iter().map(|td| td.name.clone()).collect();

        // Inject tool facts reminder so the LLM knows exactly what tools are available
        if !tool_names.is_empty() {
            let tool_list = tool_names.join(", ");
            let tool_facts = format!(
                "## Available Tools\n\nYou have access to the following tools: {}\n\n\
                 If a user asks for external actions, do not claim tools are unavailable — \
                 call the matching tool directly.",
                tool_list
            );
            // Insert as second message (after system prompt, before history/user message)
            if messages.len() > 1 {
                messages.insert(1, Message::user(tool_facts));
            } else {
                messages.push(Message::user(tool_facts));
            }
        }

        // Inject static cognitive routines instructions when enabled
        if self.cognitive_config.enabled && messages.len() > 1 {
            messages.insert(
                2,
                Message::system(
                    "## Cognitive Routines\n\n\
                     When working on complex tasks with many tool calls:\n\
                     - Periodically summarize your progress in your responses\n\
                     - If you receive a checkpoint hint, briefly note: what's done, \
                     what's in progress, what's next\n\
                     - Keep track of your overall plan and remaining steps"
                        .to_string(),
                ),
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

            // Use retry logic for provider calls
            // Use low temperature for tool-calling iterations (determinism),
            // normal temperature for final text responses
            let current_temp = if any_tools_called {
                // During active tool iteration, use low temp for determinism
                self.tool_temperature
            } else {
                // For initial/text-only responses, use configured temperature
                self.temperature
            };
            // Force tool use on first iteration to prevent text-only hallucinated responses
            let tool_choice = if iteration == 1 && !tools_defs.is_empty() {
                Some("any".to_string())
            } else {
                None // defaults to "auto" in provider
            };

            // Cost guard pre-flight check
            if let Some(ref cg) = self.cost_guard
                && let Err(msg) = cg.check_allowed()
            {
                warn!("cost guard blocked LLM call: {}", msg);
                if let Some(h) = typing_handle {
                    h.abort();
                }
                return Ok((Some(msg), last_input_tokens, tools_used, collected_media));
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

                let results = self
                    .execute_tools(&response.tool_calls, &tool_names, exec_ctx, &exfil_blocked)
                    .await;

                // Stop typing indicator after tool execution
                if let Some(h) = typing_handle {
                    h.abort();
                }

                self.handle_tool_results(
                    &mut messages,
                    &response.tool_calls,
                    results,
                    &mut collected_media,
                    &mut checkpoint_tracker,
                    iteration,
                )
                .await;
            } else if let Some(content) = response.content {
                match Self::handle_text_response(
                    &content,
                    &mut messages,
                    response.reasoning_content.as_deref(),
                    any_tools_called,
                    &mut correction_sent,
                    &tool_names,
                ) {
                    TextAction::Continue => {}
                    TextAction::Return => {
                        return Ok((
                            Some(content),
                            last_input_tokens,
                            tools_used,
                            collected_media,
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
            ));
        }

        Ok((None, last_input_tokens, tools_used, collected_media))
    }

    /// Execute tool calls — single-tool fast-path or parallel `spawn`+`join_all`.
    async fn execute_tools(
        &self,
        tool_calls: &[ToolCallRequest],
        tool_names: &[String],
        exec_ctx: &ExecutionContext,
        exfil_blocked: &[String],
    ) -> Vec<(String, bool)> {
        if tool_calls.len() == 1 {
            let tc = &tool_calls[0];
            vec![
                execute_tool_call(
                    &self.tools,
                    &tc.name,
                    &tc.arguments,
                    tool_names,
                    exec_ctx,
                    exfil_blocked,
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
                    let blocked = exfil_blocked.to_vec();
                    let ws = self.workspace.clone();
                    tokio::task::spawn(async move {
                        execute_tool_call(
                            &registry,
                            &tc_name,
                            &tc_args,
                            &available,
                            &ctx,
                            &blocked,
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

    /// Collect media from tool results, scan for prompt injection, update
    /// cognitive tracking, and fire periodic checkpoints.
    async fn handle_tool_results(
        &self,
        messages: &mut Vec<Message>,
        tool_calls: &[ToolCallRequest],
        results: Vec<(String, bool)>,
        collected_media: &mut Vec<String>,
        checkpoint_tracker: &mut CheckpointTracker,
        iteration: usize,
    ) {
        // Add all results to messages in order and collect media
        debug_assert_eq!(
            tool_calls.len(),
            results.len(),
            "tool_calls and results length mismatch: {} vs {}",
            tool_calls.len(),
            results.len()
        );
        for (tc, (result_str, is_error)) in tool_calls.iter().zip(results.into_iter()) {
            if !is_error {
                collected_media.extend(extract_media_paths(&result_str));
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
                        map.insert("content".to_string(), Value::String(m.content.clone()));
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

    /// Handle a text-only LLM response: false no-tools correction or
    /// hallucination detection. Returns [`TextAction::Continue`] if a
    /// correction was injected, or [`TextAction::Return`] if the response is final.
    fn handle_text_response(
        content: &str,
        messages: &mut Vec<Message>,
        reasoning_content: Option<&str>,
        any_tools_called: bool,
        correction_sent: &mut bool,
        tool_names: &[String],
    ) -> TextAction {
        // Detect false "no tools" claims and retry with correction
        if !tool_names.is_empty() && is_false_no_tools_claim(content) {
            warn!(
                "False no-tools claim detected: LLM claims tools unavailable but {} tools are registered",
                tool_names.len()
            );
            ContextBuilder::add_assistant_message(messages, Some(content), None, reasoning_content);
            let tool_list = tool_names.join(", ");
            messages.push(Message::user(format!(
                "You DO have tools available. Your available tools are: {}. \
                 Please use the appropriate tool to fulfill the request.",
                tool_list
            )));
            *correction_sent = true;
            return TextAction::Continue;
        }

        // Detect hallucinated actions: LLM claims it did something but never called tools
        if !any_tools_called
            && !*correction_sent
            && (contains_action_claims(content) || mentions_multiple_tools(content, tool_names))
        {
            warn!("Action hallucination detected: LLM claims actions but no tools were called");
            ContextBuilder::add_assistant_message(messages, Some(content), None, reasoning_content);
            messages.push(Message::user(
                "You claimed to have performed actions, but you did not use any tools. \
                 Do not claim to have done something you haven't. Either use the \
                 appropriate tools to actually perform the action, or explain what \
                 you would need to do."
                    .to_string(),
            ));
            *correction_sent = true;
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
        if let Ok(response) = self
            .provider
            .chat_with_retry(
                crate::providers::base::ChatRequest {
                    messages: messages.clone(),
                    tools: None,
                    model: Some(effective_model),
                    max_tokens: self.max_tokens,
                    temperature: self.temperature,
                    tool_choice: None,
                },
                Some(crate::providers::base::RetryConfig::default()),
            )
            .await
            && let Some(content) = response.content
        {
            return Ok(Some(content));
        }

        Ok(None)
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

                    // Update session metadata with new summary
                    let session_key = session.key.clone();
                    let mut updated_session = self.sessions.get_or_create(&session_key).await?;
                    updated_session
                        .metadata
                        .insert("compaction_summary".to_string(), Value::String(summary));
                    self.sessions.save(&updated_session).await?;

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

        let messages = {
            let mut context = self.context.lock().await;
            context.build_messages(
                &history,
                &msg.content,
                Some(origin_channel.as_str()),
                Some(origin_chat_id.as_str()),
                None,
                vec![],
            )?
        };

        let typing_ctx = Some((origin_channel.clone(), origin_chat_id.clone()));
        let exec_ctx = Self::build_execution_context(&origin_channel, &origin_chat_id, None);
        let (final_content, _, tools_used, collected_media) =
            self.run_agent_loop(messages, typing_ctx, &exec_ctx).await?;
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
        let session = self.sessions.get_or_create(session_key).await?;
        let history = self.get_compacted_history(&session).await?;

        let messages = {
            let mut ctx = self.context.lock().await;
            ctx.build_messages(
                &history,
                content,
                Some(channel),
                Some(chat_id),
                None,
                vec![],
            )?
        };

        let typing_ctx = Some((channel.to_string(), chat_id.to_string()));
        let exec_ctx = Self::build_execution_context(channel, chat_id, None);
        let (response, _, tools_used, _collected_media) = self
            .run_agent_loop_with_overrides(messages, typing_ctx, &exec_ctx, overrides)
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
