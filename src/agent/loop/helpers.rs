use crate::agent::tools::ToolRegistry;
use crate::agent::tools::base::{ExecutionContext, ToolResult};
use crate::bus::OutboundMessage;
use crate::providers::base::ImageData;
use anyhow::Result;
use jsonschema::error::ValidationErrorKind;
use regex::Regex;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Context for the operator approval flow, passed into [`execute_tool_call`].
/// When `None`, the approval gate is skipped (backward-compatible with tests).
pub(super) struct ApprovalContext<'a> {
    pub store: &'a crate::agent::approval::ApprovalStore,
    pub config: &'a crate::config::ApprovalConfig,
    pub outbound_tx: &'a tokio::sync::mpsc::Sender<OutboundMessage>,
    pub leak_detector: &'a crate::safety::LeakDetector,
    pub channel: &'a str,
    pub chat_id: &'a str,
    pub sender_id: &'a str,
}

const SAVED_TO_PREFIX: &str = "saved to: ";
const AUDIO_TAG_PREFIX: &str = "[audio: ";
const TYPING_INDICATOR_INTERVAL_SECS: u64 = 4;
const MAX_IMAGE_SIZE: usize = 20 * 1024 * 1024; // 20MB (Anthropic limit)
pub(super) const MAX_IMAGES: usize = 5;

/// Extract media file paths from a tool result string.
///
/// Looks for:
/// - JSON `"mediaPath"` fields (from `web_fetch` / `http` binary downloads)
/// - "Screenshot saved to: /path" or "Binary content saved to: /path" patterns
///
/// Only paths inside the oxicrab media directory are accepted to prevent
/// untrusted tool output (e.g. MCP servers) from exfiltrating arbitrary files.
pub(super) fn extract_media_paths(result: &str) -> Vec<String> {
    let media_dir = crate::utils::media::media_dir().ok();
    let mut paths = Vec::new();

    // Try JSON parsing for mediaPath
    if let Ok(json) = serde_json::from_str::<Value>(result)
        && let Some(path) = json.get("mediaPath").and_then(Value::as_str)
        && is_safe_media_path(path, media_dir.as_deref())
    {
        paths.push(path.to_string());
    }

    // Text pattern: "saved to: /path" (browser screenshots, http binary)
    for line in result.lines() {
        if let Some(idx) = line.find(SAVED_TO_PREFIX) {
            let path = line[idx + SAVED_TO_PREFIX.len()..].trim();
            if !path.is_empty() && is_safe_media_path(path, media_dir.as_deref()) {
                paths.push(path.to_string());
            }
        }
    }

    paths.sort();
    paths.dedup();
    paths
}

/// Check that a path exists and is inside the trusted media directory.
fn is_safe_media_path(path: &str, media_dir: Option<&std::path::Path>) -> bool {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return false;
    }
    let Some(media) = media_dir else {
        return false;
    };
    p.canonicalize()
        .is_ok_and(|canonical| canonical.starts_with(media))
}

/// Validate tool arguments against the tool's JSON schema.
/// Uses full JSON Schema validation (draft auto-detected by `jsonschema`).
/// Returns None if valid, `Some(error_message)` if invalid.
pub(crate) fn validate_tool_params(
    tool: &dyn crate::agent::tools::base::Tool,
    params: &Value,
) -> Option<String> {
    let schema = tool.parameters();
    let compiled = match jsonschema::validator_for(&schema) {
        Ok(c) => c,
        Err(e) => {
            return Some(format!("Invalid schema for tool '{}': {}", tool.name(), e));
        }
    };
    if compiled.is_valid(params) {
        return None;
    }

    let errors: Vec<String> = compiled
        .iter_errors(params)
        .take(6)
        .map(|err| match err.kind() {
            ValidationErrorKind::Required { property } => {
                format!("missing required parameter '{property}'")
            }
            ValidationErrorKind::AdditionalProperties { unexpected } => {
                format!("unknown parameter(s) {}", unexpected.join(", "))
            }
            _ => {
                let path = err.instance_path().to_string();
                if path.is_empty() {
                    err.to_string()
                } else {
                    format!("{path}: {err}")
                }
            }
        })
        .collect();
    Some(format!(
        "Invalid arguments for tool '{}': {}",
        tool.name(),
        errors.join("; ")
    ))
}

/// Execute a tool call via the registry's middleware pipeline.
///
/// Performs pre-execution checks (exfiltration guard, MCP approval, param
/// validation) before delegating to the registry, which handles caching,
/// timeout, panic isolation, truncation, and logging. Also handles the
/// "tool not found" case and converts the result to `(String, bool)`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_tool_call(
    registry: &ToolRegistry,
    tc_name: &str,
    tc_args: &Value,
    available_tools: &[String],
    ctx: &ExecutionContext,
    exfil_allow: Option<&[String]>,
    workspace: Option<&std::path::Path>,
    approval_ctx: Option<ApprovalContext<'_>>,
) -> ToolResult {
    // Exfiltration guard: block network-outbound tools the LLM shouldn't call
    if let Some(allow_tools) = exfil_allow {
        let is_network = registry
            .get(tc_name)
            .is_some_and(|t| t.capabilities().network_outbound);
        if is_network && !allow_tools.contains(&tc_name.to_string()) {
            warn!("security: exfiltration guard blocked tool: {}", tc_name);
            return ToolResult::error(
                "Error: this tool is not available in the current security mode",
            );
        }
    }

    // Check if tool exists before delegating to registry
    let Some(tool) = registry.get(tc_name) else {
        warn!("LLM called unknown tool: {}", tc_name);
        return ToolResult::error(format!(
            "Error: tool '{}' does not exist. Available tools: {}",
            tc_name,
            available_tools.join(", ")
        ));
    };

    // Interactive approval flow (when enabled)
    let action = tc_args.get("action").and_then(|v| v.as_str()).unwrap_or("");
    if let Some(ref approval) = approval_ctx {
        let tool_caps = tool.capabilities();
        if approval.config.enabled && approval.config.covers(tc_name, action, &tool_caps.actions) {
            return await_approval(
                tool.as_ref(),
                tc_name,
                action,
                tc_args,
                ctx,
                approval.store,
                approval.config,
                approval.outbound_tx,
                approval.leak_detector,
                approval.channel,
                approval.chat_id,
                approval.sender_id,
            )
            .await;
        }
    }

    // Legacy hard-block: per-action approval (only reached when interactive
    // approval is disabled or the action is not covered)
    if tool.requires_approval_for_action(action) {
        warn!(
            "blocked tool requiring approval: {} (action={})",
            tc_name, action
        );
        return ToolResult::error(format!(
            "Error: tool '{tc_name}' requires approval for this action. \
             Change the server's trust level to \"local\" in config to allow execution."
        ));
    }

    // Validate params against schema before execution
    if let Some(validation_error) = validate_tool_params(tool.as_ref(), tc_args) {
        warn!(
            "Tool '{}' param validation failed: {}",
            tc_name, validation_error
        );
        return ToolResult::error(validation_error);
    }

    match registry.execute(tc_name, tc_args.clone(), ctx).await {
        Ok(result) => result,
        Err(e) => {
            warn!("Tool '{}' failed: {}", tc_name, e);
            let msg = crate::utils::path_sanitize::sanitize_error_message(
                &format!("Tool execution failed: {e}"),
                workspace,
            );
            ToolResult::error(msg)
        }
    }
}

/// Wait for operator approval before executing a tool.
///
/// Sends a feedback message to the user, an approval request with buttons to
/// the operator channel, then blocks on a oneshot receiver until the operator
/// responds or the timeout expires.
#[allow(clippy::too_many_arguments)]
async fn await_approval(
    tool: &dyn crate::agent::tools::base::Tool,
    tool_name: &str,
    action: &str,
    params: &Value,
    ctx: &ExecutionContext,
    store: &crate::agent::approval::ApprovalStore,
    config: &crate::config::ApprovalConfig,
    outbound_tx: &tokio::sync::mpsc::Sender<OutboundMessage>,
    leak_detector: &crate::safety::LeakDetector,
    channel: &str,
    chat_id: &str,
    sender_id: &str,
) -> ToolResult {
    use crate::agent::approval::{ApprovalDecision, ApprovalEntry, ApprovalStore};

    let approval_id = ApprovalStore::generate_id();
    let (tx, rx) = tokio::sync::oneshot::channel();

    let display_action = if action.is_empty() {
        tool.capabilities()
            .actions
            .first()
            .map_or("execute", |a| a.name)
            .to_string()
    } else {
        action.to_string()
    };

    // Determine operator channel target
    let (operator_target, operator_channel_key) = if config.channel.is_empty() {
        ((channel.to_string(), chat_id.to_string()), String::new())
    } else if let Some((ch, id)) = config.channel.split_once(':') {
        ((ch.to_string(), id.to_string()), config.channel.clone())
    } else {
        warn!(
            "invalid approval channel format '{}', falling back to same conversation",
            config.channel
        );
        // Use empty key so self-approval semantics apply (accept any source)
        ((channel.to_string(), chat_id.to_string()), String::new())
    };

    // Register the pending approval
    store.register(
        &approval_id,
        ApprovalEntry {
            sender: tx,
            tool_name: tool_name.to_string(),
            action: display_action.clone(),
            requested_by: sender_id.to_string(),
            operator_channel: operator_channel_key,
        },
    );

    // Send feedback to user
    let feedback = OutboundMessage::builder(
        channel,
        chat_id,
        format!(
            "This action requires approval. Waiting for an operator to approve `{tool_name}.{display_action}`..."
        ),
    )
    .build();
    let _ = outbound_tx.send(feedback).await;

    // Build and send approval request to operator
    let request_text = format_approval_request(
        tool_name,
        &display_action,
        sender_id,
        channel,
        chat_id,
        params,
        leak_detector,
    );
    let approve_ctx = serde_json::json!({
        "tool": "__approval",
        "params": {"approval_id": approval_id, "decision": "approved"}
    })
    .to_string();
    let deny_ctx = serde_json::json!({
        "tool": "__approval",
        "params": {"approval_id": approval_id, "decision": "denied"}
    })
    .to_string();

    let buttons = vec![
        serde_json::json!({"id": format!("approve_{approval_id}"), "label": "Approve", "style": "primary", "context": approve_ctx}),
        serde_json::json!({"id": format!("deny_{approval_id}"), "label": "Deny", "style": "danger", "context": deny_ctx}),
    ];

    let request_msg =
        OutboundMessage::builder(&operator_target.0, &operator_target.1, request_text)
            .meta(
                crate::bus::meta::BUTTONS.to_string(),
                serde_json::Value::Array(buttons),
            )
            .build();
    let _ = outbound_tx.send(request_msg).await;

    // Wait for approval decision
    match tokio::time::timeout(std::time::Duration::from_secs(config.timeout), rx).await {
        Ok(Ok(ApprovalDecision::Approved)) => {
            info!("approval granted for {tool_name}.{display_action} (requested by {sender_id})");
            tool.execute(params.clone(), ctx).await.unwrap_or_else(|e| {
                ToolResult::error(format!("tool execution failed after approval: {e}"))
            })
        }
        Ok(Ok(ApprovalDecision::Denied { reason })) => {
            let reason_str = reason.map(|r| format!(": {r}")).unwrap_or_default();
            info!(
                "approval denied for {tool_name}.{display_action} (requested by {sender_id}){reason_str}"
            );
            ToolResult::error(format!("action denied by operator{reason_str}"))
        }
        _ => {
            // Clean up the timed-out entry to prevent unbounded growth
            store.remove(&approval_id);
            warn!("approval timed out for {tool_name}.{display_action} (requested by {sender_id})");
            ToolResult::error("approval timed out — action not executed")
        }
    }
}

fn format_approval_request(
    tool_name: &str,
    action: &str,
    sender_id: &str,
    channel: &str,
    chat_id: &str,
    params: &Value,
    leak_detector: &crate::safety::LeakDetector,
) -> String {
    let mut lines = vec![
        "Approval Request".to_string(),
        String::new(),
        format!("Tool: {tool_name} -> {action}"),
        format!("Requested by: {sender_id} ({channel} {chat_id})"),
    ];

    if let Some(obj) = params.as_object() {
        lines.push(String::new());
        let mut count = 0;
        for (key, value) in obj {
            if key == "action" {
                continue;
            }
            if count >= 10 {
                let remaining = obj.len() - count - obj.keys().filter(|k| *k == "action").count();
                if remaining > 0 {
                    lines.push(format!("[{remaining} more parameter(s) not shown]"));
                }
                break;
            }
            let val_str = if let Some(s) = value.as_str() {
                if s.len() > 500 {
                    let boundary = s.floor_char_boundary(500);
                    format!("{}...\n[{} chars total]", &s[..boundary], s.len())
                } else {
                    s.to_string()
                }
            } else {
                let s = value.to_string();
                if s.len() > 500 {
                    let boundary = s.floor_char_boundary(500);
                    format!("{}...\n[{} chars total]", &s[..boundary], s.len())
                } else {
                    s
                }
            };
            // Redact any secrets in parameter values before sending to operator channel
            let redacted = leak_detector.redact(&val_str);
            lines.push(format!("{key}: {redacted}"));
            count += 1;
        }
    }

    lines.join("\n")
}

/// Action claim regex fragments. Each captures a distinct hallucination pattern.
/// Composed into `ACTION_CLAIM_RE` via alternation.
///
/// Pattern groups:
/// - `FIRST_PERSON_PERFECT`: "I've updated", "I have created"
/// - `FIRST_PERSON_PAST`: "I updated", "I wrote", "I created"
/// - `PASSIVE_CHANGES`: "Changes have been made", "Updates were applied"
/// - `PASSIVE_ENTITY`: "File has been updated", "Config was modified"
/// - `STATUS_ALL`: "All tools working", "All tests passed"
/// - `ADVERB_PAST`: "Successfully executed", "Already completed"
/// - `TERSE_LINE_START`: "Created: ...", "Done!", "Updated —"
/// - `PRESENT_PROGRESSIVE`: "I'm creating...", "I am updating..."
/// - `GERUND_LINE_START`: "Creating now...", "Setting up the events..."
/// - `INTENT_STATEMENT`: "Let me create...", "I'll add...", "Going to schedule..."
pub(super) const ACTION_CLAIM_PATTERNS: &[&str] = &[
    // "I've updated/written/created..." or "I have updated/written/created..."
    r"\bI(?:'ve| have) (?:updated|written|created|set up|configured|saved|deleted|removed|added|modified|changed|installed|fixed|applied|edited|committed|deployed|sent|scheduled|enabled|disabled|tested|ran|executed|fetched|retrieved|processed|searched|checked|verified|completed|performed|called|started|listed|read|generated|triggered|downloaded|uploaded|moved|renamed|opened|closed|built|pushed|pulled|scanned|submitted|reviewed|organized)\b",
    // "I updated/wrote/created..." (simple past)
    r"\bI (?:updated|wrote|created|set up|configured|saved|deleted|removed|added|modified|changed|installed|fixed|applied|edited|committed|deployed|sent|scheduled|enabled|disabled|tested|ran|executed|fetched|retrieved|processed|searched|checked|verified|completed|performed|called|started|listed|read|generated|triggered|downloaded|uploaded|moved|renamed|opened|closed|built|pushed|pulled|scanned|submitted|reviewed|organized)\b",
    // "Changes have been made", "Updates were applied"
    r"\b(?:Changes|Updates|Modifications) (?:have been|were) (?:made|applied|saved|committed)\b",
    // "File has been updated", "Config was modified"
    r"\b(?:File|Config|Settings?) (?:has been|was) (?:updated|written|created|modified|saved|deleted)\b",
    // "All tools working", "All tests passed"
    r"\bAll (?:tools?|tests?|checks?) (?:are |were |have been )?(?:fully )?(?:working|functional|successful|passing|passed|completed)\b",
    // "Successfully executed", "Already completed"
    r"\b(?:Successfully|Already) (?:tested|executed|completed|verified|fetched|retrieved|processed|ran|performed|called|created|updated|sent|deleted|generated|triggered|configured|scheduled|built|searched|listed|submitted)\b",
    // Terse line-start claims: "Created: ...", "Done!", "Updated —"
    r"(?:^|\n)\s*(?:\w+ )?(?:Created|Updated|Deleted|Removed|Added|Saved|Sent|Scheduled|Completed|Done|Configured|Fixed|Applied|Deployed|Executed|Started|Enabled|Disabled|Retrieved|Processed|Generated|Submitted|Triggered|Marked(?: as)? (?:complete|done)) *[:\u{2014}!]",
    // "I'm creating/updating/adding..." or "I am creating..."
    r"\bI(?:'m| am) (?:creating|updating|deleting|removing|adding|modifying|configuring|setting up|saving|sending|scheduling|enabling|disabling|fixing|deploying|executing|installing|editing|fetching|retrieving|processing|searching|checking|starting|running|writing|reading|completing|generating|triggering|building|testing|listing|submitting|downloading|uploading|reviewing|scanning|opening|closing|moving|renaming|organizing)\b",
    // Gerund line-start: "Creating now...", "Setting up the events...", "Creating 4 calendar events now..."
    r"(?:^|\n)\s*(?:Creating|Updating|Deleting|Removing|Adding|Modifying|Configuring|Setting up|Saving|Sending|Scheduling|Enabling|Disabling|Deploying|Executing|Installing|Editing|Fetching|Retrieving|Processing|Running|Writing|Completing|Generating|Triggering|Building|Testing|Listing|Submitting|Downloading|Uploading|Reviewing|Scanning|Opening|Organizing) (?:\w+ )*?(?:now\b|the |your |it\b|them\b|this |that |all |for |to )",
    // Intent: "Let me create...", "I'll create...", "Going to create..."
    r"\b(?:Let me|I'll|I will|Going to|About to) (?:create|update|delete|remove|add|modify|configure|set up|save|send|schedule|enable|disable|fix|deploy|execute|install|edit|fetch|retrieve|process|get|show|list|find|look up|search|check|start|run|write|read|complete|open|close|move|rename|download|upload|generate|trigger|build|test|review|scan|submit|pull|push|mark|organize|browse|summarize)\b",
];

/// Regex that matches phrases where the LLM claims to have performed an action.
/// Built from composable `ACTION_CLAIM_PATTERNS` fragments.
static ACTION_CLAIM_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    let combined = ACTION_CLAIM_PATTERNS.join("|");
    Regex::new(&format!("(?i)(?:{combined})")).expect("Invalid action claim regex")
});

/// Returns `true` if the text contains phrases claiming actions were performed.
pub fn contains_action_claims(text: &str) -> bool {
    ACTION_CLAIM_RE.is_match(text)
}

/// Load media files (images and documents) from disk and base64-encode them for LLM consumption.
/// Skips files that are missing, too large, or have unsupported formats.
pub(super) fn load_and_encode_images(media_paths: &[String]) -> Vec<ImageData> {
    use base64::Engine;

    let mut images = Vec::new();
    for path in media_paths.iter().take(MAX_IMAGES) {
        let file_path = std::path::Path::new(path);
        if !file_path.exists() {
            warn!("Media file not found: {}", path);
            continue;
        }
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        let media_type = match ext {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "pdf" => "application/pdf",
            _ => {
                warn!("Unsupported media format: {}", ext);
                continue;
            }
        };
        match std::fs::read(file_path) {
            Ok(data) => {
                if data.len() > MAX_IMAGE_SIZE {
                    warn!(
                        "Media file too large ({} bytes, max {}): {}",
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
                    "pdf" => data.starts_with(b"%PDF"),
                    _ => false,
                };
                if !valid {
                    warn!(
                        "Media file {} has invalid magic bytes for format '{}' (first bytes: {:02x?}). File may be corrupted.",
                        path,
                        ext,
                        &data[..8.min(data.len())]
                    );
                    continue;
                }
                let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
                info!(
                    "Encoded media for LLM: {} ({}, {} raw bytes, {} base64 chars)",
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
                warn!("Failed to read media file {}: {}", path, e);
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
            // No closing bracket -- keep the rest as-is
            remaining = &remaining[start..];
            break;
        }
    }
    result.push_str(remaining);
    result.trim().to_string()
}

/// Strip `<think>...</think>` blocks from model output.
/// Some models (`DeepSeek`, `Qwen`) emit inline thinking tags instead of using
/// the structured `reasoning_content` field.
/// Also handles unclosed `<think>` tags (e.g. from output truncation).
pub(super) fn strip_think_tags(content: &str) -> String {
    if !content.contains("<think>") {
        return content.to_string();
    }
    let result = crate::utils::regex::RegexPatterns::think_tags()
        .replace_all(content, "")
        .to_string();
    // Handle unclosed <think> tag: strip everything from it to the end
    if let Some(idx) = result.find("<think>") {
        result[..idx].trim().to_string()
    } else {
        result.trim().to_string()
    }
}

/// Strip `[image: /path/to/file]` tags from message content.
/// These tags are added by channels when images are downloaded, but become
/// redundant (and misleading) once images are base64-encoded into content blocks.
pub(super) fn strip_image_tags(content: &str) -> String {
    replace_bracketed_tags(content, "[image: ", None)
}

/// Strip `[document: /path/to/file]` tags from message content.
/// Same as `strip_image_tags` but for document attachments (PDFs, etc.).
pub(super) fn strip_document_tags(content: &str) -> String {
    replace_bracketed_tags(content, "[document: ", None)
}

/// Replace `[audio: /path/to/file]` tags with a notice when transcription is not configured.
/// This ensures the LLM knows a voice message was sent even without transcription.
pub(super) fn strip_audio_tags(content: &str) -> String {
    replace_bracketed_tags(
        content,
        "[audio: ",
        Some("[Voice message received, but transcription is not configured]"),
    )
}

/// Replace `[audio: /path/to/file]` tags with transcribed text.
pub(super) async fn transcribe_audio_tags(
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
                    let _ = write!(result, "[Voice message: \"{text}\"]");
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
///
/// Uses flat `read_dir` (not recursive `walkdir`) because all channel
/// implementations save media directly into `~/.oxicrab/media/` with
/// flat naming (`telegram_{id}.{ext}`, `discord_{id}.{ext}`, etc.).
/// No channel creates subdirectories, so recursion is unnecessary.
pub(super) fn cleanup_old_media(ttl_days: u32) -> Result<()> {
    let media_dir = crate::utils::get_oxicrab_home()?.join("media");
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

/// Guard that aborts the typing indicator background task on drop.
/// This prevents unbounded background tasks if the caller forgets to abort.
pub(super) struct TypingGuard(tokio::task::JoinHandle<()>);

impl Drop for TypingGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Periodic typing indicator: sends every 4s until the returned guard is dropped.
pub(super) fn start_typing(
    typing_tx: Option<&Arc<tokio::sync::mpsc::Sender<(String, String)>>>,
    ctx: Option<&(String, String)>,
) -> Option<TypingGuard> {
    if let (Some(tx), Some(ctx)) = (typing_tx, ctx) {
        let tx = tx.clone();
        let ctx = ctx.clone();
        Some(TypingGuard(tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(TYPING_INDICATOR_INTERVAL_SECS));
            loop {
                interval.tick().await;
                if tx.send(ctx.clone()).await.is_err() {
                    break;
                }
            }
        })))
    } else {
        None
    }
}
