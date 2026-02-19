use crate::agent::tools::base::{ExecutionContext, Tool, ToolResult};
use async_trait::async_trait;
use rmcp::RoleClient;
use rmcp::model::{CallToolRequestParams, RawContent};
use rmcp::service::Peer;
use serde_json::Value;
use std::borrow::Cow;
use std::fmt::Write;
use std::sync::Arc;
use tracing::{debug, warn};

/// Wraps a single MCP server tool as an `impl Tool` for the oxicrab agent.
pub struct McpProxyTool {
    peer: Peer<RoleClient>,
    tool_name: String,
    /// Leaked string so we can return `&'static str` from `description()`.
    tool_description: &'static str,
    input_schema: Value,
}

impl McpProxyTool {
    pub fn new(
        peer: Peer<RoleClient>,
        _server_name: &str,
        tool_name: String,
        description: String,
        input_schema: Value,
    ) -> Self {
        // Leak the description so we can return &'static str.
        // This is fine because tools are registered once and live for the process lifetime.
        let tool_description: &'static str = Box::leak(description.into_boxed_str());
        Self {
            peer,
            tool_name,
            tool_description,
            input_schema,
        }
    }
}

#[async_trait]
impl Tool for McpProxyTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &'static str {
        self.tool_description
    }

    fn parameters(&self) -> Value {
        self.input_schema.clone()
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        debug!("MCP tool call: {}", self.tool_name);
        // Convert params Value to a Map for the MCP call
        let arguments = match params {
            Value::Object(map) => Some(map),
            Value::Null => None,
            other => {
                // Wrap non-object values in a map
                let mut map = serde_json::Map::new();
                map.insert("input".to_string(), other);
                Some(map)
            }
        };

        let request = CallToolRequestParams {
            meta: None,
            name: Cow::Owned(self.tool_name.clone()),
            arguments,
            task: None,
        };

        let result = match self.peer.call_tool(request).await {
            Ok(r) => r,
            Err(e) => {
                warn!("MCP tool '{}' failed: {}", self.tool_name, e);
                return Ok(ToolResult::error(format!(
                    "MCP tool '{}' call failed: {}",
                    self.tool_name, e
                )));
            }
        };

        let is_error = result.is_error.unwrap_or(false);

        // Convert MCP content blocks to a string result.
        // Content is Annotated<RawContent>, which Derefs to RawContent.
        let mut output = String::new();
        for content in &result.content {
            match &content.raw {
                RawContent::Text(text) => {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&text.text);
                }
                RawContent::Image(img) => {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    let _ = write!(
                        output,
                        "[Image: {} ({} bytes)]",
                        img.mime_type,
                        img.data.len()
                    );
                }
                RawContent::Audio(audio) => {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    let _ = write!(
                        output,
                        "[Audio: {} ({} bytes)]",
                        audio.mime_type,
                        audio.data.len()
                    );
                }
                _ => {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str("[Unsupported MCP content type]");
                }
            }
        }

        if output.is_empty() {
            output = "(no output)".to_string();
        }

        debug!(
            "MCP tool result: {} (error={}, len={})",
            self.tool_name,
            is_error,
            output.len()
        );

        if is_error {
            Ok(ToolResult::error(output))
        } else {
            Ok(ToolResult::new(output))
        }
    }
}

/// Wrapper that forces `requires_approval() = true` for untrusted MCP tools.
pub struct AttenuatedMcpTool {
    inner: Arc<dyn Tool>,
}

impl AttenuatedMcpTool {
    pub fn new(inner: Arc<dyn Tool>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl Tool for AttenuatedMcpTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &'static str {
        self.inner.description()
    }

    fn parameters(&self) -> Value {
        self.inner.parameters()
    }

    async fn execute(&self, params: Value, ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        self.inner.execute(params, ctx).await
    }

    fn requires_approval(&self) -> bool {
        true
    }

    fn execution_timeout(&self) -> std::time::Duration {
        self.inner.execution_timeout()
    }

    fn cacheable(&self) -> bool {
        self.inner.cacheable()
    }
}
