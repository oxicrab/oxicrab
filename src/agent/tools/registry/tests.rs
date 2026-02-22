use super::*;
use serde_json::json;

#[test]
fn test_canonical_json_sorts_keys() {
    // Keys in different order should produce the same canonical form
    let a = json!({"z": 1, "a": 2, "m": 3});
    let b = json!({"a": 2, "m": 3, "z": 1});
    assert_eq!(canonical_json(&a), canonical_json(&b));
}

#[test]
fn test_canonical_json_nested_objects() {
    let a = json!({"outer": {"z": 1, "a": 2}});
    let b = json!({"outer": {"a": 2, "z": 1}});
    assert_eq!(canonical_json(&a), canonical_json(&b));
}

#[test]
fn test_canonical_json_arrays_preserved() {
    // Arrays should preserve order (not sorted)
    let a = json!({"items": [3, 1, 2]});
    let b = json!({"items": [1, 2, 3]});
    assert_ne!(canonical_json(&a), canonical_json(&b));
}

#[test]
fn test_canonical_json_scalars() {
    assert_eq!(canonical_json(&json!(42)), "42");
    assert_eq!(canonical_json(&json!("hello")), "\"hello\"");
    assert_eq!(canonical_json(&json!(null)), "null");
    assert_eq!(canonical_json(&json!(true)), "true");
}

#[test]
fn test_canonical_json_nested_arrays_with_objects() {
    let a = json!({"list": [{"z": 1, "a": 2}, {"b": 3, "c": 4}]});
    let b = json!({"list": [{"a": 2, "z": 1}, {"c": 4, "b": 3}]});
    assert_eq!(canonical_json(&a), canonical_json(&b));
}

#[tokio::test]
async fn test_middleware_truncation() {
    use async_trait::async_trait;

    struct DummyTool;
    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "dummy"
        }
        fn description(&self) -> &'static str {
            "test"
        }
        fn parameters(&self) -> Value {
            json!({})
        }
        async fn execute(
            &self,
            _params: Value,
            _ctx: &ExecutionContext,
        ) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::new("ok"))
        }
    }

    let mw = TruncationMiddleware::new(200);
    let dummy = DummyTool;
    let mut result = ToolResult::new("a".repeat(500));
    mw.after_execute(
        "test",
        &json!({}),
        &ExecutionContext::default(),
        &dummy,
        &mut result,
    )
    .await;
    // The truncated content should be much shorter than the original 500 chars
    assert!(
        result.content.len() < 500,
        "Expected truncation, got {} chars",
        result.content.len()
    );
    // Should contain the truncation marker
    assert!(result.content.contains("truncated"));
}

#[tokio::test]
async fn test_middleware_cache_skips_non_cacheable() {
    use async_trait::async_trait;

    struct NonCacheableTool;
    #[async_trait]
    impl Tool for NonCacheableTool {
        fn name(&self) -> &str {
            "non_cacheable"
        }
        fn description(&self) -> &'static str {
            "test"
        }
        fn parameters(&self) -> Value {
            json!({})
        }
        async fn execute(
            &self,
            _params: Value,
            _ctx: &ExecutionContext,
        ) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::new("ok"))
        }
    }

    let cache_mw = CacheMiddleware::new(10, 300);
    let tool = NonCacheableTool;
    let params = json!({});
    let ctx = ExecutionContext::default();

    // before_execute should return None for non-cacheable tools
    let result = cache_mw
        .before_execute("non_cacheable", &params, &ctx, &tool)
        .await;
    assert!(result.is_none());

    // after_execute should NOT store results for non-cacheable tools
    let mut result = ToolResult::new("should_not_cache");
    cache_mw
        .after_execute("non_cacheable", &params, &ctx, &tool, &mut result)
        .await;
    // Verify nothing was cached
    let hit = cache_mw
        .before_execute("non_cacheable", &params, &ctx, &tool)
        .await;
    assert!(
        hit.is_none(),
        "non-cacheable tool result should not be cached"
    );
}

#[tokio::test]
async fn test_middleware_cache_returns_hit() {
    use async_trait::async_trait;

    struct CacheTestTool;
    #[async_trait]
    impl Tool for CacheTestTool {
        fn name(&self) -> &str {
            "cache_test"
        }
        fn description(&self) -> &'static str {
            "test"
        }
        fn parameters(&self) -> Value {
            json!({})
        }
        async fn execute(
            &self,
            _params: Value,
            _ctx: &ExecutionContext,
        ) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::new("ok"))
        }
        fn cacheable(&self) -> bool {
            true
        }
    }

    let cache_mw = CacheMiddleware::new(10, 300);
    let tool = CacheTestTool;
    let params = json!({"key": "val"});
    let ctx = ExecutionContext::default();

    // No cache hit initially
    assert!(
        cache_mw
            .before_execute("cache_test", &params, &ctx, &tool)
            .await
            .is_none()
    );

    // Store a result
    let mut result = ToolResult::new("cached_value");
    cache_mw
        .after_execute("cache_test", &params, &ctx, &tool, &mut result)
        .await;

    // Now should get cache hit
    let hit = cache_mw
        .before_execute("cache_test", &params, &ctx, &tool)
        .await;
    assert!(hit.is_some());
    assert_eq!(hit.unwrap().content, "cached_value");
}

#[tokio::test]
async fn test_middleware_cache_skips_errors() {
    use async_trait::async_trait;

    struct CacheErrorTool;
    #[async_trait]
    impl Tool for CacheErrorTool {
        fn name(&self) -> &str {
            "cache_err"
        }
        fn description(&self) -> &'static str {
            "test"
        }
        fn parameters(&self) -> Value {
            json!({})
        }
        async fn execute(
            &self,
            _params: Value,
            _ctx: &ExecutionContext,
        ) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::error("fail"))
        }
        fn cacheable(&self) -> bool {
            true
        }
    }

    let cache_mw = CacheMiddleware::new(10, 300);
    let tool = CacheErrorTool;
    let params = json!({});
    let ctx = ExecutionContext::default();

    // Store an error result
    let mut result = ToolResult::error("fail");
    cache_mw
        .after_execute("cache_err", &params, &ctx, &tool, &mut result)
        .await;

    // after_execute skips storing errors (is_error check), so no cache hit
    let hit = cache_mw
        .before_execute("cache_err", &params, &ctx, &tool)
        .await;
    assert!(hit.is_none());
}
