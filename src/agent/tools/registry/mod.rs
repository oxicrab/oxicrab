use crate::agent::tools::base::{ExecutionContext, ToolMiddleware};
use crate::agent::tools::{Tool, ToolResult};
use crate::agent::truncation::truncate_tool_result;
use anyhow::Result;
use lru::LruCache;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

/// Produce a canonical JSON string with object keys sorted recursively.
/// This ensures cache keys are stable regardless of key insertion order.
fn canonical_json(value: &Value) -> String {
    // Fast path: flat objects with few keys skip the expensive BTreeMap sort.
    // serde_json::Map preserves insertion order deterministically for identical
    // JSON inputs, so sorting is only needed for complex nested structures.
    if let Value::Object(map) = value {
        if map.len() <= 8 && map.values().all(|v| !v.is_object()) {
            return serde_json::to_string(value).unwrap_or_default();
        }
    }
    // Full recursive sort path for complex nested objects
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

/// Coerce LLM-provided parameter values to match the JSON Schema types declared
/// by a tool. LLMs frequently return `"5"` when a schema expects a number, or
/// `5` when a schema expects a string. This auto-casting avoids wasting a full
/// LLM round-trip on trivially fixable type mismatches.
fn coerce_params_to_schema(mut params: Value, schema: &Value) -> Value {
    let Some(Value::Object(properties)) = schema.get("properties") else {
        return params;
    };
    let Some(params_obj) = params.as_object_mut() else {
        return params;
    };

    for (key, prop_schema) in properties {
        let Some(expected_type) = prop_schema.get("type").and_then(Value::as_str) else {
            continue;
        };
        let Some(value) = params_obj.get_mut(key) else {
            continue;
        };

        match expected_type {
            "integer" if value.is_string() => {
                // "5" → 5
                if let Some(s) = value.as_str()
                    && let Ok(n) = s.parse::<i64>()
                {
                    *value = Value::Number(n.into());
                }
            }
            "number" if value.is_string() => {
                // "3.14" → 3.14
                if let Some(s) = value.as_str()
                    && let Ok(n) = s.parse::<f64>()
                    && let Some(num) = serde_json::Number::from_f64(n)
                {
                    *value = Value::Number(num);
                }
            }
            "string" if value.is_number() => {
                // 5 → "5"
                *value = Value::String(value.to_string());
            }
            "boolean" if value.is_string() => {
                // "true" → true, "false" → false
                match value.as_str() {
                    Some("true") => *value = Value::Bool(true),
                    Some("false") => *value = Value::Bool(false),
                    _ => {}
                }
            }
            "array" | "object" if value.is_string() => {
                // "{\"a\":1}" → {"a":1}, "[1,2]" → [1,2]
                if let Some(s) = value.as_str()
                    && let Ok(parsed) = serde_json::from_str::<Value>(s)
                    && ((expected_type == "array" && parsed.is_array())
                        || (expected_type == "object" && parsed.is_object()))
                {
                    *value = parsed;
                }
            }
            _ => {}
        }
    }

    params
}

struct CachedResult {
    result: ToolResult,
    cached_at: Instant,
}

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    middleware: Vec<Arc<dyn ToolMiddleware>>,
    /// Tools whose schemas are excluded from LLM requests until activated
    /// via `tool_search`. Execution still works for all registered tools.
    deferred: HashSet<String>,
    /// Per-tool definition cache: computed once at registration time, never changes.
    definition_cache: HashMap<String, crate::providers::base::ToolDefinition>,
    /// Cached sorted+filtered tool definitions list (invalidated on `register`).
    /// Only used when no deferred tools have been activated (the common path).
    cached_definitions: std::sync::Mutex<Option<Vec<crate::providers::base::ToolDefinition>>>,
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
            deferred: HashSet::new(),
            definition_cache: HashMap::new(),
            cached_definitions: std::sync::Mutex::new(None),
        }
    }

    /// Create a registry with a tool output stash for recovering truncated content.
    pub fn with_stash(stash: Arc<crate::agent::tools::stash::ToolOutputStash>) -> Self {
        Self {
            tools: HashMap::new(),
            middleware: vec![
                Arc::new(TruncationMiddleware::with_stash(
                    DEFAULT_MAX_RESULT_CHARS,
                    stash,
                )),
                Arc::new(CacheMiddleware::new(
                    DEFAULT_CACHE_MAX_ENTRIES,
                    DEFAULT_CACHE_TTL_SECS,
                )),
                Arc::new(LoggingMiddleware),
            ],
            deferred: HashSet::new(),
            definition_cache: HashMap::new(),
            cached_definitions: std::sync::Mutex::new(None),
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
        // Compute and cache the definition once at registration time
        let definition = Self::tool_to_definition(&tool);
        self.definition_cache.insert(name.clone(), definition);
        self.tools.insert(name, tool);
        self.cached_definitions.lock().unwrap().take();
    }

    /// Register a tool whose schema is hidden from LLM requests until
    /// activated via `tool_search`. The tool can still be executed.
    pub fn register_deferred(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        self.deferred.insert(name.clone());
        self.register(tool);
    }

    /// Check if a tool is deferred (schema hidden from LLM by default).
    pub fn is_deferred(&self, name: &str) -> bool {
        self.deferred.contains(name)
    }

    /// Number of deferred tools.
    pub fn deferred_count(&self) -> usize {
        self.deferred.len()
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
        self.get_tool_definitions_with_activated(&HashSet::new())
    }

    /// Get tool definitions, including deferred tools that have been activated.
    /// Uses a cached copy when no deferred tools are activated (the common path).
    pub fn get_tool_definitions_with_activated(
        &self,
        activated: &HashSet<String>,
    ) -> Vec<crate::providers::base::ToolDefinition> {
        // Use cache only when no deferred tools activated (common case)
        if activated.is_empty() {
            let mut cache = self.cached_definitions.lock().unwrap();
            if let Some(ref defs) = *cache {
                return defs.clone();
            }
            let defs = self.build_all_definitions(activated);
            *cache = Some(defs.clone());
            return defs;
        }
        // Deferred tools activated — rebuild (rare path, don't cache)
        self.build_all_definitions(activated)
    }

    fn build_all_definitions(
        &self,
        activated: &HashSet<String>,
    ) -> Vec<crate::providers::base::ToolDefinition> {
        let mut defs: Vec<_> = self
            .tools
            .keys()
            .filter(|name| !self.deferred.contains(*name) || activated.contains(*name))
            .filter_map(|name| self.definition_cache.get(name).cloned())
            .collect();
        defs.sort_by(|a, b| a.name.cmp(&b.name));
        defs
    }

    /// Get tool definitions filtered to only tools in the given categories.
    pub fn get_filtered_definitions(
        &self,
        categories: &[crate::agent::tools::base::ToolCategory],
    ) -> Vec<crate::providers::base::ToolDefinition> {
        self.get_filtered_definitions_with_activated(categories, &HashSet::new())
    }

    /// Get filtered definitions, including activated deferred tools in matching categories.
    pub fn get_filtered_definitions_with_activated(
        &self,
        categories: &[crate::agent::tools::base::ToolCategory],
        activated: &HashSet<String>,
    ) -> Vec<crate::providers::base::ToolDefinition> {
        let mut defs: Vec<_> = self
            .tools
            .iter()
            .filter(|(name, t)| {
                let in_category = categories.contains(&t.capabilities().category);
                let visible = !self.deferred.contains(*name) || activated.contains(*name);
                in_category && visible
            })
            .filter_map(|(name, _)| self.definition_cache.get(name).cloned())
            .collect();
        defs.sort_by(|a, b| a.name.cmp(&b.name));
        defs
    }

    fn tool_to_definition(t: &Arc<dyn Tool>) -> crate::providers::base::ToolDefinition {
        let schema = t.to_schema();
        crate::providers::base::ToolDefinition {
            name: schema["function"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            description: schema["function"]["description"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            parameters: schema["function"]["parameters"].clone(),
        }
    }

    /// When a tool returns an error, append its description and parameter schema
    /// as a hint so the LLM can self-correct without needing the full schema in
    /// every request. Caps: 500 char description, 3000 char schema.
    fn inject_schema_hint(tool: &dyn Tool, result: &mut ToolResult) {
        use std::fmt::Write as _;

        let desc = tool.description();
        let desc_capped = if desc.len() > 500 { &desc[..500] } else { desc };
        let schema = serde_json::to_string_pretty(&tool.parameters()).unwrap_or_default();
        let schema_capped = if schema.len() > 3000 {
            &schema[..schema.floor_char_boundary(3000)]
        } else {
            &schema
        };

        let _ = write!(
            result.content,
            "\n\nTool description: {desc_capped}\nExpected parameters:\n{schema_capped}"
        );
    }

    /// Execute a tool through the full middleware pipeline:
    /// 1. Coerce parameters to match schema types (auto-cast string↔number, etc.)
    /// 2. Run `before_execute` middleware (any can short-circuit with cached/precomputed result)
    /// 3. Spawn tool in `tokio::task` with timeout (panic guard)
    /// 4. Run `after_execute` middleware (truncation, caching, logging)
    /// 5. On error, inject schema hint
    pub async fn execute(
        &self,
        name: &str,
        params: Value,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Tool '{name}' not found"))?
            .clone();

        // Phase 0: Coerce LLM params to match schema types
        let params = coerce_params_to_schema(params, &tool.parameters());

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

        // Phase 4: On error, inject schema hint so the LLM learns the correct usage.
        // Especially useful for deferred/MCP tools whose schemas the LLM may not have seen.
        if result.is_error {
            Self::inject_schema_hint(tool.as_ref(), &mut result);
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
                    "Tool '{tool_name}' timed out after {timeout_secs}s"
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
                    error!("Tool '{tool_name}' panicked: {panic_msg}");
                    // Return generic message to LLM (details stay in error log)
                    Ok(ToolResult::error(format!(
                        "Tool '{tool_name}' crashed unexpectedly"
                    )))
                } else {
                    Err(anyhow::anyhow!("Tool '{tool_name}' was cancelled"))
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
/// When a stash is provided, large outputs are saved before truncation so the
/// LLM can retrieve them via `stash_retrieve`.
pub struct TruncationMiddleware {
    max_chars: usize,
    stash: Option<Arc<crate::agent::tools::stash::ToolOutputStash>>,
}

impl TruncationMiddleware {
    pub fn new(max_chars: usize) -> Self {
        Self {
            max_chars,
            stash: None,
        }
    }

    pub fn with_stash(
        max_chars: usize,
        stash: Arc<crate::agent::tools::stash::ToolOutputStash>,
    ) -> Self {
        Self {
            max_chars,
            stash: Some(stash),
        }
    }
}

#[async_trait::async_trait]
impl ToolMiddleware for TruncationMiddleware {
    async fn after_execute(
        &self,
        name: &str,
        _params: &Value,
        _ctx: &ExecutionContext,
        _tool: &dyn Tool,
        result: &mut ToolResult,
    ) {
        use std::fmt::Write as _;

        // Skip truncation for stash retrieval — the LLM explicitly asked for it
        if name == "stash_retrieve" {
            return;
        }

        let raw_len = result.content.len();
        // Stash + truncate when content exceeds limit and stash is available
        if raw_len > self.max_chars
            && let Some(ref stash) = self.stash
            && let Some(key) = stash.stash(result.content.clone()).await
        {
            result.content = truncate_tool_result(&result.content, self.max_chars);
            let _ = write!(
                result.content,
                "\n\n[Full output ({raw_len} chars) stashed as '{key}'. Use stash_retrieve tool to access.]"
            );
            return;
        }

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
