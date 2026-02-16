use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }
}

impl std::fmt::Display for ToolResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.content)
    }
}

/// Tool version information
#[derive(Debug, Clone)]
pub struct ToolVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl ToolVersion {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl std::fmt::Display for ToolVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Default for ToolVersion {
    fn default() -> Self {
        Self::new(1, 0, 0)
    }
}

/// Context passed to every tool execution, providing the current channel,
/// chat ID, and an optional conversation summary for context injection.
#[derive(Debug, Clone, Default)]
pub struct ExecutionContext {
    pub channel: String,
    pub chat_id: String,
    pub context_summary: Option<String>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> Value; // JSON Schema

    async fn execute(&self, params: Value, ctx: &ExecutionContext) -> anyhow::Result<ToolResult>;

    /// Get tool version (defaults to 1.0.0)
    fn version(&self) -> ToolVersion {
        ToolVersion::default()
    }

    fn to_schema(&self) -> Value {
        let mut schema = serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name(),
                "description": self.description(),
                "parameters": self.parameters()
            }
        });

        // Add version info if available
        let version = self.version();
        if version.major != 1 || version.minor != 0 || version.patch != 0 {
            schema["function"]["version"] = serde_json::Value::String(version.to_string());
        }

        schema
    }

    /// Whether this tool's results can be cached.
    /// Only read-only tools should return true. Tools that mutate state must return false.
    fn cacheable(&self) -> bool {
        false
    }

    /// Whether this tool requires user approval before execution.
    /// Used for untrusted MCP tools.
    fn requires_approval(&self) -> bool {
        false
    }

    /// Per-tool execution timeout. Overrides the registry-level default.
    fn execution_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_mins(2)
    }
}

/// Middleware that can intercept tool execution for cross-cutting concerns
/// like caching, truncation, and logging.
#[async_trait]
pub trait ToolMiddleware: Send + Sync {
    /// Called before tool execution. Return `Some` to short-circuit (e.g., cache hit).
    async fn before_execute(
        &self,
        _name: &str,
        _params: &Value,
        _ctx: &ExecutionContext,
        _tool: &dyn Tool,
    ) -> Option<ToolResult> {
        None
    }

    /// Called after tool execution. Can modify the result (e.g., truncation).
    async fn after_execute(
        &self,
        _name: &str,
        _params: &Value,
        _ctx: &ExecutionContext,
        _result: &mut ToolResult,
    ) {
    }
}
