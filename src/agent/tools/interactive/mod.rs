use crate::agent::tools::base::{ExecutionContext, ToolCapabilities, ToolCategory};
use crate::agent::tools::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// A button specification for interactive messages.
#[derive(Debug, Clone)]
pub struct ButtonSpec {
    pub id: String,
    pub label: String,
    pub style: String,
}

/// Shared state for pending buttons. The `add_buttons` tool writes here;
/// the agent loop reads and clears after each run.
pub type PendingButtons = Arc<Mutex<Option<Vec<ButtonSpec>>>>;

pub fn new_pending_buttons() -> PendingButtons {
    Arc::new(Mutex::new(None))
}

/// Tool that lets the LLM attach interactive buttons to its next response.
pub struct AddButtonsTool {
    pending: PendingButtons,
}

impl AddButtonsTool {
    pub fn new(pending: PendingButtons) -> Self {
        Self { pending }
    }
}

#[async_trait]
impl Tool for AddButtonsTool {
    fn name(&self) -> &str {
        "add_buttons"
    }

    fn description(&self) -> &str {
        "Attach interactive buttons to your next response message. Users can click these buttons \
         to trigger actions. Each button has an id (returned as [button:id] when clicked), \
         a label (displayed text), and an optional style (primary, danger, success, secondary)."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "buttons": {
                    "type": "array",
                    "description": "Array of button specifications",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "Unique identifier returned when clicked (e.g. 'confirm', 'cancel')"
                            },
                            "label": {
                                "type": "string",
                                "description": "Button display text"
                            },
                            "style": {
                                "type": "string",
                                "enum": ["primary", "danger", "success", "secondary"],
                                "description": "Button visual style (default: secondary)"
                            }
                        },
                        "required": ["id", "label"]
                    }
                }
            },
            "required": ["buttons"]
        })
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            category: ToolCategory::Core,
            ..Default::default()
        }
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        let buttons_arr = params["buttons"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("buttons must be an array"))?;

        if buttons_arr.is_empty() {
            return Ok(ToolResult::error("buttons array must not be empty"));
        }
        if buttons_arr.len() > 5 {
            return Ok(ToolResult::error(
                "maximum 5 buttons per message (Slack/Discord limitation)",
            ));
        }

        let mut specs = Vec::with_capacity(buttons_arr.len());
        for b in buttons_arr {
            let id = b["id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("each button must have an 'id' string"))?;
            let label = b["label"].as_str().unwrap_or(id);
            let style = b["style"].as_str().unwrap_or("secondary");
            specs.push(ButtonSpec {
                id: id.to_string(),
                label: label.to_string(),
                style: style.to_string(),
            });
        }

        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(specs);

        Ok(ToolResult::new(
            "Buttons will be attached to your next response message.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_buttons_stores_specs() {
        let pending = new_pending_buttons();
        let tool = AddButtonsTool::new(pending.clone());
        let params = serde_json::json!({
            "buttons": [
                {"id": "yes", "label": "Yes", "style": "primary"},
                {"id": "no", "label": "No", "style": "danger"}
            ]
        });
        let ctx = ExecutionContext::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(tool.execute(params, &ctx)).unwrap();
        assert!(!result.is_error);

        let specs = pending.lock().unwrap().take().unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].id, "yes");
        assert_eq!(specs[0].label, "Yes");
        assert_eq!(specs[0].style, "primary");
        assert_eq!(specs[1].id, "no");
    }

    #[test]
    fn test_pending_buttons_cleared_after_take() {
        let pending = new_pending_buttons();
        *pending.lock().unwrap() = Some(vec![ButtonSpec {
            id: "x".into(),
            label: "X".into(),
            style: "primary".into(),
        }]);
        let taken = pending.lock().unwrap().take();
        assert!(taken.is_some());
        assert!(pending.lock().unwrap().is_none());
    }

    #[test]
    fn test_add_buttons_empty_array_rejected() {
        let pending = new_pending_buttons();
        let tool = AddButtonsTool::new(pending);
        let params = serde_json::json!({"buttons": []});
        let ctx = ExecutionContext::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(tool.execute(params, &ctx)).unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn test_add_buttons_too_many_rejected() {
        let pending = new_pending_buttons();
        let tool = AddButtonsTool::new(pending);
        let params = serde_json::json!({
            "buttons": [
                {"id": "1", "label": "1"},
                {"id": "2", "label": "2"},
                {"id": "3", "label": "3"},
                {"id": "4", "label": "4"},
                {"id": "5", "label": "5"},
                {"id": "6", "label": "6"},
            ]
        });
        let ctx = ExecutionContext::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(tool.execute(params, &ctx)).unwrap();
        assert!(result.is_error);
    }
}
