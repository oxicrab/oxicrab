use crate::agent::tools::ToolRegistry;
use crate::agent::tools::base::{ExecutionContext, ToolResult};
use crate::bus::OutboundMessage;
use crate::providers::base::ImageData;
use anyhow::Result;
#[cfg(test)]
use jsonschema::error::ValidationErrorKind;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Context for the operator approval flow, passed into [`execute_tool_call`].
/// When `None`, the approval gate is skipped — used by tests that
/// don't exercise the operator approval workflow.
pub(super) struct ApprovalContext<'a> {
    pub store: &'a crate::agent::approval::ApprovalStore,
    pub config: &'a crate::config::ApprovalConfig,
    pub outbound_tx: &'a tokio::sync::mpsc::Sender<OutboundMessage>,
    pub leak_detector: &'a crate::safety::LeakDetector,
    pub channel: &'a str,
    pub chat_id: &'a str,
    pub sender_id: &'a str,
}

/// Context for the LLM-as-Judge gate, passed into [`execute_tool_call`].
/// When `None`, the judge layer is skipped. The judge sees only
/// `(tool_name, args, user_intent)` — no history, no prior results —
/// for poison resistance.
pub(super) struct JudgeContext<'a> {
    pub config: &'a crate::config::JudgeConfig,
    pub provider: &'a dyn crate::providers::base::LLMProvider,
    pub model: &'a str,
    pub user_intent: &'a str,
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
pub(crate) fn extract_media_paths(result: &str) -> Vec<String> {
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
    let Ok(canonical_media) = media.canonicalize() else {
        return false;
    };
    p.canonicalize()
        .is_ok_and(|canonical| canonical.starts_with(&canonical_media))
}

/// Validate tool arguments against the tool's JSON schema.
/// Uses full JSON Schema validation (draft auto-detected by `jsonschema`).
/// Returns None if valid, `Some(error_message)` if invalid.
///
/// Test-only helper. Production validation runs inside the
/// registry's Phase 0.6 (`validate_against_schema`) AFTER
/// `coerce_params_to_schema`, so the registry can rescue common
/// LLM type mismatches (e.g. `{"limit": "5"}`) before validating.
#[cfg(test)]
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
    exfil_allow: Option<&crate::config::DenyByDefaultList>,
    workspace: Option<&std::path::Path>,
    approval_ctx: Option<ApprovalContext<'_>>,
    judge_ctx: Option<JudgeContext<'_>>,
) -> ToolResult {
    // Exfiltration guard: block network-outbound tools the LLM shouldn't call
    if let Some(allow_tools) = exfil_allow {
        let is_network = registry
            .get(tc_name)
            .is_some_and(|t| t.capabilities().network_outbound);
        if is_network && !allow_tools.allows(tc_name) {
            warn!("security: exfiltration guard blocked tool: {}", tc_name);
            return ToolResult::error(
                "Error: this tool is not available in the current security mode",
            );
        }
    }

    // Check if tool exists before delegating to registry
    let Some(tool) = registry.get(tc_name) else {
        warn!("lLM called unknown tool: {}", tc_name);
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
            return await_approval(registry, tc_name, action, tc_args, ctx, approval).await;
        }
    }

    // Per-tool hard-block baseline: only reached when interactive
    // approval is disabled or the action isn't covered by it.
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

    // (JSON-schema validation runs inside ToolRegistry::execute() AFTER
    // coerce_params_to_schema — see registry/mod.rs Phase 0.6. Validating
    // here would reject calls like {"limit": "5"} that coerce would fix.)

    // LLM-as-Judge: poison-resistant semantic gate. Fires after the
    // approval workflow (operators get the explicit click) but before
    // execution. Judge sees only (tool_name, args, user_intent) — no
    // history, no prior results — so an injected page can't poison
    // the gate with the same payload that poisoned the agent.
    if let Some(jctx) = judge_ctx
        && let Some(verdict) = super::judge::judge_tool_call(
            jctx.config,
            jctx.provider,
            jctx.model,
            tc_name,
            tc_args,
            jctx.user_intent,
        )
        .await
        && !verdict.allow
    {
        warn!(
            "judge: blocked tool='{}' action='{}' reason='{}'",
            tc_name, action, verdict.reason
        );
        return ToolResult::error(format!(
            "Tool call blocked by safety judge: {}. Reconsider whether this call matches what the user asked for.",
            verdict.reason
        ));
    }

    match registry.execute(tc_name, tc_args.clone(), ctx).await {
        Ok(result) => result,
        Err(e) => {
            warn!("tool '{}' failed: {}", tc_name, e);
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
async fn await_approval(
    registry: &crate::agent::tools::ToolRegistry,
    tool_name: &str,
    action: &str,
    params: &Value,
    ctx: &ExecutionContext,
    approval: &ApprovalContext<'_>,
) -> ToolResult {
    use crate::agent::approval::{ApprovalDecision, ApprovalEntry, ApprovalStore};

    let approval_id = ApprovalStore::generate_id();
    let (tx, rx) = tokio::sync::oneshot::channel();

    let display_action = if action.is_empty() {
        registry
            .get(tool_name)
            .map(|t| t.capabilities())
            .and_then(|c| c.actions.first().map(|a| a.name.to_string()))
            .unwrap_or_else(|| "execute".to_string())
    } else {
        action.to_string()
    };

    // Determine operator channel target
    let (operator_target, operator_channel_key) = match &approval.config.channel {
        Some(target) => (
            (
                target.channel_type().to_string(),
                target.chat_id().to_string(),
            ),
            target.to_string(),
        ),
        None => {
            // Self-approval: use same conversation, empty key accepts any source
            (
                (approval.channel.to_string(), approval.chat_id.to_string()),
                String::new(),
            )
        }
    };

    // Register the pending approval
    approval.store.register(
        &approval_id,
        ApprovalEntry {
            sender: tx,
            tool_name: tool_name.to_string(),
            action: display_action.clone(),
            requested_by: approval.sender_id.to_string(),
            operator_channel: operator_channel_key,
            source_channel: format!("{}:{}", approval.channel, approval.chat_id),
        },
    );

    // Send feedback to user
    let feedback = OutboundMessage::builder(
        approval.channel,
        approval.chat_id,
        format!(
            "This action requires approval. Waiting for an operator to approve `{tool_name}.{display_action}`..."
        ),
    )
    .build();
    if approval.outbound_tx.send(feedback).await.is_err() {
        approval.store.remove(&approval_id);
        warn!(
            "approval: failed to send feedback to user channel={} chat_id={} — outbound bus closed",
            approval.channel, approval.chat_id
        );
        return ToolResult::error(
            "Could not request approval: message bus is unavailable, try again after restart",
        );
    }

    // Build and send approval request to operator
    let request_text = format_approval_request(
        tool_name,
        &display_action,
        approval.sender_id,
        approval.channel,
        approval.chat_id,
        params,
        approval.leak_detector,
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
    if approval.outbound_tx.send(request_msg).await.is_err() {
        approval.store.remove(&approval_id);
        warn!(
            "approval: failed to send operator request to {}:{} — outbound bus closed",
            operator_target.0, operator_target.1
        );
        return ToolResult::error(
            "Could not deliver approval request to the operator channel, message bus may be closed",
        );
    }

    // Wait for approval decision
    let sender_id = approval.sender_id;
    match tokio::time::timeout(std::time::Duration::from_secs(approval.config.timeout), rx).await {
        Ok(Ok(ApprovalDecision::Approved)) => {
            info!("approval granted for {tool_name}.{display_action} (requested by {sender_id})");
            // Route through the registry to get timeout, panic isolation, truncation, and metrics
            match registry.execute(tool_name, params.clone(), ctx).await {
                Ok(result) => result,
                Err(e) => ToolResult::error(format!("tool execution failed after approval: {e}")),
            }
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
            approval.store.remove(&approval_id);
            warn!("approval timed out for {tool_name}.{display_action} (requested by {sender_id})");
            // Include the (redacted) request the operator didn't act on
            // so the LLM has context to retry or pivot — without it the
            // model only sees "approval timed out" and can't recover.
            let redacted_params = approval.leak_detector.redact(&params.to_string());
            let trimmed_params = if redacted_params.chars().count() > 400 {
                let mut end = 400;
                while !redacted_params.is_char_boundary(end) {
                    end = end.saturating_sub(1);
                }
                format!("{}…", &redacted_params[..end])
            } else {
                redacted_params
            };
            ToolResult::error(format!(
                "approval timed out — action not executed. Request was: \
                 {tool_name}.{display_action} with params {trimmed_params}. \
                 Try again with a different approach or ask the operator directly."
            ))
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
        let has_action_key = obj.contains_key("action");
        let displayable_params = obj.len() - usize::from(has_action_key);
        for (key, value) in obj {
            if key == "action" {
                continue;
            }
            if count >= 10 {
                let remaining = displayable_params - count;
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

/// Load media files (images and documents) from disk and base64-encode them for LLM consumption.
/// Skips files that are missing, too large, or have unsupported formats.
/// Returns `(encoded_images, skip_warnings)` so the caller can surface
/// total or partial failure to the user instead of dropping silently.
pub(super) fn load_and_encode_images(media_paths: &[String]) -> (Vec<ImageData>, Vec<String>) {
    use base64::Engine;

    let mut images = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    for path in media_paths.iter().take(MAX_IMAGES) {
        let file_path = std::path::Path::new(path);
        let display_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path.as_str());
        if !file_path.exists() {
            warn!("media file not found: {}", path);
            warnings.push(format!("'{display_name}' was not found on disk"));
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
                warn!("unsupported media format: {}", ext);
                warnings.push(format!(
                    "'{display_name}' has unsupported extension '.{ext}' (supported: jpg, png, gif, webp, pdf)"
                ));
                continue;
            }
        };
        match std::fs::read(file_path) {
            Ok(data) => {
                if data.len() > MAX_IMAGE_SIZE {
                    warn!(
                        "media file too large ({} bytes, max {}): {}",
                        data.len(),
                        MAX_IMAGE_SIZE,
                        path
                    );
                    warnings.push(format!(
                        "'{display_name}' is {} bytes — over the {}MB limit",
                        data.len(),
                        MAX_IMAGE_SIZE / (1024 * 1024)
                    ));
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
                        "media file {} has invalid magic bytes for format '{}' (first bytes: {:02x?}) — file may be corrupted",
                        path,
                        ext,
                        &data[..8.min(data.len())]
                    );
                    warnings.push(format!(
                        "'{display_name}' is corrupted (extension is .{ext} but file content doesn't match)"
                    ));
                    continue;
                }
                let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
                info!(
                    "encoded media for LLM: {} ({}, {} raw bytes, {} base64 chars)",
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
                warn!("failed to read media file {}: {}", path, e);
                warnings.push(format!("'{display_name}' could not be read: {e}"));
            }
        }
    }
    // If MAX_IMAGES exceeded, surface that too — easy to miss otherwise.
    if media_paths.len() > MAX_IMAGES {
        warnings.push(format!(
            "{} extra media file(s) skipped (limit is {})",
            media_paths.len() - MAX_IMAGES,
            MAX_IMAGES
        ));
    }
    (images, warnings)
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
        info!("cleaned up {} old media files", removed);
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
