use async_trait::async_trait;
use oxicrab::agent::tools::base::ExecutionContext;
use oxicrab::agent::tools::tool_search::{ActivatedTools, ToolIndexEntry, ToolSearchTool};
use oxicrab::agent::tools::{Tool, ToolRegistry, ToolResult};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

/// A simple test tool that echoes back its parameters.
struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &'static str {
        "Echoes the input"
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" }
            },
            "required": ["text"]
        })
    }
    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        let text = params["text"].as_str().unwrap_or("no text");
        Ok(ToolResult::new(format!("Echo: {}", text)))
    }
}

/// A cacheable test tool with a call counter.
struct CacheableTool {
    call_count: Arc<std::sync::Mutex<usize>>,
}

#[async_trait]
impl Tool for CacheableTool {
    fn name(&self) -> &str {
        "cacheable_echo"
    }
    fn description(&self) -> &'static str {
        "A cacheable echo tool"
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" }
            }
        })
    }
    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        *self.call_count.lock().expect("lock call count") += 1;
        let text = params["text"].as_str().unwrap_or("no text");
        Ok(ToolResult::new(format!("Cached: {}", text)))
    }
    fn cacheable(&self) -> bool {
        true
    }
}

/// A tool that always panics (for testing panic isolation).
struct PanicTool;

#[async_trait]
impl Tool for PanicTool {
    fn name(&self) -> &str {
        "panic_tool"
    }
    fn description(&self) -> &'static str {
        "A tool that panics"
    }
    fn parameters(&self) -> Value {
        json!({"type": "object"})
    }
    async fn execute(&self, _params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        panic!("Intentional panic for testing");
    }
}

/// A tool that returns an error result.
struct ErrorTool;

#[async_trait]
impl Tool for ErrorTool {
    fn name(&self) -> &str {
        "error_tool"
    }
    fn description(&self) -> &'static str {
        "A tool that returns an error"
    }
    fn parameters(&self) -> Value {
        json!({"type": "object"})
    }
    async fn execute(&self, _params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::error("Something went wrong".to_string()))
    }
}

fn default_ctx() -> ExecutionContext {
    ExecutionContext::default()
}

#[tokio::test]
async fn test_register_and_execute() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));

    let result = registry
        .execute("echo", json!({"text": "hello world"}), &default_ctx())
        .await
        .expect("execute echo tool");

    assert_eq!(result.content, "Echo: hello world");
    assert!(!result.is_error);
}

#[tokio::test]
async fn test_execute_unknown_tool() {
    let registry = ToolRegistry::new();
    let result = registry
        .execute("nonexistent", json!({}), &default_ctx())
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[tokio::test]
async fn test_get_tool_definitions() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));

    let defs = registry.get_tool_definitions();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name, "echo");
    assert_eq!(defs[0].description, "Echoes the input");
}

#[tokio::test]
async fn test_cache_hit_same_params() {
    let call_count = Arc::new(std::sync::Mutex::new(0usize));
    let tool = CacheableTool {
        call_count: call_count.clone(),
    };

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(tool));

    let params = json!({"text": "cached value"});

    // First call - should execute
    let result1 = registry
        .execute("cacheable_echo", params.clone(), &default_ctx())
        .await
        .expect("execute cacheable tool first call");
    assert_eq!(result1.content, "Cached: cached value");
    assert_eq!(*call_count.lock().expect("lock call count"), 1);

    // Second call with same params - should hit cache
    let result2 = registry
        .execute("cacheable_echo", params, &default_ctx())
        .await
        .expect("execute cacheable tool second call");
    assert_eq!(result2.content, "Cached: cached value");
    // Call count should still be 1 (cached)
    assert_eq!(*call_count.lock().expect("lock call count"), 1);
}

#[tokio::test]
async fn test_cache_miss_different_params() {
    let call_count = Arc::new(std::sync::Mutex::new(0usize));
    let tool = CacheableTool {
        call_count: call_count.clone(),
    };

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(tool));

    // First call
    registry
        .execute("cacheable_echo", json!({"text": "a"}), &default_ctx())
        .await
        .expect("execute cacheable tool first call");
    assert_eq!(*call_count.lock().expect("lock call count"), 1);

    // Second call with different params - should miss cache
    registry
        .execute("cacheable_echo", json!({"text": "b"}), &default_ctx())
        .await
        .expect("execute cacheable tool second call");
    assert_eq!(*call_count.lock().expect("lock call count"), 2);
}

#[tokio::test]
async fn test_non_cacheable_always_executes() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));

    let params = json!({"text": "test"});

    // EchoTool is not cacheable, so both calls should execute
    let r1 = registry
        .execute("echo", params.clone(), &default_ctx())
        .await
        .expect("execute echo first call");
    let r2 = registry
        .execute("echo", params, &default_ctx())
        .await
        .expect("execute echo second call");
    assert_eq!(r1.content, r2.content);
    // Both executed (no cache)
}

#[tokio::test]
async fn test_panic_caught_gracefully() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(PanicTool));

    // Should not propagate the panic — should return an error ToolResult
    let result = registry
        .execute("panic_tool", json!({}), &default_ctx())
        .await
        .expect("execute panic tool");
    assert!(result.is_error);
    assert!(result.content.contains("crashed"));
}

#[tokio::test]
async fn test_error_result_not_cached() {
    let mut registry = ToolRegistry::new();

    // Create a cacheable tool that returns errors
    struct CacheableErrorTool {
        call_count: Arc<std::sync::Mutex<usize>>,
    }

    #[async_trait]
    impl Tool for CacheableErrorTool {
        fn name(&self) -> &str {
            "cacheable_error"
        }
        fn description(&self) -> &'static str {
            "Cacheable tool that errors"
        }
        fn parameters(&self) -> Value {
            json!({"type": "object"})
        }
        async fn execute(
            &self,
            _params: Value,
            _ctx: &ExecutionContext,
        ) -> anyhow::Result<ToolResult> {
            *self.call_count.lock().expect("lock call count") += 1;
            Ok(ToolResult::error("Transient failure".to_string()))
        }
        fn cacheable(&self) -> bool {
            true
        }
    }

    let call_count = Arc::new(std::sync::Mutex::new(0usize));
    let tool = CacheableErrorTool {
        call_count: call_count.clone(),
    };
    registry.register(Arc::new(tool));

    // First call - error result
    let r1 = registry
        .execute("cacheable_error", json!({}), &default_ctx())
        .await
        .expect("execute cacheable error tool first call");
    assert!(r1.is_error);
    assert_eq!(*call_count.lock().expect("lock call count"), 1);

    // Second call - should NOT be cached (errors aren't cached)
    let r2 = registry
        .execute("cacheable_error", json!({}), &default_ctx())
        .await
        .expect("execute cacheable error tool second call");
    assert!(r2.is_error);
    assert_eq!(*call_count.lock().expect("lock call count"), 2);
}

#[tokio::test]
async fn test_error_tool_returns_error_result() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ErrorTool));

    let result = registry
        .execute("error_tool", json!({}), &default_ctx())
        .await
        .expect("execute error tool");
    assert!(result.is_error);
    assert!(
        result.content.starts_with("Something went wrong"),
        "error content should start with tool error message, got: {}",
        result.content
    );
    // Schema hint injection appends description + parameters to error results
    assert!(result.content.contains("Tool description:"));
}

#[tokio::test]
async fn test_multiple_tools_registered() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    registry.register(Arc::new(ErrorTool));
    registry.register(Arc::new(PanicTool));

    let defs = registry.get_tool_definitions();
    assert_eq!(defs.len(), 3);

    // Each tool should be individually accessible
    assert!(registry.get("echo").is_some());
    assert!(registry.get("error_tool").is_some());
    assert!(registry.get("panic_tool").is_some());
    assert!(registry.get("nonexistent").is_none());
}

/// A tool that requires an integer parameter, used to test auto-casting.
struct IntegerParamTool;

#[async_trait]
impl Tool for IntegerParamTool {
    fn name(&self) -> &str {
        "integer_tool"
    }
    fn description(&self) -> &'static str {
        "A tool with an integer parameter"
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "count": { "type": "integer", "description": "A count value" }
            },
            "required": ["count"]
        })
    }
    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        // This will fail if the param wasn't auto-cast from string to integer
        let count = params["count"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("count is not an integer"))?;
        Ok(ToolResult::new(format!("Count: {count}")))
    }
}

#[tokio::test]
async fn test_tool_param_auto_casting() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(IntegerParamTool));

    // Pass a string "42" where the schema expects an integer.
    // The registry's coerce_params_to_schema should auto-cast it.
    let result = registry
        .execute("integer_tool", json!({"count": "42"}), &default_ctx())
        .await
        .expect("execute integer_tool with string param");

    assert!(!result.is_error, "auto-casting should prevent error");
    assert!(
        result.content.contains("Count: 42"),
        "expected 'Count: 42', got: {}",
        result.content
    );
}

/// A simple tool used for deferred registration testing.
struct DeferredEchoTool;

#[async_trait]
impl Tool for DeferredEchoTool {
    fn name(&self) -> &str {
        "deferred_echo"
    }
    fn description(&self) -> &'static str {
        "A deferred echo tool for testing"
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" }
            }
        })
    }
    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        let text = params["text"].as_str().unwrap_or("no text");
        Ok(ToolResult::new(format!("Deferred: {text}")))
    }
}

#[tokio::test]
async fn test_deferred_tool_activation_flow() {
    let activated = ActivatedTools::new();

    // Build an index for tool_search
    let index = vec![
        ToolIndexEntry {
            name: "echo".into(),
            description: "Echoes the input".into(),
            deferred: false,
        },
        ToolIndexEntry {
            name: "deferred_echo".into(),
            description: "A deferred echo tool for testing".into(),
            deferred: true,
        },
    ];

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    registry.register_deferred(Arc::new(DeferredEchoTool));
    registry.register(Arc::new(ToolSearchTool::new(index, activated.clone())));

    // Step 1: Deferred tool should NOT appear in default definitions
    let defs = registry.get_tool_definitions();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(
        !names.contains(&"deferred_echo"),
        "deferred tool should be excluded from definitions"
    );
    assert!(
        names.contains(&"echo"),
        "regular tool should be in definitions"
    );
    assert!(
        names.contains(&"tool_search"),
        "tool_search should be in definitions"
    );

    // Step 2: Use tool_search to discover the deferred tool
    let request_id = "test-req-1";
    let search_ctx = ExecutionContext {
        metadata: HashMap::from([(
            "request_id".to_string(),
            Value::String(request_id.to_string()),
        )]),
        ..ExecutionContext::default()
    };
    let search_result = registry
        .execute("tool_search", json!({"query": "deferred"}), &search_ctx)
        .await
        .expect("tool_search should succeed");
    assert!(
        !search_result.is_error,
        "tool_search should not return error"
    );
    assert!(
        search_result.content.contains("deferred_echo"),
        "search should find deferred_echo"
    );

    // Step 3: Deferred tool should now be activated for this request
    let activated_set = activated.snapshot(request_id).await;
    assert!(
        activated_set.contains("deferred_echo"),
        "deferred_echo should be activated"
    );

    // Step 4: Definitions with activated set should include the deferred tool
    let defs = registry.get_tool_definitions_with_activated(&activated_set);
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(
        names.contains(&"deferred_echo"),
        "activated deferred tool should appear in definitions"
    );

    // Step 5: Execute the deferred tool (should work even before activation)
    let result = registry
        .execute("deferred_echo", json!({"text": "hello"}), &default_ctx())
        .await
        .expect("execute deferred_echo");
    assert!(!result.is_error);
    assert!(result.content.contains("Deferred: hello"));
}
