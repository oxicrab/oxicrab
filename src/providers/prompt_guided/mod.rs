use crate::providers::base::{
    ChatRequest, LLMProvider, LLMResponse, Message, ToolCallRequest, ToolDefinition,
};
use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;
use std::fmt::Write;
use std::sync::{Arc, LazyLock};
use tracing::{debug, warn};

#[cfg(test)]
mod tests;

/// Render a compact text representation of tool definitions for injection into system prompts.
pub fn render_tool_definitions(tools: &[ToolDefinition]) -> String {
    let mut out = String::from(
        "\n\n## Available Tools\n\n\
         To use a tool, respond with a <tool_call> XML block. You may use multiple.\n\n\
         <tool_call>\n\
         {\"name\": \"tool_name\", \"arguments\": {\"param1\": \"value1\"}}\n\
         </tool_call>\n\n\
         ### Tools\n",
    );

    for tool in tools {
        let _ = write!(out, "\n**{}** - {}\n", tool.name, tool.description);
        let params_text = render_parameters(&tool.parameters);
        if !params_text.is_empty() {
            out.push_str("Parameters:\n");
            out.push_str(&params_text);
        }
    }

    out
}

/// Render JSON Schema parameters into a human-readable list.
fn render_parameters(schema: &Value) -> String {
    let Some(Value::Object(properties)) = schema.get("properties") else {
        return String::new();
    };

    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut out = String::new();
    for (name, prop) in properties {
        let type_str = prop.get("type").and_then(Value::as_str).unwrap_or("any");
        let is_required = required.contains(&name.as_str());
        let req_label = if is_required { "required" } else { "optional" };

        let _ = write!(out, "- {name} ({type_str}, {req_label})");

        if let Some(desc) = prop.get("description").and_then(Value::as_str) {
            let _ = write!(out, ": {desc}");
        }

        if let Some(Value::Array(vals)) = prop.get("enum") {
            let enum_strs: Vec<String> = vals
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect();
            let _ = write!(out, " [{}]", enum_strs.join(", "));
        }

        out.push('\n');
    }

    out
}

/// Regex for matching `<tool_call>...</tool_call>` XML blocks.
/// Uses a greedy match with `</tool_call>` as the terminator to handle nested JSON braces.
fn tool_call_xml_re() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?s)<tool_call>\s*(\{.*?)</tool_call>")
            .expect("failed to compile tool_call XML regex")
    });
    &RE
}

/// Regex for matching `` ```json ... ``` `` code blocks containing tool calls.
/// Uses a greedy match with closing backticks as the terminator to handle nested JSON braces.
fn tool_call_json_block_re() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?s)```json\s*(\{.*?)```")
            .expect("failed to compile tool_call JSON block regex")
    });
    &RE
}

/// Parse tool calls from LLM text output.
///
/// Tries two formats in priority order:
/// 1. `<tool_call>{"name":"...","arguments":{...}}</tool_call>` XML blocks
/// 2. `` ```json{"name":"...","arguments":{...}}``` `` code blocks
///
/// Returns parsed `ToolCallRequest`s with synthetic IDs and the remaining text
/// with tool call blocks stripped.
pub fn parse_tool_calls_from_text(text: &str) -> (Vec<ToolCallRequest>, Option<String>) {
    let mut calls = Vec::new();
    let mut remaining = text.to_string();

    // Strategy 1: XML tags
    let xml_re = tool_call_xml_re();
    for (i, cap) in xml_re.captures_iter(text).enumerate() {
        if let Some(json_str) = cap.get(1)
            && let Some(tc) = try_parse_tool_call(json_str.as_str(), i + 1)
        {
            calls.push(tc);
        }
    }

    if calls.is_empty() {
        // Strategy 2: JSON code blocks (only if no XML matches)
        let json_re = tool_call_json_block_re();
        for (i, cap) in json_re.captures_iter(text).enumerate() {
            if let Some(json_str) = cap.get(1)
                && let Some(tc) = try_parse_tool_call(json_str.as_str(), i + 1)
            {
                calls.push(tc);
            }
        }
        if !calls.is_empty() {
            remaining = json_re.replace_all(&remaining, "").to_string();
        }
    } else {
        remaining = xml_re.replace_all(&remaining, "").to_string();
    }

    let remaining = remaining.trim().to_string();
    let remaining = if remaining.is_empty() {
        None
    } else {
        Some(remaining)
    };

    (calls, remaining)
}

/// Try to parse a single JSON string as a tool call.
fn try_parse_tool_call(json_str: &str, index: usize) -> Option<ToolCallRequest> {
    let parsed: Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            warn!("failed to parse tool call JSON: {e}");
            return None;
        }
    };

    let name = parsed.get("name")?.as_str()?.to_string();
    if name.is_empty() {
        return None;
    }

    let arguments = parsed
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));

    Some(ToolCallRequest {
        id: format!("prompt_tc_{index}"),
        name,
        arguments,
    })
}

/// An LLM provider wrapper that injects tool definitions into the system prompt
/// and parses `<tool_call>` XML blocks from text responses. This enables tool
/// use with local models (Ollama, vLLM) that don't support native function calling.
pub struct PromptGuidedToolsProvider {
    inner: Arc<dyn LLMProvider>,
}

impl PromptGuidedToolsProvider {
    pub fn wrap(inner: Arc<dyn LLMProvider>) -> Arc<dyn LLMProvider> {
        Arc::new(Self { inner })
    }

    /// Rewrite a chat request: move tool definitions into the system prompt,
    /// clear native `tools`/`tool_choice`, and convert tool-related messages.
    fn rewrite_request(req: ChatRequest<'_>) -> ChatRequest<'_> {
        let tools = match &req.tools {
            Some(t) if !t.is_empty() => t,
            _ => return req,
        };

        let tool_defs_text = render_tool_definitions(tools);
        let tool_choice_was_any = req.tool_choice.as_deref() == Some("any");

        let mut messages = Vec::with_capacity(req.messages.len());
        let mut injected_tools = false;

        for msg in &req.messages {
            match msg.role.as_str() {
                "system" if !injected_tools => {
                    let mut content = msg.content.clone();
                    content.push_str(&tool_defs_text);
                    if tool_choice_was_any {
                        content.push_str(
                            "\n\nYou MUST respond by calling at least one tool \
                             using the `<tool_call>` format above.",
                        );
                    }
                    messages.push(Message {
                        role: "system".into(),
                        content,
                        tool_calls: None,
                        tool_call_id: None,
                        is_error: msg.is_error,
                        images: msg.images.clone(),
                        ..Default::default()
                    });
                    injected_tools = true;
                }
                "assistant" if msg.tool_calls.is_some() => {
                    // Convert assistant tool_call messages into inline <tool_call> text
                    let mut content = msg.content.clone();
                    if let Some(tcs) = &msg.tool_calls {
                        for tc in tcs {
                            let json = serde_json::json!({
                                "name": tc.name,
                                "arguments": tc.arguments,
                            });
                            let _ = write!(
                                content,
                                "\n<tool_call>\n{}\n</tool_call>",
                                serde_json::to_string(&json).unwrap_or_default()
                            );
                        }
                    }
                    messages.push(Message {
                        role: "assistant".into(),
                        content,
                        tool_calls: None,
                        tool_call_id: None,
                        is_error: false,
                        images: msg.images.clone(),
                        ..Default::default()
                    });
                }
                "tool" => {
                    // Convert tool result messages into user messages
                    let tool_id = msg.tool_call_id.as_deref().unwrap_or("unknown");
                    let content = format!("[Tool result for {tool_id}]:\n{}", msg.content);
                    messages.push(Message {
                        role: "user".into(),
                        content,
                        tool_calls: None,
                        tool_call_id: None,
                        is_error: false,
                        images: vec![],
                        ..Default::default()
                    });
                }
                _ => {
                    messages.push(msg.clone());
                }
            }
        }

        // If no system message was found, prepend one with tool defs
        if !injected_tools {
            let mut content = tool_defs_text;
            if tool_choice_was_any {
                content.push_str(
                    "\n\nYou MUST respond by calling at least one tool \
                     using the `<tool_call>` format above.",
                );
            }
            messages.insert(
                0,
                Message {
                    role: "system".into(),
                    content,
                    tool_calls: None,
                    tool_call_id: None,
                    is_error: false,
                    images: vec![],
                    ..Default::default()
                },
            );
        }

        ChatRequest {
            messages,
            tools: None,
            model: req.model,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            tool_choice: None,
            response_format: req.response_format.clone(),
        }
    }
}

#[async_trait]
impl LLMProvider for PromptGuidedToolsProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        let has_tools = req.tools.as_ref().is_some_and(|t| !t.is_empty());

        if !has_tools {
            debug!("prompt-guided: no tools in request, passthrough");
            return self.inner.chat(req).await;
        }

        let rewritten = Self::rewrite_request(req);
        let mut response = self.inner.chat(rewritten).await?;

        // If the inner provider already returned tool calls, pass through
        if response.has_tool_calls() {
            return Ok(response);
        }

        // Try to parse tool calls from text content
        if let Some(ref content) = response.content {
            let (parsed_calls, remaining_text) = parse_tool_calls_from_text(content);
            if !parsed_calls.is_empty() {
                debug!(
                    "prompt-guided: parsed {} tool call(s) from text",
                    parsed_calls.len()
                );
                response.tool_calls = parsed_calls;
                response.content = remaining_text;
            }
        }

        Ok(response)
    }

    fn default_model(&self) -> &str {
        self.inner.default_model()
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        self.inner.warmup().await
    }
}
