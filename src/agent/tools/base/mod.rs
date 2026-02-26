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

    /// Convert a `Result<String>` into a `ToolResult`, formatting errors with
    /// the given prefix (e.g. `"GitHub"`). Replaces the common pattern:
    /// `match result { Ok(c) => Ok(ToolResult::new(c)), Err(e) => Ok(ToolResult::error(...)) }`
    pub fn from_result(result: anyhow::Result<String>, error_prefix: &str) -> Self {
        match result {
            Ok(content) => Self::new(content),
            Err(e) => Self::error(format!("{} error: {}", error_prefix, e)),
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
    /// Metadata from the originating inbound message (e.g., Slack `ts` for threading).
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
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

    /// Capability metadata for this tool. Used by subagent builder,
    /// exfiltration guard, and MCP trust filter.
    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::default()
    }
}

/// How a tool should be exposed in subagent contexts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentAccess {
    /// All actions available
    Full,
    /// Only read-only actions exposed; mutating actions hidden from schema
    /// and rejected at execution time
    ReadOnly,
    /// Tool not available to subagents at all (e.g., spawn, cron)
    Denied,
}

/// Per-action metadata for tools using the action dispatch pattern.
#[derive(Debug, Clone)]
pub struct ActionDescriptor {
    /// Action name matching the `"action"` enum value in `parameters()`
    pub name: &'static str,
    /// Whether this action only reads data (no side effects)
    pub read_only: bool,
}

/// Build a `Vec<ActionDescriptor>` concisely.
///
/// Mark read-only actions with `: ro`:
/// ```ignore
/// actions![
///     list_issues: ro,       // read-only action
///     create_issue,          // mutating action (default)
/// ]
/// ```
#[macro_export]
macro_rules! actions {
    (@one $name:ident : ro) => {
        $crate::agent::tools::base::ActionDescriptor { name: stringify!($name), read_only: true }
    };
    (@one $name:ident) => {
        $crate::agent::tools::base::ActionDescriptor { name: stringify!($name), read_only: false }
    };
    ($($name:ident $(: $ro:ident)?),+ $(,)?) => {
        vec![$(actions!(@one $name $(: $ro)?)),+]
    };
}

/// Capability metadata intrinsic to a tool. Queried by the registry,
/// subagent builder, exfiltration guard, and MCP trust filter.
#[derive(Debug, Clone)]
pub struct ToolCapabilities {
    /// Tool is built-in to oxicrab. Protected from MCP shadowing.
    pub built_in: bool,
    /// Tool's primary purpose involves outbound network requests.
    /// Used by exfiltration guard to determine default blocking.
    pub network_outbound: bool,
    /// How this tool should be exposed in subagent contexts.
    pub subagent_access: SubagentAccess,
    /// Per-action metadata. Empty for single-purpose tools.
    /// For action-based tools, every action MUST be listed.
    pub actions: Vec<ActionDescriptor>,
}

impl Default for ToolCapabilities {
    fn default() -> Self {
        Self {
            built_in: false,
            network_outbound: false,
            subagent_access: SubagentAccess::Denied,
            actions: vec![],
        }
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
        _tool: &dyn Tool,
        _result: &mut ToolResult,
    ) {
    }
}

#[cfg(test)]
mod tests;
