use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
}

impl ToolResult {
    pub fn new(content: String) -> Self {
        Self { content }
    }

    pub fn error(content: String) -> Self {
        Self { content }
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
        Self { major, minor, patch }
    }

    pub fn to_string(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Default for ToolVersion {
    fn default() -> Self {
        Self::new(1, 0, 0)
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value; // JSON Schema

    async fn execute(&self, params: Value) -> anyhow::Result<ToolResult>;

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

    /// Set context for tools that need it (channel, chat_id).
    /// Default implementation does nothing - tools that need context override this.
    async fn set_context(&self, _channel: &str, _chat_id: &str) {
        // Default: no-op
    }
}
