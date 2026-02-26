use super::*;
use crate::agent::tools::base::{ActionDescriptor, SubagentAccess, ToolCapabilities};

struct MockActionTool;

#[async_trait]
impl Tool for MockActionTool {
    fn name(&self) -> &'static str {
        "mock_action"
    }

    fn description(&self) -> &'static str {
        "Mock tool. Actions: read_data, write_data, delete_data."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["read_data", "write_data", "delete_data"]
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        let action = params["action"].as_str().unwrap_or("unknown");
        Ok(ToolResult::new(format!("executed: {action}")))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: vec![
                ActionDescriptor {
                    name: "read_data",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "write_data",
                    read_only: false,
                },
                ActionDescriptor {
                    name: "delete_data",
                    read_only: false,
                },
            ],
        }
    }
}

#[test]
fn test_wrapper_filters_action_enum() {
    let tool = Arc::new(MockActionTool);
    let wrapper = ReadOnlyToolWrapper::new(tool).unwrap();
    let params = wrapper.parameters();
    let actions: Vec<String> = params["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actions, vec!["read_data"]);
}

#[test]
fn test_wrapper_updates_description() {
    let tool = Arc::new(MockActionTool);
    let wrapper = ReadOnlyToolWrapper::new(tool).unwrap();
    let desc = wrapper.description();
    assert!(
        desc.contains("read_data"),
        "description should list read-only actions"
    );
    assert!(
        !desc.contains("write_data"),
        "description should not list mutating actions"
    );
}

#[tokio::test]
async fn test_wrapper_rejects_mutating_action() {
    let tool = Arc::new(MockActionTool);
    let wrapper = ReadOnlyToolWrapper::new(tool).unwrap();
    let ctx = ExecutionContext::default();
    let result = wrapper
        .execute(serde_json::json!({"action": "write_data"}), &ctx)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("not available"));
}

#[tokio::test]
async fn test_wrapper_allows_read_only_action() {
    let tool = Arc::new(MockActionTool);
    let wrapper = ReadOnlyToolWrapper::new(tool).unwrap();
    let ctx = ExecutionContext::default();
    let result = wrapper
        .execute(serde_json::json!({"action": "read_data"}), &ctx)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("executed: read_data"));
}

#[test]
fn test_wrapper_returns_none_for_all_mutating() {
    struct AllMutatingTool;

    #[async_trait]
    impl Tool for AllMutatingTool {
        fn name(&self) -> &'static str {
            "all_mutating"
        }

        fn description(&self) -> &'static str {
            "test"
        }

        fn parameters(&self) -> Value {
            serde_json::json!({})
        }

        async fn execute(&self, _: Value, _: &ExecutionContext) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::new(""))
        }

        fn capabilities(&self) -> ToolCapabilities {
            ToolCapabilities {
                built_in: true,
                network_outbound: false,
                subagent_access: SubagentAccess::ReadOnly,
                actions: vec![ActionDescriptor {
                    name: "delete",
                    read_only: false,
                }],
            }
        }
    }

    let tool = Arc::new(AllMutatingTool);
    assert!(ReadOnlyToolWrapper::new(tool).is_none());
}

#[test]
fn test_wrapper_capabilities_set_full_access() {
    let tool = Arc::new(MockActionTool);
    let wrapper = ReadOnlyToolWrapper::new(tool).unwrap();
    let caps = wrapper.capabilities();
    assert_eq!(caps.subagent_access, SubagentAccess::Full);
    assert!(caps.built_in);
}
