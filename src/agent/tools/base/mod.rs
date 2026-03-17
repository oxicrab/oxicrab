use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write as _;

/// Usage example for a tool, appended to schema description for LLM accuracy.
#[derive(Debug, Clone)]
pub struct ToolExample {
    pub user_request: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    /// Structured metadata for internal consumption (e.g. suggested buttons).
    /// Never sent to the LLM — carried through the agent loop for processing
    /// by the caller (iteration logic, response builder, etc.).
    pub metadata: Option<HashMap<String, Value>>,
}

impl ToolResult {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            metadata: None,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
            metadata: None,
        }
    }

    /// Convert a `Result<String>` into a `ToolResult`, formatting errors with
    /// the given prefix (e.g. `"GitHub"`). Replaces the common pattern:
    /// `match result { Ok(c) => Ok(ToolResult::new(c)), Err(e) => Ok(ToolResult::error(...)) }`
    pub fn from_result(result: anyhow::Result<String>, error_prefix: &str) -> Self {
        match result {
            Ok(content) => Self::new(content),
            Err(e) => Self::error(format!("{error_prefix} error: {e}")),
        }
    }

    /// Attach structured metadata to this result. The metadata is for internal
    /// consumption only and will not be sent to the LLM.
    #[must_use]
    pub fn with_metadata(mut self, metadata: HashMap<String, Value>) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Attach suggested buttons metadata to this result (if any).
    /// Buttons are surfaced by the agent loop to channels that support them.
    #[must_use]
    pub fn with_buttons(self, buttons: Vec<Value>) -> Self {
        if buttons.is_empty() {
            self
        } else {
            self.with_metadata(HashMap::from([(
                "suggested_buttons".to_string(),
                Value::Array(buttons),
            )]))
        }
    }
}

impl std::fmt::Display for ToolResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.content)
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

    fn to_schema(&self) -> Value {
        let examples = self.usage_examples();
        let description = if examples.is_empty() {
            self.description().to_string()
        } else {
            let mut s = self.description().to_string();
            s.push_str("\n\nExamples:");
            for ex in &examples {
                let _ = write!(
                    s,
                    "\n- {:?} → {}",
                    ex.user_request,
                    serde_json::to_string(&ex.params).unwrap_or_default()
                );
            }
            s
        };
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name(),
                "description": description,
                "parameters": self.parameters()
            }
        })
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

    /// Static routing rules for deterministic dispatch. Called once at registration.
    fn routing_rules(&self) -> Vec<crate::router::rules::StaticRule> {
        Vec::new()
    }

    /// Usage examples appended to tool description in LLM schema.
    fn usage_examples(&self) -> Vec<ToolExample> {
        Vec::new()
    }
}

/// Broad functional category for tool pre-filtering.
///
/// When the total tool count exceeds `TOOL_FILTER_THRESHOLD`, only tools
/// in message-relevant categories (plus `Core` and `System` which are always
/// included) are sent to the LLM, reducing prompt noise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolCategory {
    /// Shell, filesystem, memory search — always included
    Core,
    /// Web search, web fetch, HTTP, browser
    Web,
    /// Google Mail
    Communication,
    /// GitHub
    Development,
    /// Cron, Google Calendar
    Scheduling,
    /// Image generation, media (Radarr/Sonarr)
    Media,
    /// Todoist, Obsidian, workspace, Reddit
    Productivity,
    /// Spawn, subagent control, tmux
    System,
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

/// Extract a required string parameter from a JSON `Value`, returning a
/// `ToolResult::error` if the key is missing or not a string.
///
/// Usage: `let action = require_param!(params, "action");`
#[macro_export]
macro_rules! require_param {
    ($params:expr, $key:literal) => {
        match $params[$key].as_str() {
            Some(v) => v,
            None => {
                return Ok($crate::agent::tools::base::ToolResult::error(format!(
                    "Missing '{}' parameter",
                    $key
                )));
            }
        }
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
    /// Functional category for pre-filtering. Defaults to `Core`.
    pub category: ToolCategory,
}

impl Default for ToolCapabilities {
    fn default() -> Self {
        Self {
            built_in: false,
            network_outbound: false,
            subagent_access: SubagentAccess::Denied,
            actions: vec![],
            category: ToolCategory::Core,
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
