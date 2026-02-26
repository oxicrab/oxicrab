use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Wraps an action-based tool to expose only its read-only actions.
/// Dual enforcement: schema filtering (belt) + execution-time rejection (suspenders).
pub struct ReadOnlyToolWrapper {
    inner: Arc<dyn Tool>,
    read_only_actions: Vec<&'static str>,
    filtered_schema: Value,
    filtered_description: String,
}

impl ReadOnlyToolWrapper {
    /// Create a read-only wrapper. Returns `None` if the tool has no read-only actions.
    pub fn new(tool: Arc<dyn Tool>) -> Option<Self> {
        let caps = tool.capabilities();
        let read_only_actions: Vec<&'static str> = caps
            .actions
            .iter()
            .filter(|a| a.read_only)
            .map(|a| a.name)
            .collect();

        if read_only_actions.is_empty() {
            return None;
        }

        let filtered_schema = filter_action_enum(&tool.parameters(), &read_only_actions);
        let base_desc = tool
            .description()
            .split(". Actions:")
            .next()
            .unwrap_or(tool.description());
        let filtered_description = format!(
            "{} (read-only actions: {})",
            base_desc.trim_end_matches('.'),
            read_only_actions.join(", ")
        );

        Some(Self {
            inner: tool,
            read_only_actions,
            filtered_schema,
            filtered_description,
        })
    }
}

/// Filter the action enum in a parameters JSON schema to only include allowed actions.
fn filter_action_enum(schema: &Value, allowed: &[&str]) -> Value {
    let mut filtered = schema.clone();
    if let Some(Value::Array(arr)) = filtered
        .get_mut("properties")
        .and_then(|p| p.get_mut("action"))
        .and_then(|a| a.get_mut("enum"))
    {
        arr.retain(|v| v.as_str().is_some_and(|s| allowed.contains(&s)));
    }
    filtered
}

#[async_trait]
impl Tool for ReadOnlyToolWrapper {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        &self.filtered_description
    }

    fn parameters(&self) -> Value {
        self.filtered_schema.clone()
    }

    async fn execute(&self, params: Value, ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        if let Some(action) = params.get("action").and_then(|a| a.as_str())
            && !self.read_only_actions.contains(&action)
        {
            return Ok(ToolResult::error(format!(
                "action '{}' is not available in this context (read-only access)",
                action
            )));
        }
        self.inner.execute(params, ctx).await
    }

    fn capabilities(&self) -> ToolCapabilities {
        let mut caps = self.inner.capabilities();
        // Already filtered — mark as Full so subagent builder doesn't re-wrap
        caps.subagent_access = SubagentAccess::Full;
        caps
    }

    fn cacheable(&self) -> bool {
        self.inner.cacheable()
    }

    fn requires_approval(&self) -> bool {
        self.inner.requires_approval()
    }

    fn execution_timeout(&self) -> std::time::Duration {
        self.inner.execution_timeout()
    }
}

#[cfg(test)]
mod tests;
