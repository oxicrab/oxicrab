use crate::agent::tools::base::{ExecutionContext, ToolMiddleware};
use crate::agent::tools::{Tool, ToolResult};
use crate::agent::truncation::truncate_tool_result;
use anyhow::Result;
use lru::LruCache;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

/// Produce a canonical JSON string with object keys sorted recursively.
/// This ensures cache keys are stable regardless of key insertion order.
fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, Value> =
                map.iter().map(|(k, v)| (k, canonical_value(v))).collect();
            serde_json::to_string(&sorted).unwrap_or_default()
        }
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, Value> =
                map.iter().map(|(k, v)| (k, canonical_value(v))).collect();
            Value::Object(sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect())
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonical_value).collect()),
        other => other.clone(),
    }
}

const DEFAULT_CACHE_MAX_ENTRIES: usize = 128;
const DEFAULT_CACHE_TTL_SECS: u64 = 300; // 5 minutes
const DEFAULT_MAX_RESULT_CHARS: usize = 10000;

struct CachedResult {
    result: ToolResult,
    cached_at: Instant,
}

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    middleware: Vec<Arc<dyn ToolMiddleware>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            middleware: vec![
                // Order matters: truncation runs before cache in after_execute,
                // so cached results are already truncated on cache hits.
                Arc::new(TruncationMiddleware::new(DEFAULT_MAX_RESULT_CHARS)),
                Arc::new(CacheMiddleware::new(
                    DEFAULT_CACHE_MAX_ENTRIES,
                    DEFAULT_CACHE_TTL_SECS,
                )),
                Arc::new(LoggingMiddleware),
            ],
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            tracing::warn!("tool registry: overwriting duplicate tool '{}'", name);
        }
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn get_tool_definitions(&self) -> Vec<crate::providers::base::ToolDefinition> {
        let mut defs: Vec<_> = self
            .tools
            .values()
            .map(|t| {
                let schema = t.to_schema();
                crate::providers::base::ToolDefinition {
                    name: schema["function"]["name"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    description: schema["function"]["description"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    parameters: schema["function"]["parameters"].clone(),
                }
            })
            .collect();
        defs.sort_by(|a, b| a.name.cmp(&b.name));
        defs
    }

    /// Execute a tool through the full middleware pipeline:
    /// 1. Run `before_execute` middleware (any can short-circuit with cached/precomputed result)
    /// 2. Spawn tool in `tokio::task` with timeout (panic guard)
    /// 3. Run `after_execute` middleware (truncation, caching, logging)
    pub async fn execute(
        &self,
        name: &str,
        params: Value,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Tool '{}' not found", name))?
            .clone();

        // Phase 1: before_execute middleware chain
        for mw in &self.middleware {
            if let Some(result) = mw.before_execute(name, &params, ctx, tool.as_ref()).await {
                return Ok(result);
            }
        }

        // Phase 2: Execute with timeout + panic guard
        let mut result = self
            .execute_with_guards(name, tool.clone(), params.clone(), ctx)
            .await?;

        // Phase 3: after_execute middleware chain
        for mw in &self.middleware {
            mw.after_execute(name, &params, ctx, tool.as_ref(), &mut result)
                .await;
        }

        Ok(result)
    }

    /// Execute a tool in a spawned task with timeout and panic isolation.
    async fn execute_with_guards(
        &self,
        name: &str,
        tool: Arc<dyn Tool>,
        params: Value,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult> {
        let tool_name = name.to_string();
        let ctx = ctx.clone();
        let timeout = tool.execution_timeout();
        let timeout_secs = timeout.as_secs();

        let handle = tokio::task::spawn(async move {
            tokio::time::timeout(timeout, tool.execute(params, &ctx)).await
        });

        match handle.await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                warn!("Tool '{}' timed out after {}s", tool_name, timeout_secs);
                Ok(ToolResult::error(format!(
                    "Tool '{}' timed out after {}s",
                    tool_name, timeout_secs
                )))
            }
            Err(join_err) => {
                if join_err.is_panic() {
                    error!("Tool '{}' panicked: {:?}", tool_name, join_err);
                    Ok(ToolResult::error(format!(
                        "Tool '{}' crashed unexpectedly",
                        tool_name
                    )))
                } else {
                    Err(anyhow::anyhow!("Tool '{}' was cancelled", tool_name))
                }
            }
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// --- Middleware implementations ---

/// Cache middleware — checks LRU cache before execution, stores results after.
pub struct CacheMiddleware {
    cache: Mutex<LruCache<String, CachedResult>>,
    ttl_secs: u64,
}

impl CacheMiddleware {
    pub fn new(max_entries: usize, ttl_secs: u64) -> Self {
        Self {
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(max_entries).expect("cache max_entries must be > 0"),
            )),
            ttl_secs,
        }
    }
}

#[async_trait::async_trait]
impl ToolMiddleware for CacheMiddleware {
    async fn before_execute(
        &self,
        name: &str,
        params: &Value,
        _ctx: &ExecutionContext,
        tool: &dyn Tool,
    ) -> Option<ToolResult> {
        if !tool.cacheable() {
            return None;
        }
        let cache_key = format!("{}:{}", name, canonical_json(params));
        let mut cache = self.cache.lock().await;
        if let Some(cached) = cache.get(&cache_key) {
            if cached.cached_at.elapsed().as_secs() < self.ttl_secs {
                debug!(
                    "Cache hit for tool '{}' (age: {:?})",
                    name,
                    cached.cached_at.elapsed()
                );
                return Some(cached.result.clone());
            }
            cache.pop(&cache_key);
        }
        None
    }

    async fn after_execute(
        &self,
        name: &str,
        params: &Value,
        _ctx: &ExecutionContext,
        tool: &dyn Tool,
        result: &mut ToolResult,
    ) {
        if !tool.cacheable() || result.is_error {
            return;
        }
        let cache_key = format!("{}:{}", name, canonical_json(params));
        let mut cache = self.cache.lock().await;
        cache.put(
            cache_key,
            CachedResult {
                result: result.clone(),
                cached_at: Instant::now(),
            },
        );
    }
}

/// Truncation middleware — truncates tool results to a maximum character count.
pub struct TruncationMiddleware {
    max_chars: usize,
}

impl TruncationMiddleware {
    pub fn new(max_chars: usize) -> Self {
        Self { max_chars }
    }
}

#[async_trait::async_trait]
impl ToolMiddleware for TruncationMiddleware {
    async fn after_execute(
        &self,
        _name: &str,
        _params: &Value,
        _ctx: &ExecutionContext,
        _tool: &dyn Tool,
        result: &mut ToolResult,
    ) {
        result.content = truncate_tool_result(&result.content, self.max_chars);
    }
}

/// Logging middleware — logs tool execution timing and results.
pub struct LoggingMiddleware;

#[async_trait::async_trait]
impl ToolMiddleware for LoggingMiddleware {
    async fn before_execute(
        &self,
        name: &str,
        params: &Value,
        _ctx: &ExecutionContext,
        _tool: &dyn Tool,
    ) -> Option<ToolResult> {
        debug!("Executing tool: {} with arguments: {}", name, params);
        None
    }

    async fn after_execute(
        &self,
        name: &str,
        _params: &Value,
        _ctx: &ExecutionContext,
        _tool: &dyn Tool,
        result: &mut ToolResult,
    ) {
        if result.is_error {
            warn!("Tool '{}' returned error: {}", name, result.content);
        } else {
            info!("Tool '{}' completed ({} chars)", name, result.content.len());
        }
    }
}

#[cfg(test)]
#[allow(clippy::unnecessary_literal_bound)]
mod tests {
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

        let mw = TruncationMiddleware::new(50);
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
}
