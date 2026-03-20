use crate::actions;
use crate::agent::tools::base::{ExecutionContext, ToolCapabilities, ToolCategory};
use crate::agent::tools::{Tool, ToolResult};
use lru::LruCache;
use serde_json::Value;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::Mutex;

const DEFAULT_MAX_ENTRIES: usize = 32;
const DEFAULT_MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024; // 32 MB
const DEFAULT_RETRIEVE_LIMIT: usize = 50_000;

struct StashInner {
    entries: LruCache<String, String>,
    total_bytes: usize,
    next_id: usize,
}

/// In-memory LRU cache for large tool outputs that would otherwise be lost
/// to truncation. Bounded by entry count and total byte size.
pub struct ToolOutputStash {
    inner: Mutex<StashInner>,
    max_total_bytes: usize,
}

impl Default for ToolOutputStash {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolOutputStash {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(StashInner {
                entries: LruCache::new(NonZeroUsize::new(DEFAULT_MAX_ENTRIES).expect("non-zero")),
                total_bytes: 0,
                next_id: 1,
            }),
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
        }
    }

    /// Stash content and return the stash key.
    /// Returns `None` if the content exceeds the total byte budget.
    pub async fn stash(&self, content: String) -> Option<String> {
        let mut inner = self.inner.lock().await;
        let id = inner.next_id;
        inner.next_id += 1;
        let key = format!("stash_{id}");
        let content_len = content.len();

        // Single entry larger than budget cannot fit — reject
        if content_len > self.max_total_bytes {
            return None;
        }

        // Evict until we have space
        while inner.total_bytes + content_len > self.max_total_bytes {
            if let Some((_, evicted)) = inner.entries.pop_lru() {
                inner.total_bytes = inner.total_bytes.saturating_sub(evicted.len());
            } else {
                break;
            }
        }

        // Track evicted entry size from LRU push
        if let Some((_, old)) = inner.entries.push(key.clone(), content) {
            inner.total_bytes = inner.total_bytes.saturating_sub(old.len());
        }
        inner.total_bytes += content_len;

        Some(key)
    }

    /// Retrieve a slice of stashed content. Returns `(chunk, total_len)`.
    pub async fn retrieve(
        &self,
        key: &str,
        offset: usize,
        limit: usize,
    ) -> Option<(String, usize)> {
        let mut inner = self.inner.lock().await;
        let content = inner.entries.get(key)?;
        let total = content.len();
        if offset >= total {
            return Some((String::new(), total));
        }
        let end = (offset + limit).min(total);
        // Safe char-boundary slicing
        let start = content.floor_char_boundary(offset);
        let end = content.floor_char_boundary(end);
        Some((content[start..end].to_string(), total))
    }
}

/// Tool that lets the LLM retrieve stashed tool output.
pub struct StashRetrieveTool {
    stash: Arc<ToolOutputStash>,
}

impl StashRetrieveTool {
    pub fn new(stash: Arc<ToolOutputStash>) -> Self {
        Self { stash }
    }
}

#[async_trait::async_trait]
impl Tool for StashRetrieveTool {
    fn name(&self) -> &'static str {
        "stash_retrieve"
    }

    fn description(&self) -> &'static str {
        "Retrieve full tool output that was stashed due to truncation. Use when a tool result says output was stashed."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "The stash key from the truncation message (e.g. 'stash_1')"
                },
                "offset": {
                    "type": "integer",
                    "description": "Byte offset to start from (default: 0)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum bytes to return (default: 50000)"
                }
            },
            "required": ["key"]
        })
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            actions: actions![retrieve: ro],
            category: ToolCategory::Core,
            ..Default::default()
        }
    }

    fn cacheable(&self) -> bool {
        true
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        let Some(key) = params["key"].as_str() else {
            return Ok(ToolResult::error("missing 'key' parameter".to_string()));
        };
        let offset = params["offset"].as_u64().unwrap_or(0) as usize;
        let limit = params["limit"]
            .as_u64()
            .map_or(DEFAULT_RETRIEVE_LIMIT, |l| l as usize);

        match self.stash.retrieve(key, offset, limit).await {
            Some((chunk, total)) => {
                let end = offset + chunk.len();
                if end >= total {
                    Ok(ToolResult::new(chunk))
                } else {
                    Ok(ToolResult::new(format!(
                        "{chunk}\n\n[Showing {offset}..{end} of {total} bytes. Use offset={end} to continue.]"
                    )))
                }
            }
            None => Ok(ToolResult::error(format!(
                "stash key '{key}' not found (may have been evicted)"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stash_and_retrieve() {
        let stash = ToolOutputStash::new();
        let content = "hello world".repeat(100);
        let key = stash.stash(content.clone()).await.unwrap();
        assert!(key.starts_with("stash_"));

        let (chunk, total) = stash.retrieve(&key, 0, 50000).await.unwrap();
        assert_eq!(total, content.len());
        assert_eq!(chunk, content);
    }

    #[tokio::test]
    async fn test_retrieve_with_offset() {
        let stash = ToolOutputStash::new();
        let content = "abcdefghij".repeat(10); // 100 chars
        let key = stash.stash(content.clone()).await.unwrap();

        let (chunk, total) = stash.retrieve(&key, 50, 20).await.unwrap();
        assert_eq!(total, 100);
        assert_eq!(chunk.len(), 20);
        assert_eq!(chunk, &content[50..70]);
    }

    #[tokio::test]
    async fn test_retrieve_past_end() {
        let stash = ToolOutputStash::new();
        let key = stash.stash("short".to_string()).await.unwrap();

        let (chunk, total) = stash.retrieve(&key, 100, 50).await.unwrap();
        assert_eq!(total, 5);
        assert!(chunk.is_empty());
    }

    #[tokio::test]
    async fn test_oversized_returns_none() {
        let stash = ToolOutputStash::new();
        // 32 MB + 1 byte exceeds the default budget
        let oversized = "x".repeat(DEFAULT_MAX_TOTAL_BYTES + 1);
        assert!(stash.stash(oversized).await.is_none());
    }

    #[tokio::test]
    async fn test_eviction_by_count() {
        let stash = ToolOutputStash::new();
        let mut keys = Vec::new();
        for i in 0..33 {
            keys.push(stash.stash(format!("entry_{i}")).await.unwrap());
        }
        // First entry should be evicted (32 max)
        assert!(stash.retrieve(&keys[0], 0, 100).await.is_none());
        // Last entry should exist
        assert!(stash.retrieve(&keys[32], 0, 100).await.is_some());
    }

    #[tokio::test]
    async fn test_not_found() {
        let stash = ToolOutputStash::new();
        assert!(stash.retrieve("nonexistent", 0, 100).await.is_none());
    }
}
