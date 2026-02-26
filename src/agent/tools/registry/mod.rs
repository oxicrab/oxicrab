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
        if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            warn!(
                "tool registry: rejecting tool with invalid name (len={}, has_control_chars={})",
                name.len(),
                name.chars().any(char::is_control)
            );
            return;
        }
        if self.tools.contains_key(&name) {
            warn!("tool registry: overwriting duplicate tool '{}'", name);
        }
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Returns a sorted list of all registered tool names.
    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort();
        names
    }

    /// Iterate over all registered tools.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Arc<dyn Tool>)> {
        self.tools.iter().map(|(k, v)| (k.as_str(), v))
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

    /// Execute a tool in a spawned `tokio::task` with timeout and panic isolation.
    ///
    /// The tool runs in a separate task so that panics are caught (via `JoinError::is_panic`)
    /// and timeouts are enforced (via `tokio::time::timeout`). Both cases return a
    /// `ToolResult::error` instead of propagating the failure, keeping the agent loop alive.
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
                    // Extract panic message for the LLM so it can avoid repeating the call.
                    // into_panic() consumes the JoinError so we must extract in one step.
                    let panic_payload = join_err.into_panic();
                    let panic_msg = panic_payload
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| panic_payload.downcast_ref::<&str>().copied())
                        .unwrap_or("unknown cause");
                    error!("Tool '{}' panicked: {}", tool_name, panic_msg);
                    Ok(ToolResult::error(format!(
                        "Tool '{}' crashed: {}",
                        tool_name, panic_msg
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
        let cache_key = format!("{}#{}:{}", name.len(), name, canonical_json(params));
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
        let cache_key = format!("{}#{}:{}", name.len(), name, canonical_json(params));
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
        ctx: &ExecutionContext,
        _tool: &dyn Tool,
    ) -> Option<ToolResult> {
        debug!(
            "Executing tool: {} (channel={}) with arguments: {}",
            name, ctx.channel, params
        );
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
mod tests;
