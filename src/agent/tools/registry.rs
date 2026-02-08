use crate::agent::tools::{Tool, ToolResult};
use anyhow::Result;
use lru::LruCache;
use serde_json::Value;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{debug, error};

const CACHE_MAX_ENTRIES: usize = 128;
const CACHE_TTL_SECS: u64 = 300; // 5 minutes

struct CachedResult {
    result: ToolResult,
    cached_at: Instant,
}

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    cache: Mutex<LruCache<String, CachedResult>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(CACHE_MAX_ENTRIES).expect("CACHE_MAX_ENTRIES must be > 0"),
            )),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn get_tool_definitions(&self) -> Vec<crate::providers::base::ToolDefinition> {
        self.tools
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
            .collect()
    }

    pub async fn execute(&self, name: &str, params: Value) -> Result<ToolResult> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Tool '{}' not found", name))?
            .clone();

        // Check cache for cacheable tools
        if tool.cacheable() {
            let cache_key = format!(
                "{}:{}",
                name,
                serde_json::to_string(&params).unwrap_or_default()
            );
            {
                let mut cache = self.cache.lock().await;
                if let Some(cached) = cache.get(&cache_key) {
                    if cached.cached_at.elapsed().as_secs() < CACHE_TTL_SECS {
                        debug!(
                            "Cache hit for tool '{}' (age: {:?})",
                            name,
                            cached.cached_at.elapsed()
                        );
                        return Ok(cached.result.clone());
                    }
                    // Expired — remove it
                    cache.pop(&cache_key);
                }
            }

            // Execute and cache the result
            let result = self.execute_with_panic_guard(name, tool, params).await?;

            if !result.is_error {
                let mut cache = self.cache.lock().await;
                cache.put(
                    cache_key,
                    CachedResult {
                        result: result.clone(),
                        cached_at: Instant::now(),
                    },
                );
            }

            Ok(result)
        } else {
            self.execute_with_panic_guard(name, tool, params).await
        }
    }

    /// Execute a tool in a spawned task to catch panics.
    async fn execute_with_panic_guard(
        &self,
        name: &str,
        tool: Arc<dyn Tool>,
        params: Value,
    ) -> Result<ToolResult> {
        let tool_name = name.to_string();
        let handle = tokio::task::spawn(async move { tool.execute(params).await });

        match handle.await {
            Ok(result) => result,
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
