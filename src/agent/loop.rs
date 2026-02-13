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
    media::MediaTool,
    memory_search::MemorySearchTool,
    message::MessageTool,
    obsidian::{ObsidianSyncService, ObsidianTool},
    reddit::RedditTool,
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
use crate::providers::base::{ImageData, LLMProvider, Message};
use crate::session::{Session, SessionManager, SessionStore};
use crate::utils::task_tracker::TaskTracker;
use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

const EMPTY_RESPONSE_RETRIES: usize = 2;
const TOOL_EXECUTION_TIMEOUT_SECS: u64 = 120;
const MAX_TOOL_RESULT_CHARS: usize = 10000;
const AGENT_POLL_INTERVAL_MS: u64 = 100;

/// Validate tool arguments against the tool's JSON schema.
/// Checks: (1) required fields are present, (2) field types match schema.
/// Returns None if valid, Some(error_message) if invalid.
pub(crate) fn validate_tool_params(
    tool: &dyn crate::agent::tools::base::Tool,
    params: &Value,
) -> Option<String> {
    let schema = tool.parameters();
    let mut errors = Vec::new();

    // Check required fields
    if let Some(required) = schema["required"].as_array() {
        for field in required {
            if let Some(field_name) = field.as_str() {
                if params.get(field_name).is_none() || params[field_name].is_null() {
                    errors.push(format!("missing required parameter '{}'", field_name));
                }
            }
        }
    }

    // Check types of provided fields
    if let Some(properties) = schema["properties"].as_object() {
        for (field_name, field_schema) in properties {
            if let Some(value) = params.get(field_name) {
                if !value.is_null() {
                    if let Some(expected_type) = field_schema["type"].as_str() {
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

/// Core logic for executing a single tool call and producing (result_string, is_error).
async fn execute_tool_call_inner(
    _tc_id: &str,
    tc_name: &str,
    tc_args: &Value,
    tool_opt: Option<Arc<dyn crate::agent::tools::base::Tool>>,
    available_tools: &[String],
) -> (String, bool) {
    if let Some(tool) = tool_opt {
        // Validate params against schema before execution
        if let Some(validation_error) = validate_tool_params(tool.as_ref(), tc_args) {
            warn!(
                "Tool '{}' param validation failed: {}",
                tc_name, validation_error
            );
            return (validation_error, true);
        }

        debug!("Executing tool: {} with arguments: {}", tc_name, tc_args);
        let tool_name = tc_name.to_string();
        let params = tc_args.clone();
        let timeout_duration = std::time::Duration::from_secs(TOOL_EXECUTION_TIMEOUT_SECS);
        match tokio::time::timeout(timeout_duration, tool.execute(params)).await {
            Ok(Ok(result)) => {
                if result.is_error {
                    warn!("Tool '{}' returned error: {}", tool_name, result.content);
                }
                (
                    truncate_tool_result(&result.content, MAX_TOOL_RESULT_CHARS),
                    result.is_error,
                )
            }
            Ok(Err(e)) => {
                warn!("Tool '{}' failed: {}", tool_name, e);
                (format!("Tool execution failed: {}", e), true)
            }
            Err(_) => {
                warn!(
                    "Tool '{}' timed out after {}s",
                    tool_name,
                    timeout_duration.as_secs()
                );
                (
                    format!(
                        "Tool '{}' timed out after {}s",
                        tool_name,
                        timeout_duration.as_secs()
                    ),
                    true,
                )
            }
        }
    } else {
        warn!("LLM called unknown tool: {}", tc_name);
        (
            format!(
                "Error: tool '{}' does not exist. Available tools: {}",
                tc_name,
                available_tools.join(", ")
            ),
            true,
        )
    }
}

/// Execute a tool call with panic isolation (single-tool fast-path).
async fn execute_tool_call(
    tc: &crate::providers::base::ToolCallRequest,
    tool_opt: Option<Arc<dyn crate::agent::tools::base::Tool>>,
    available_tools: Vec<String>,
) -> (String, bool) {
    let tc_id = tc.id.clone();
    let tc_name = tc.name.clone();
    let tc_args = tc.arguments.clone();
    let handle = tokio::task::spawn(async move {
        execute_tool_call_inner(&tc_id, &tc_name, &tc_args, tool_opt, &available_tools).await
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
    count >= 3
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

/// Strip `[image: /path/to/file]` tags from message content.
/// These tags are added by channels when images are downloaded, but become
/// redundant (and misleading) once images are base64-encoded into content blocks.
fn strip_image_tags(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut remaining = content;
    while let Some(start) = remaining.find("[image: ") {
        result.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find(']') {
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

/// Delete media files older than the given TTL (in days).
fn cleanup_old_media(ttl_days: u32) -> Result<()> {
    let media_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
        .join(".nanobot")
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
        if path.is_file() {
            if let Ok(metadata) = std::fs::metadata(&path) {
                if let Ok(modified) = metadata.modified() {
                    if modified < cutoff && std::fs::remove_file(&path).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
    }
    if removed > 0 {
        info!("Cleaned up {} old media files", removed);
    }
    Ok(())
}

/// Periodic typing indicator: sends every 4s until the returned handle is aborted.
fn start_typing(
    typing_tx: &Option<Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
    ctx: &Option<(String, String)>,
) -> Option<tokio::task::JoinHandle<()>> {
    if let (Some(tx), Some(ctx)) = (typing_tx, ctx) {
        let tx = tx.clone();
        let ctx = ctx.clone();
        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(4));
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

/// Format a human-readable status line for a tool call about to execute.
fn format_tool_status(name: &str, args: &serde_json::Value) -> String {
    match name {
        "web_search" => {
            let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("...");
            format!("\u{1f50d} Searching: {}", q)
        }
        "web_fetch" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("...");
            let domain = url.split('/').nth(2).unwrap_or(url);
            format!("\u{1f310} Fetching: {}", domain)
        }
        "obsidian" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("...");
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            format!("\u{1f4d3} Obsidian {}: {}", action, path)
        }
        "exec" => format!("\u{2699}\u{fe0f} Running command"),
        "read_file" | "write_file" | "edit_file" => {
            let p = args.get("path").and_then(|v| v.as_str()).unwrap_or("...");
            format!("\u{1f4c1} {}: {}", name, p)
        }
        _ => format!("\u{1f527} {}", name),
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
    /// Sender for typing indicator events (channel, chat_id)
    pub typing_tx: Option<Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
    /// Channel configurations for multi-channel cron target resolution
    pub channels_config: Option<crate::config::ChannelsConfig>,
    /// Memory indexer interval in seconds (default 300)
    pub memory_indexer_interval: u64,
    /// Media file TTL in days for cleanup (default 7)
    pub media_ttl_days: u32,
    /// Maximum concurrent subagents (default 5)
    pub max_concurrent_subagents: usize,
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
    max_tokens: u32,
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

        // Clean up old media files in background
        if media_ttl_days > 0 {
            let ttl = media_ttl_days;
            tokio::spawn(async move {
                if let Err(e) = cleanup_old_media(ttl) {
                    tracing::warn!("Media cleanup failed: {}", e);
                }
            });
        }

        let sessions: Arc<dyn SessionStore> = Arc::new(session_mgr);
        let memory = Arc::new(MemoryStore::with_indexer_interval(
            &workspace,
            memory_indexer_interval,
        )?);
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
        if let Some(ref ws_cfg) = web_search_config {
            tools.register(Arc::new(WebSearchTool::from_config(ws_cfg)));
        } else {
            tools.register(Arc::new(WebSearchTool::new(brave_api_key.clone(), 5)));
        }
        tools.register(Arc::new(WebFetchTool::new(50000)?));

        // Register message tool with outbound sender
        let outbound_tx_for_tool = outbound_tx.clone();
        tools.register(Arc::new(MessageTool::new(Some(outbound_tx_for_tool))));

        // Create subagent manager
        let subagents = Arc::new(SubagentManager::new(
            SubagentConfig {
                provider: provider.clone(),
                workspace: workspace.clone(),
                model: Some(model.clone()),
                brave_api_key: brave_api_key.clone(),
                exec_timeout,
                restrict_to_workspace,
                allowed_commands,
                max_tokens,
                tool_temperature,
                max_concurrent: max_concurrent_subagents,
            },
            bus.clone(),
        ));

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

        // Register Media tool (Radarr/Sonarr) if configured
        if let Some(ref media_cfg) = media_config {
            if media_cfg.enabled {
                tools.register(Arc::new(MediaTool::new(media_cfg)));
                info!("Media tool registered (Radarr/Sonarr)");
            }
        }

        // Register Obsidian tool if configured
        if let Some(ref obsidian_cfg) = obsidian_config {
            if obsidian_cfg.enabled
                && !obsidian_cfg.api_url.is_empty()
                && !obsidian_cfg.api_key.is_empty()
            {
                match ObsidianTool::new(
                    &obsidian_cfg.api_url,
                    &obsidian_cfg.api_key,
                    &obsidian_cfg.vault_name,
                    obsidian_cfg.timeout,
                )
                .await
                {
                    Ok((tool, cache)) => {
                        tools.register(Arc::new(tool));
                        let sync_svc = ObsidianSyncService::new(cache, obsidian_cfg.sync_interval);
                        tokio::spawn(async move {
                            if let Err(e) = sync_svc.start().await {
                                tracing::error!("Obsidian sync failed to start: {}", e);
                            }
                        });
                        info!("Obsidian tool registered");
                    }
                    Err(e) => {
                        warn!("Obsidian tool not available: {}", e);
                    }
                }
            }
        }

        // Register HTTP tool (always available, no config needed)
        tools.register(Arc::new(HttpTool::new()));

        // Register Reddit tool (always available, no auth needed)
        tools.register(Arc::new(RedditTool::new()));

        // Register memory search tool (always available)
        tools.register(Arc::new(MemorySearchTool::new(memory.clone())));

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
            max_tokens,
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
                        debug!(
                            "No outbound message needed (content delivered via tool or suppressed)"
                        );
                    }
                    Err(e) => {
                        error!("Error processing message: {}", e);
                    }
                }
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(AGENT_POLL_INTERVAL_MS))
                    .await;
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

        let session_key = msg.session_key();
        // Reuse session to avoid repeated lookups
        debug!("Loading session: {}", session_key);
        let mut session = self.sessions.get_or_create(&session_key).await?;

        // Set tool contexts (pass compaction summary for subagent context injection)
        let context_summary = session
            .metadata
            .get("compaction_summary")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        debug!("Setting tool contexts");
        self.set_tool_contexts(&msg.channel, &msg.chat_id, context_summary.as_deref())
            .await;

        debug!("Getting compacted history");
        let history = self.get_compacted_history(&session).await?;
        debug!("Got {} history messages", history.len());

        // Load and encode any attached images
        let images = if !msg.media.is_empty() {
            info!(
                "Loading {} media files for LLM: {:?}",
                msg.media.len(),
                msg.media
            );
            let imgs = load_and_encode_images(&msg.media);
            info!("Encoded {} images for LLM", imgs.len());
            imgs
        } else {
            vec![]
        };

        // Strip [image: ...] tags from content when images were successfully encoded,
        // since the LLM receives them as content blocks and doesn't need the file paths
        // (which can cause it to try read_file on binary image data).
        let content = if !images.is_empty() {
            strip_image_tags(&msg.content)
        } else {
            msg.content.clone()
        };

        debug!("Acquiring context lock");
        let messages = {
            let mut context = self.context.lock().await;
            context.build_messages(
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
        let (final_content, input_tokens) = self.run_agent_loop(messages, typing_ctx).await?;

        // Save conversation (reuse session variable)
        let extra = HashMap::new();
        session.add_message("user".to_string(), msg.content.clone(), extra.clone());
        if let Some(ref content) = final_content {
            session.add_message("assistant".to_string(), content.clone(), extra);
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
                media: vec![],
                metadata: HashMap::new(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Returns `(final_content, input_tokens)`. `input_tokens` is the
    /// provider-reported input token count from the most recent LLM call (if available).
    async fn run_agent_loop(
        &self,
        mut messages: Vec<Message>,
        typing_context: Option<(String, String)>,
    ) -> Result<(Option<String>, Option<u64>)> {
        let mut empty_retries_left = EMPTY_RESPONSE_RETRIES;
        let mut last_used_delivery_tool = false;
        let mut any_tools_called = false;
        let mut last_input_tokens: Option<u64> = None;

        // Cache tool definitions to avoid repeated lock acquisition
        let tools_defs = {
            let tools_guard = self.tools.lock().await;
            tools_guard.get_tool_definitions()
        };

        // Extract tool names for hallucination detection
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

        for iteration in 1..=self.max_iterations {
            // Start periodic typing indicator before LLM call
            let typing_handle = start_typing(&self.typing_tx, &typing_context);

            // Use retry logic for provider calls
            // Use low temperature for tool-calling iterations (determinism),
            // normal temperature for final text responses
            let current_temp = if tools_defs.is_empty() {
                self.temperature
            } else {
                self.tool_temperature
            };
            // Force tool use on first iteration to prevent text-only hallucinated responses
            let tool_choice = if iteration == 1 && !tools_defs.is_empty() {
                Some("any".to_string())
            } else {
                None // defaults to "auto" in provider
            };

            let response = self
                .provider
                .chat_with_retry(
                    crate::providers::base::ChatRequest {
                        messages: messages.clone(),
                        tools: Some(tools_defs.clone()),
                        model: Some(&self.model),
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

            if response.has_tool_calls() {
                any_tools_called = true;
                let called_tool_names: Vec<&str> = response
                    .tool_calls
                    .iter()
                    .map(|tc| tc.name.as_str())
                    .collect();
                last_used_delivery_tool = called_tool_names
                    .iter()
                    .any(|n| *n == "message" || *n == "spawn");

                ContextBuilder::add_assistant_message(
                    &mut messages,
                    response.content.as_deref(),
                    Some(response.tool_calls.clone()),
                    response.reasoning_content.as_deref(),
                );

                // Send status update showing which tools are about to run
                let status_parts: Vec<String> = response
                    .tool_calls
                    .iter()
                    .map(|tc| format_tool_status(&tc.name, &tc.arguments))
                    .collect();
                let status_msg = status_parts.join("\n");

                if let Some(ref ctx) = typing_context {
                    let _ = self
                        .outbound_tx
                        .send(OutboundMessage {
                            channel: ctx.0.clone(),
                            chat_id: ctx.1.clone(),
                            content: status_msg,
                            reply_to: None,
                            media: vec![],
                            metadata: HashMap::from([(
                                "status".to_string(),
                                serde_json::Value::Bool(true),
                            )]),
                        })
                        .await;
                }

                // Start periodic typing indicator before tool execution
                let typing_handle = start_typing(&self.typing_tx, &typing_context);

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
                    vec![execute_tool_call(tc, tool_opt.clone(), tool_names.clone()).await]
                } else {
                    // Parallel execution: spawn all, await all
                    let handles: Vec<_> = tool_lookups
                        .iter()
                        .map(|(tc, tool_opt)| {
                            let tc_id = tc.id.clone();
                            let tc_name = tc.name.clone();
                            let tc_args = tc.arguments.clone();
                            let tool_opt = tool_opt.clone();
                            let available = tool_names.clone();
                            tokio::task::spawn(async move {
                                execute_tool_call_inner(
                                    &tc_id, &tc_name, &tc_args, tool_opt, &available,
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
                };

                // Stop typing indicator after tool execution
                if let Some(h) = typing_handle {
                    h.abort();
                }

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

                // Inject reflection prompt to guide next action
                messages.push(Message::user(
                    "Review the results and continue. Use more tools if needed, or provide your final response to the user.".to_string()
                ));

                // Send composing indicator so the user sees progress during LLM thinking
                if let Some(ref ctx) = typing_context {
                    let _ = self
                        .outbound_tx
                        .send(OutboundMessage {
                            channel: ctx.0.clone(),
                            chat_id: ctx.1.clone(),
                            content: "\u{270d}\u{fe0f} Composing response...".to_string(),
                            reply_to: None,
                            media: vec![],
                            metadata: HashMap::from([(
                                "status".to_string(),
                                serde_json::Value::Bool(true),
                            )]),
                        })
                        .await;
                }
            } else if let Some(content) = response.content {
                // Detect false "no tools" claims and retry with correction
                if !tool_names.is_empty() && is_false_no_tools_claim(&content) {
                    warn!("False no-tools claim detected: LLM claims tools unavailable but {} tools are registered", tool_names.len());
                    ContextBuilder::add_assistant_message(
                        &mut messages,
                        Some(&content),
                        None,
                        response.reasoning_content.as_deref(),
                    );
                    let tool_list = tool_names.join(", ");
                    messages.push(Message::user(format!(
                        "You DO have tools available. Your available tools are: {}. \
                         Please use the appropriate tool to fulfill the request.",
                        tool_list
                    )));
                    any_tools_called = true; // Prevent infinite correction loop
                    continue;
                }

                // Detect hallucinated actions: LLM claims it did something but never called tools
                if !any_tools_called
                    && (contains_action_claims(&content)
                        || mentions_multiple_tools(&content, &tool_names))
                {
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
                return Ok((Some(content), last_input_tokens));
            } else {
                // Empty response
                if last_used_delivery_tool {
                    debug!("LLM returned empty after delivery tool — treating as successful completion");
                    return Ok((None, last_input_tokens));
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

        // If tools were called but the loop ended without final content,
        // make one more LLM call with no tools to force a text summary.
        if any_tools_called {
            messages.push(Message::user(
                "Provide a brief summary of what you accomplished for the user.".to_string(),
            ));
            if let Ok(response) = self
                .provider
                .chat_with_retry(
                    crate::providers::base::ChatRequest {
                        messages: messages.clone(),
                        tools: None,
                        model: Some(&self.model),
                        max_tokens: self.max_tokens,
                        temperature: self.temperature,
                        tool_choice: None,
                    },
                    Some(crate::providers::base::RetryConfig::default()),
                )
                .await
            {
                if let Some(content) = response.content {
                    return Ok((Some(content), last_input_tokens));
                }
            }
        }

        Ok((None, last_input_tokens))
    }

    async fn set_tool_contexts(&self, channel: &str, chat_id: &str, context_summary: Option<&str>) {
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
            if let Some(summary) = context_summary {
                spawn_tool.set_context_summary(summary).await;
            }
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
        let threshold = self.compaction_config.threshold_tokens as u64;

        // Prefer provider-reported input tokens (precise), fall back to heuristic
        let token_est = session
            .metadata
            .get("last_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| estimate_messages_tokens(&full_history) as u64);

        if token_est < threshold {
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
            None,
            vec![],
        )?;

        let typing_ctx = Some((origin_channel.clone(), origin_chat_id.clone()));
        let (final_content, _) = self.run_agent_loop(messages, typing_ctx).await?;
        let final_content =
            final_content.unwrap_or_else(|| "Background task completed.".to_string());

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
        let (response, _) = self.run_agent_loop(messages, typing_ctx).await?;
        let response = response.unwrap_or_else(|| "No response generated.".to_string());

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

    // --- Expanded hallucination detection tests ---

    #[test]
    fn test_detects_i_tested() {
        assert!(contains_action_claims("I tested all the tools."));
    }

    #[test]
    fn test_detects_ive_executed() {
        assert!(contains_action_claims("I've executed the commands."));
    }

    #[test]
    fn test_detects_ive_fetched() {
        assert!(contains_action_claims("I've fetched the latest data."));
    }

    #[test]
    fn test_detects_i_verified() {
        assert!(contains_action_claims("I verified all the results."));
    }

    #[test]
    fn test_detects_i_searched() {
        assert!(contains_action_claims("I searched for the information."));
    }

    #[test]
    fn test_detects_i_listed() {
        assert!(contains_action_claims(
            "I listed all the directory contents."
        ));
    }

    #[test]
    fn test_detects_all_tools_working() {
        assert!(contains_action_claims("All Tools Working:"));
    }

    #[test]
    fn test_detects_all_tools_fully_functional() {
        assert!(contains_action_claims("All tools are fully functional!"));
    }

    #[test]
    fn test_detects_all_tests_passed() {
        assert!(contains_action_claims("All tests passed successfully."));
    }

    #[test]
    fn test_detects_all_tests_successful() {
        assert!(contains_action_claims("All tests were successful."));
    }

    #[test]
    fn test_detects_successfully_executed() {
        assert!(contains_action_claims("Successfully executed the command."));
    }

    #[test]
    fn test_detects_successfully_tested() {
        assert!(contains_action_claims("Successfully tested all endpoints."));
    }

    #[test]
    fn test_detects_already_completed() {
        assert!(contains_action_claims("Already completed the migration."));
    }

    #[test]
    fn test_tool_name_mentions_detects_hallucination() {
        let tool_names = vec![
            "web_search".to_string(),
            "weather".to_string(),
            "cron".to_string(),
            "reddit".to_string(),
            "exec".to_string(),
        ];
        let text = "## Tool Test Results\n- web_search - Found news\n- weather - 45°F\n- cron - 5 jobs\n- reddit - 10 posts";
        assert!(mentions_multiple_tools(text, &tool_names));
    }

    #[test]
    fn test_tool_name_mentions_no_false_positive() {
        let tool_names = vec![
            "web_search".to_string(),
            "weather".to_string(),
            "cron".to_string(),
        ];
        // Only mentions 1 tool name — should not trigger
        let text = "I can help you with web_search if you'd like.";
        assert!(!mentions_multiple_tools(text, &tool_names));
    }

    #[test]
    fn test_tool_name_mentions_exactly_two_no_trigger() {
        let tool_names = vec![
            "web_search".to_string(),
            "weather".to_string(),
            "cron".to_string(),
        ];
        let text = "The web_search and weather tools are available.";
        assert!(!mentions_multiple_tools(text, &tool_names));
    }

    // --- Silent response tests ---

    #[test]
    fn test_silent_response_detected() {
        let content = "[SILENT] Internal note recorded.";
        assert!(content.starts_with("[SILENT]"));
    }

    #[test]
    fn test_silent_prefix_case_sensitive() {
        // Lowercase [silent] should NOT be treated as silent
        let content = "[silent] This should pass through.";
        assert!(!content.starts_with("[SILENT]"));
    }

    #[test]
    fn test_non_silent_response_passes_through() {
        let content = "Here is a normal response.";
        assert!(!content.starts_with("[SILENT]"));
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

    fn empty_tools() -> Vec<String> {
        vec![]
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
                let available = empty_tools();
                tokio::task::spawn(async move {
                    execute_tool_call_inner(&tc_id, &tc_name, &tc_args, tool_opt, &available).await
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
        let (result, is_error) = execute_tool_call(&tc, tool, empty_tools()).await;

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
                let available = empty_tools();
                tokio::task::spawn(async move {
                    execute_tool_call_inner(&tc_id, &tc_name, &tc_args, tool_opt, &available).await
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
                let available = empty_tools();
                tokio::task::spawn(async move {
                    execute_tool_call_inner(&tc_id, &tc_name, &tc_args, tool_opt, &available).await
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

    // --- Unknown tool error improvement tests ---

    #[tokio::test]
    async fn test_unknown_tool_lists_available() {
        let available = vec![
            "read_file".to_string(),
            "write_file".to_string(),
            "exec".to_string(),
        ];
        let (result, is_error) = execute_tool_call_inner(
            "id1",
            "nonexistent_tool",
            &serde_json::json!({}),
            None,
            &available,
        )
        .await;
        assert!(is_error);
        assert!(result.contains("does not exist"));
        assert!(result.contains("read_file"));
        assert!(result.contains("write_file"));
        assert!(result.contains("exec"));
    }

    // --- Schema validation tests ---

    /// A mock tool with a defined parameter schema for validation tests.
    struct SchemaTestTool;

    #[async_trait]
    impl Tool for SchemaTestTool {
        fn name(&self) -> &str {
            "schema_test"
        }
        fn description(&self) -> &str {
            "test tool with schema"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "count": { "type": "integer" },
                    "verbose": { "type": "boolean" },
                    "tags": { "type": "array" },
                    "options": { "type": "object" }
                },
                "required": ["query"]
            })
        }
        async fn execute(&self, _params: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::new("ok".to_string()))
        }
    }

    #[test]
    fn test_validate_params_missing_required() {
        let tool = SchemaTestTool;
        let params = serde_json::json!({});
        let result = validate_tool_params(&tool, &params);
        assert!(result.is_some());
        let msg = result.unwrap();
        assert!(msg.contains("missing required parameter 'query'"));
    }

    #[test]
    fn test_validate_params_wrong_type() {
        let tool = SchemaTestTool;
        // query should be string, but we pass a number
        let params = serde_json::json!({"query": 42});
        let result = validate_tool_params(&tool, &params);
        assert!(result.is_some());
        let msg = result.unwrap();
        assert!(msg.contains("parameter 'query' should be string but got number"));
    }

    #[test]
    fn test_validate_params_valid() {
        let tool = SchemaTestTool;
        let params = serde_json::json!({"query": "hello", "count": 5, "verbose": true});
        let result = validate_tool_params(&tool, &params);
        assert!(result.is_none());
    }

    #[test]
    fn test_validate_params_no_required() {
        // MockTool has empty schema (no required array) — should always pass
        let tool = MockTool {
            tool_name: "no_schema".into(),
            delay_ms: 0,
            response: "ok".into(),
        };
        let params = serde_json::json!({});
        let result = validate_tool_params(&tool, &params);
        assert!(result.is_none());
    }

    #[test]
    fn test_validate_params_optional_missing_ok() {
        let tool = SchemaTestTool;
        // Only required field "query" is provided; optional fields omitted — should pass
        let params = serde_json::json!({"query": "test"});
        let result = validate_tool_params(&tool, &params);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_validation_rejects_before_execution() {
        // Tool with required param "query" — call without it, should get validation error
        let tool: Option<Arc<dyn Tool>> = Some(Arc::new(SchemaTestTool));
        let (result, is_error) = execute_tool_call_inner(
            "id1",
            "schema_test",
            &serde_json::json!({}),
            tool,
            &empty_tools(),
        )
        .await;
        assert!(is_error);
        assert!(result.contains("missing required parameter 'query'"));
    }

    // --- Image loading tests ---

    // Minimal valid magic bytes for each format
    const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
    const PNG_MAGIC: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    const GIF_MAGIC: &[u8] = b"GIF89a";
    fn webp_magic() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&[0x00; 4]); // file size placeholder
        data.extend_from_slice(b"WEBP");
        data
    }

    #[test]
    fn test_load_and_encode_images_valid_jpg() {
        let tmp = tempfile::TempDir::new().unwrap();
        let img_path = tmp.path().join("test.jpg");
        std::fs::write(&img_path, JPEG_MAGIC).unwrap();

        let paths = vec![img_path.to_string_lossy().to_string()];
        let images = load_and_encode_images(&paths);

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].media_type, "image/jpeg");
        assert!(!images[0].data.is_empty());
    }

    #[test]
    fn test_load_and_encode_images_multiple_formats() {
        let tmp = tempfile::TempDir::new().unwrap();

        std::fs::write(tmp.path().join("test.jpg"), JPEG_MAGIC).unwrap();
        std::fs::write(tmp.path().join("test.png"), PNG_MAGIC).unwrap();
        std::fs::write(tmp.path().join("test.gif"), GIF_MAGIC).unwrap();
        std::fs::write(tmp.path().join("test.webp"), webp_magic()).unwrap();

        let paths: Vec<String> = ["jpg", "png", "gif", "webp"]
            .iter()
            .map(|ext| {
                tmp.path()
                    .join(format!("test.{}", ext))
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        let images = load_and_encode_images(&paths);

        assert_eq!(images.len(), 4);
        assert_eq!(images[0].media_type, "image/jpeg");
        assert_eq!(images[1].media_type, "image/png");
        assert_eq!(images[2].media_type, "image/gif");
        assert_eq!(images[3].media_type, "image/webp");
    }

    #[test]
    fn test_load_and_encode_images_skips_missing() {
        let images = load_and_encode_images(&["/nonexistent/path/image.jpg".to_string()]);
        assert!(images.is_empty());
    }

    #[test]
    fn test_load_and_encode_images_skips_unsupported_format() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("test.bmp");
        std::fs::write(&path, b"bmp data").unwrap();

        let images = load_and_encode_images(&[path.to_string_lossy().to_string()]);
        assert!(images.is_empty());
    }

    #[test]
    fn test_load_and_encode_images_rejects_bad_magic_bytes() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Write a .png file with JPEG content
        let path = tmp.path().join("fake.png");
        std::fs::write(&path, JPEG_MAGIC).unwrap();

        let images = load_and_encode_images(&[path.to_string_lossy().to_string()]);
        assert!(images.is_empty(), "should reject mismatched magic bytes");
    }

    #[test]
    fn test_load_and_encode_images_rejects_html_as_image() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Simulate Slack returning HTML instead of image
        let path = tmp.path().join("download.png");
        std::fs::write(&path, b"<html><body>Error</body></html>").unwrap();

        let images = load_and_encode_images(&[path.to_string_lossy().to_string()]);
        assert!(images.is_empty(), "should reject HTML content");
    }

    #[test]
    fn test_load_and_encode_images_max_limit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut paths = Vec::new();
        for i in 0..8 {
            let path = tmp.path().join(format!("img{}.png", i));
            std::fs::write(&path, PNG_MAGIC).unwrap();
            paths.push(path.to_string_lossy().to_string());
        }

        let images = load_and_encode_images(&paths);
        assert_eq!(images.len(), MAX_IMAGES); // Capped at 5
    }

    #[test]
    fn test_load_and_encode_images_empty_input() {
        let images = load_and_encode_images(&[]);
        assert!(images.is_empty());
    }

    #[test]
    fn test_load_and_encode_images_base64_roundtrip() {
        use base64::Engine;
        let tmp = tempfile::TempDir::new().unwrap();
        let img_path = tmp.path().join("test.png");
        // Use valid PNG magic + extra data
        let mut original_data = PNG_MAGIC.to_vec();
        original_data.extend_from_slice(b"extra png data here");
        std::fs::write(&img_path, &original_data).unwrap();

        let images = load_and_encode_images(&[img_path.to_string_lossy().to_string()]);
        assert_eq!(images.len(), 1);

        // Decode and verify roundtrip
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&images[0].data)
            .unwrap();
        assert_eq!(decoded, original_data);
    }

    // --- Media cleanup tests ---

    // --- False no-tools claim detection tests ---

    #[test]
    fn test_false_no_tools_claim_dont_have_tools() {
        assert!(is_false_no_tools_claim(
            "I don't have access to tools to help with that."
        ));
    }

    #[test]
    fn test_false_no_tools_claim_cannot_have_tools() {
        assert!(is_false_no_tools_claim(
            "I cannot have access to any tools."
        ));
    }

    #[test]
    fn test_false_no_tools_claim_unable_to_use() {
        assert!(is_false_no_tools_claim("I'm unable to use tools directly."));
    }

    #[test]
    fn test_false_no_tools_claim_no_tools_available() {
        assert!(is_false_no_tools_claim("No tools are available to me."));
    }

    #[test]
    fn test_false_no_tools_claim_not_triggered_by_normal_text() {
        assert!(!is_false_no_tools_claim(
            "Here's how to use the tools in this project."
        ));
    }

    #[test]
    fn test_false_no_tools_claim_not_triggered_by_tool_usage() {
        assert!(!is_false_no_tools_claim(
            "I'll use the exec tool to run that command."
        ));
    }

    #[test]
    fn test_cleanup_old_media_no_dir() {
        // Should not error when media dir doesn't exist
        // cleanup_old_media uses home_dir, so we can't easily test with a custom path.
        // Instead, test the no-op case: TTL=0 is never called, and missing dir returns Ok.
        // This is a smoke test that the function doesn't panic.
        let result = cleanup_old_media(9999);
        assert!(result.is_ok());
    }
}
