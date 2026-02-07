use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    #[allow(dead_code)] // Part of ToolResult structure, may be used for error handling
    pub is_error: bool,
    #[allow(dead_code)] // Part of ToolResult structure, may be used for tool metadata
    pub metadata: std::collections::HashMap<String, Value>,
}

impl ToolResult {
    pub fn new(content: String) -> Self {
        Self {
            content,
            is_error: false,
            metadata: std::collections::HashMap::new(),
        }
    }

    pub fn error(content: String) -> Self {
        Self {
            content,
            is_error: true,
            metadata: std::collections::HashMap::new(),
        }
    }
}

impl std::fmt::Display for ToolResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.content)
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value; // JSON Schema

    async fn execute(&self, params: Value) -> anyhow::Result<ToolResult>;

    fn to_schema(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name(),
                "description": self.description(),
                "parameters": self.parameters()
            }
        })
    }
    
    /// Set context for tools that need it (channel, chat_id).
    /// Default implementation does nothing - tools that need context override this.
    async fn set_context(&self, _channel: &str, _chat_id: &str) {
        // Default: no-op
    }
}
