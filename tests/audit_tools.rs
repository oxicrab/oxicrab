//! Audit tests for the 10 TOOLS findings. Each test that can be reduced to a
//! reproducing case lives here as `audit_tools_NN_shortname`.
//!
//! A test is FAILING (red) when the bug is present. Tests are named
//! descriptively so the audit report lines up with the CI output.

use async_trait::async_trait;
use oxicrab::agent::tools::base::ExecutionContext;
use oxicrab::agent::tools::base::{
    ActionDescriptor, ToolCapabilities, ToolCategory, ToolConcurrency,
};
use oxicrab::agent::tools::read_only_wrapper::ReadOnlyToolWrapper;
use oxicrab::agent::tools::{Tool, ToolRegistry, ToolResult};
use serde_json::{Value, json};
use std::sync::Arc;

/// Test tool that echoes the coerced parameters back as JSON so we can
/// observe what the registry sent after coercion.
struct ParamEchoTool;

#[async_trait]
impl Tool for ParamEchoTool {
    fn name(&self) -> &str {
        "param_echo"
    }
    fn description(&self) -> &'static str {
        "Echoes params as JSON"
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "n": { "type": "number" },
            }
        })
    }
    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::new(params.to_string()))
    }
}

fn default_ctx() -> ExecutionContext {
    ExecutionContext::default()
}

/// Finding 1 (HIGH): NaN / Infinity silently pass through as strings because
/// `serde_json::Number::from_f64` returns `None` for non-finite values. The
/// registry should emit a clear validation error or a finite substitute — not
/// leave the string as a "number"-typed field.
#[tokio::test]
async fn audit_tools_01_nan_infinity_silent_skip() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ParamEchoTool));

    // String "NaN" should NOT silently survive coercion when the schema
    // expects `number`. Today it does — the param is still the string "NaN".
    let result = registry
        .execute("param_echo", json!({"n": "NaN"}), &default_ctx())
        .await
        .expect("execute");
    // The bug is present if the echoed params still contain the string "NaN"
    // (i.e. coercion silently skipped). We assert the fixed behavior: either
    // the registry returns an error, OR the coerced value is numeric.
    let echoed: Value = serde_json::from_str(&result.content).expect("json");
    assert!(
        echoed["n"].is_number() || result.is_error,
        "NaN string should either be rejected or coerced to a number, got: {echoed}"
    );

    // Same for "Infinity"
    let result = registry
        .execute("param_echo", json!({"n": "Infinity"}), &default_ctx())
        .await
        .expect("execute");
    let echoed: Value = serde_json::from_str(&result.content).expect("json");
    assert!(
        echoed["n"].is_number() || result.is_error,
        "Infinity string should either be rejected or coerced to a number, got: {echoed}"
    );
}

/// Finding 7 (MED): ReadOnlyToolWrapper runtime check rejects tools that
/// declare only metadata-only (single-purpose) read-only actions because the
/// execute path hard-requires `params["action"]`. A single-purpose read-only
/// tool cannot be exposed to subagents via the wrapper.
struct SinglePurposeReadOnlyTool;

#[async_trait]
impl Tool for SinglePurposeReadOnlyTool {
    fn name(&self) -> &str {
        "single_purpose_ro"
    }
    fn description(&self) -> &'static str {
        "A single-purpose read-only tool with no action param"
    }
    fn parameters(&self) -> Value {
        // no "action" property — single-purpose tool
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            }
        })
    }
    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            actions: vec![ActionDescriptor {
                name: "lookup",
                read_only: true,
            }],
            category: ToolCategory::Core,
            concurrency: ToolConcurrency::ReadOnly,
            ..Default::default()
        }
    }
    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        let q = params["query"].as_str().unwrap_or("");
        Ok(ToolResult::new(format!("lookup: {q}")))
    }
}

#[tokio::test]
async fn audit_tools_07_readonly_wrapper_single_purpose_drift() {
    let inner: Arc<dyn Tool> = Arc::new(SinglePurposeReadOnlyTool);
    let wrapper = ReadOnlyToolWrapper::new(inner).expect("wrapper should be constructable");

    // A single-purpose tool has no "action" param in its schema, but the
    // wrapper's execute() demands params["action"] exist. Calling with the
    // tool's actual schema yields an error — this is the drift.
    let result = wrapper
        .execute(json!({"query": "hi"}), &default_ctx())
        .await
        .expect("execute should not propagate");
    assert!(
        !result.is_error,
        "single-purpose RO tool should execute through wrapper but got error: {}",
        result.content
    );
}
