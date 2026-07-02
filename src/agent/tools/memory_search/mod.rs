use crate::actions;
use crate::agent::memory::MemoryStore;
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult};
use crate::safety::LeakDetector;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
#[cfg(feature = "embeddings")]
use tracing::warn;

pub struct MemorySearchTool {
    memory: Arc<MemoryStore>,
    leak_detector: Arc<LeakDetector>,
}

impl MemorySearchTool {
    pub fn new(memory: Arc<MemoryStore>, leak_detector: Arc<LeakDetector>) -> Self {
        Self {
            memory,
            leak_detector,
        }
    }

    fn record_retrieval_metrics_for_sources<'a, I>(source_keys: I)
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut fast_hits = 0_u64;
        let mut llm_hits = 0_u64;

        for source_key in source_keys {
            if source_key.starts_with("daily:") {
                if source_key.matches(':').count() >= 2 && source_key.ends_with(":Facts") {
                    llm_hits += 1;
                } else {
                    fast_hits += 1;
                }
            }
        }

        if fast_hits > 0 {
            metrics::counter!("oxicrab_memory_remember_retrieved_total", "path" => "fast")
                .increment(fast_hits);
        }
        if llm_hits > 0 {
            metrics::counter!("oxicrab_memory_remember_retrieved_total", "path" => "llm")
                .increment(llm_hits);
        }
    }

    #[cfg(feature = "embeddings")]
    fn record_retrieval_metrics(hits: &[oxicrab_memory::memory_db::MemoryHit]) {
        Self::record_retrieval_metrics_for_sources(hits.iter().map(|hit| hit.source_key.as_str()));
    }
}

impl MemorySearchTool {
    fn action_explain_last(&self) -> Result<ToolResult> {
        let details = self.memory.db().get_last_search_details()?;
        let Some(d) = details else {
            return Ok(ToolResult::new(
                "No memory searches recorded yet.".to_string(),
            ));
        };

        let score_str = d
            .top_score
            .map_or_else(|| "n/a".to_string(), |s| format!("{s:.3}"));
        let sources = if d.source_keys.is_empty() {
            "(none)".to_string()
        } else {
            d.source_keys.join(", ")
        };

        Ok(ToolResult::new(format!(
            "Last memory search:\n\
             - Query: \"{}\"\n\
             - Method: {}\n\
             - Results: {}\n\
             - Top score: {}\n\
             - Sources: {}\n\
             - Time: {}",
            d.query, d.search_type, d.result_count, score_str, sources, d.timestamp
        )))
    }

    fn action_list_sources(&self) -> Result<ToolResult> {
        let sources = self.memory.db().list_sources_with_counts()?;
        if sources.is_empty() {
            return Ok(ToolResult::new("No memory sources found.".to_string()));
        }
        let lines: Vec<String> = sources
            .iter()
            .map(|(key, count)| format!("- {key} ({count} entries)"))
            .collect();
        Ok(ToolResult::new(format!(
            "{} sources:\n{}",
            sources.len(),
            lines.join("\n")
        )))
    }

    fn action_delete(&self, source_key: &str) -> Result<ToolResult> {
        if source_key.starts_with("knowledge:") {
            return Ok(ToolResult::error(
                "Cannot delete knowledge entries — they are protected from deletion.".to_string(),
            ));
        }
        let deleted = self.memory.db().delete_by_source_key(source_key)?;
        if deleted == 0 {
            Ok(ToolResult::new(format!(
                "No entries found for source '{source_key}'."
            )))
        } else {
            Ok(ToolResult::new(format!(
                "Deleted {deleted} entries from source '{source_key}'."
            )))
        }
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &'static str {
        "memory_search"
    }

    fn description(&self) -> &'static str {
        "Search long-term memory and daily notes. Actions: 'search' (default) finds relevant memories; 'explain_last' shows provenance details of the most recent search; 'list_sources' lists all memory source keys with counts; 'delete' removes entries by source key."
    }

    fn cacheable(&self) -> bool {
        true
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: actions![search: ro, explain_last: ro, list_sources: ro, delete],
            ..Default::default()
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "explain_last", "list_sources", "delete"],
                    "description": "Action to perform. 'search' (default) retrieves memories; 'explain_last' returns provenance of the most recent search; 'list_sources' lists all source keys with counts; 'delete' removes entries by source key."
                },
                "query": {
                    "type": "string",
                    "description": "Search query to find relevant memories. Required when action is 'search' (the default)."
                },
                "source_key": {
                    "type": "string",
                    "description": "Source key for delete action. Required when action is 'delete'."
                }
            }
        })
    }

    async fn execute(&self, params: Value, ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = params["action"].as_str().unwrap_or("search");

        if action == "explain_last" {
            return self.action_explain_last();
        }

        if action == "list_sources" {
            return self.action_list_sources();
        }

        if action == "delete" {
            let source_key = match params["source_key"].as_str() {
                Some(k) if !k.trim().is_empty() => k,
                _ => {
                    return Ok(ToolResult::error(
                        "missing or empty 'source_key' parameter for delete action".to_string(),
                    ));
                }
            };
            return self.action_delete(source_key);
        }

        let query = match params["query"].as_str() {
            Some(q) if !q.trim().is_empty() => q,
            _ => {
                return Ok(ToolResult::error(
                    "missing or empty 'query' parameter".to_string(),
                ));
            }
        };

        // Group-chat memory isolation: personal `daily:` entries must never
        // surface in a shared context. Mirror the system-prompt retrieval path
        // by excluding them from both search backends.
        let is_group = ctx
            .metadata
            .get(crate::bus::meta::IS_GROUP)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        // Use hybrid search when embeddings are available
        #[cfg(feature = "embeddings")]
        if self.memory.has_embeddings() {
            let exclude = self.memory.group_exclude_set(is_group);
            match self.memory.hybrid_search(query, 8, Some(&exclude)) {
                Ok(hits) if !hits.is_empty() => {
                    Self::record_retrieval_metrics(&hits);
                    let chunks: Vec<String> = hits
                        .iter()
                        .map(|h| format!("**{}**: {}", h.source_key, h.content))
                        .collect();
                    let result = chunks.join("\n\n---\n\n");
                    let result = oxicrab_safety::redact_memory_output(&result, &self.leak_detector);
                    return Ok(ToolResult::new(result));
                }
                Ok(_) => {} // empty, fall through to keyword search
                Err(e) => {
                    warn!("hybrid search failed, falling back to keyword: {}", e);
                }
            }
        }

        // Fallback to keyword-only search
        match self.memory.get_memory_context_scoped(Some(query), is_group) {
            Ok(context) => {
                if context.trim().is_empty() {
                    Ok(ToolResult::new(
                        "No relevant memories found for this query.".to_string(),
                    ))
                } else {
                    let details = self.memory.db().get_last_search_details()?;
                    if let Some(details) = details {
                        Self::record_retrieval_metrics_for_sources(
                            details.source_keys.iter().map(String::as_str),
                        );
                    }
                    let context =
                        oxicrab_safety::redact_memory_output(&context, &self.leak_detector);
                    Ok(ToolResult::new(context))
                }
            }
            Err(e) => Ok(ToolResult::error(format!("memory search error: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests;
