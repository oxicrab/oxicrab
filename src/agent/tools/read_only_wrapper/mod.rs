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
        // Drift guard: if `caps.actions` declares read-only actions
        // but the JSON-schema enum has no overlap, filter_action_enum
        // returns an empty enum with `required: ["action"]` —
        // every LLM call would validation-fail. Skip wrapping in
        // that case so the unwrapped tool stays out of the
        // subagent surface.
        let enum_empty = filtered_schema
            .get("properties")
            .and_then(|p| p.get("action"))
            .and_then(|a| a.get("enum"))
            .and_then(|e| e.as_array())
            .is_some_and(Vec::is_empty);
        if enum_empty {
            return None;
        }
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
        if let Some(action) = params.get("action").and_then(|a| a.as_str()) {
            if !self.read_only_actions.contains(&action) {
                return Ok(ToolResult::error(format!(
                    "action '{action}' is not available in this context (read-only access)"
                )));
            }
            return self.inner.execute(params, ctx).await;
        }
        // Single-purpose read-only tool: exactly one read-only action
        // descriptor and no `action` property in the schema. The capability
        // descriptor is metadata-only; the inner tool doesn't dispatch on
        // `action` and shouldn't be forced to.
        let schema_has_action = self
            .filtered_schema
            .get("properties")
            .and_then(|p| p.get("action"))
            .is_some();
        if self.read_only_actions.len() == 1 && !schema_has_action {
            return self.inner.execute(params, ctx).await;
        }
        Ok(ToolResult::error(
            "action parameter is required".to_string(),
        ))
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

    fn requires_approval_for_action(&self, action: &str) -> bool {
        self.inner.requires_approval_for_action(action)
    }

    fn execution_timeout(&self) -> std::time::Duration {
        self.inner.execution_timeout()
    }
}

#[cfg(test)]
mod tests;
