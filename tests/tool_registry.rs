use async_trait::async_trait;
use nanobot::agent::tools::{Tool, ToolRegistry, ToolResult};
use serde_json::{json, Value};
use std::sync::Arc;

/// A simple test tool that echoes back its parameters.
struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
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
    async fn execute(&self, params: Value) -> anyhow::Result<ToolResult> {
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
    fn description(&self) -> &str {
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
    async fn execute(&self, params: Value) -> anyhow::Result<ToolResult> {
        *self.call_count.lock().unwrap() += 1;
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
    fn description(&self) -> &str {
        "A tool that panics"
    }
    fn parameters(&self) -> Value {
        json!({"type": "object"})
    }
    async fn execute(&self, _params: Value) -> anyhow::Result<ToolResult> {
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
    fn description(&self) -> &str {
        "A tool that returns an error"
    }
    fn parameters(&self) -> Value {
        json!({"type": "object"})
    }
    async fn execute(&self, _params: Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::error("Something went wrong".to_string()))
    }
}

#[tokio::test]
async fn test_register_and_execute() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));

    let result = registry
        .execute("echo", json!({"text": "hello world"}))
        .await
        .unwrap();

    assert_eq!(result.content, "Echo: hello world");
    assert!(!result.is_error);
}

#[tokio::test]
async fn test_execute_unknown_tool() {
    let registry = ToolRegistry::new();
    let result = registry.execute("nonexistent", json!({})).await;
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
        .execute("cacheable_echo", params.clone())
        .await
        .unwrap();
    assert_eq!(result1.content, "Cached: cached value");
    assert_eq!(*call_count.lock().unwrap(), 1);

    // Second call with same params - should hit cache
    let result2 = registry.execute("cacheable_echo", params).await.unwrap();
    assert_eq!(result2.content, "Cached: cached value");
    // Call count should still be 1 (cached)
    assert_eq!(*call_count.lock().unwrap(), 1);
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
        .execute("cacheable_echo", json!({"text": "a"}))
        .await
        .unwrap();
    assert_eq!(*call_count.lock().unwrap(), 1);

    // Second call with different params - should miss cache
    registry
        .execute("cacheable_echo", json!({"text": "b"}))
        .await
        .unwrap();
    assert_eq!(*call_count.lock().unwrap(), 2);
}

#[tokio::test]
async fn test_non_cacheable_always_executes() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));

    let params = json!({"text": "test"});

    // EchoTool is not cacheable, so both calls should execute
    let r1 = registry.execute("echo", params.clone()).await.unwrap();
    let r2 = registry.execute("echo", params).await.unwrap();
    assert_eq!(r1.content, r2.content);
    // Both executed (no cache)
}

#[tokio::test]
async fn test_panic_caught_gracefully() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(PanicTool));

    // Should not propagate the panic — should return an error ToolResult
    let result = registry.execute("panic_tool", json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("crashed unexpectedly"));
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
        fn description(&self) -> &str {
            "Cacheable tool that errors"
        }
        fn parameters(&self) -> Value {
            json!({"type": "object"})
        }
        async fn execute(&self, _params: Value) -> anyhow::Result<ToolResult> {
            *self.call_count.lock().unwrap() += 1;
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
        .execute("cacheable_error", json!({}))
        .await
        .unwrap();
    assert!(r1.is_error);
    assert_eq!(*call_count.lock().unwrap(), 1);

    // Second call - should NOT be cached (errors aren't cached)
    let r2 = registry
        .execute("cacheable_error", json!({}))
        .await
        .unwrap();
    assert!(r2.is_error);
    assert_eq!(*call_count.lock().unwrap(), 2);
}

#[tokio::test]
async fn test_error_tool_returns_error_result() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ErrorTool));

    let result = registry.execute("error_tool", json!({})).await.unwrap();
    assert!(result.is_error);
    assert_eq!(result.content, "Something went wrong");
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
