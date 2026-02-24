use crate::agent::tools::ToolRegistry;
use crate::agent::tools::base::ExecutionContext;
use crate::providers::base::ImageData;
use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

const SAVED_TO_PREFIX: &str = "saved to: ";
const AUDIO_TAG_PREFIX: &str = "[audio: ";
const TYPING_INDICATOR_INTERVAL_SECS: u64 = 4;
const TOOL_MENTION_HALLUCINATION_THRESHOLD: usize = 3;
const MAX_IMAGE_SIZE: usize = 20 * 1024 * 1024; // 20MB (Anthropic limit)
pub(super) const MAX_IMAGES: usize = 5;

/// Extract media file paths from a tool result string.
///
/// Looks for:
/// - JSON `"mediaPath"` fields (from `web_fetch` / `http` binary downloads)
/// - "Screenshot saved to: /path" or "Binary content saved to: /path" patterns
pub(super) fn extract_media_paths(result: &str) -> Vec<String> {
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
pub(super) async fn execute_tool_call(
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
        r"(?i)(?:\b(?:I(?:'ve| have) (?:updated|written|created|set up|configured|saved|deleted|removed|added|modified|changed|installed|fixed|applied|edited|committed|deployed|sent|scheduled|enabled|disabled|tested|ran|executed|fetched|searched|checked|verified|completed|performed|called|started|listed|read)|I (?:updated|wrote|created|set up|configured|saved|deleted|removed|added|modified|changed|installed|fixed|applied|edited|committed|deployed|sent|scheduled|enabled|disabled|tested|ran|executed|fetched|searched|checked|verified|completed|performed|called|started|listed|read)|(?:Changes|Updates|Modifications) (?:have been|were) (?:made|applied|saved|committed)|(?:File|Config|Settings?) (?:has been|was) (?:updated|written|created|modified|saved|deleted)|All (?:tools?|tests?|checks?) (?:are |were |have been )?(?:fully )?(?:working|functional|successful|passing|passed|completed)|(?:Successfully|Already) (?:tested|executed|completed|verified|fetched|ran|performed|called|created|updated|sent|deleted))\b|(?:^|\n)(?:Created|Updated|Deleted|Removed|Added|Saved|Sent|Scheduled|Completed|Done|Configured|Fixed|Applied|Deployed|Executed|Started|Enabled|Disabled|Marked(?: as)? (?:complete|done)) *[:\u2014!])"
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
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
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
pub(super) fn cleanup_old_media(ttl_days: u32) -> Result<()> {
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
pub(super) fn start_typing(
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
