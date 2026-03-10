use super::*;
use crate::agent::tools::base::SubagentAccess;
use anyhow::Result;

struct FakeTool;

#[async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &'static str {
        "fake_mcp_tool"
    }
    fn description(&self) -> &'static str {
        "A fake MCP tool"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(&self, _params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        Ok(ToolResult::new("ok".to_string()))
    }
}

#[test]
fn test_attenuated_mcp_capabilities() {
    let inner: Arc<dyn Tool> = Arc::new(FakeTool);
    let tool = AttenuatedMcpTool::new(inner);
    let caps = tool.capabilities();
    assert!(!caps.built_in);
    assert!(caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::Denied);
    assert!(caps.actions.is_empty());
}

#[test]
fn test_attenuated_mcp_requires_approval() {
    let inner: Arc<dyn Tool> = Arc::new(FakeTool);
    let tool = AttenuatedMcpTool::new(inner);
    assert!(tool.requires_approval());
}

#[test]
fn test_null_params_filtered() {
    use serde_json::{Value, json};
    let params = json!({"key": "value", "optional": null, "another": null});
    let filtered: serde_json::Map<String, Value> = match params {
        Value::Object(map) => map.into_iter().filter(|(_, v)| !v.is_null()).collect(),
        _ => unreachable!(),
    };
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered.get("key").unwrap(), "value");
    assert!(!filtered.contains_key("optional"));
}
