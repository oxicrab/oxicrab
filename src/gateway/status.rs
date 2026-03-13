use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde::Serialize;

use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::tools::ToolRegistry;
use crate::config::Config;

use super::HttpApiState;

/// Pre-extracted config data with no secrets.
#[derive(Clone, Serialize)]
pub struct StatusConfigSnapshot {
    pub models: ModelsSnapshot,
    pub channels: ChannelsSnapshot,
    pub safety: SafetySnapshot,
    pub gateway: GatewaySnapshot,
    pub embeddings_enabled: bool,
}

#[derive(Clone, Serialize)]
pub struct ModelsSnapshot {
    pub default: String,
    pub tasks: HashMap<String, String>,
    pub fallbacks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_routing: Option<ChatRoutingSnapshot>,
}

#[derive(Clone, Serialize)]
pub struct ChatRoutingSnapshot {
    pub standard: String,
    pub heavy: String,
    pub thresholds: ChatThresholdsSnapshot,
}

#[derive(Clone, Serialize)]
pub struct ChatThresholdsSnapshot {
    pub standard: f64,
    pub heavy: f64,
}

#[derive(Clone, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct ChannelsSnapshot {
    pub telegram: bool,
    pub discord: bool,
    pub slack: bool,
    pub whatsapp: bool,
    pub twilio: bool,
}

#[derive(Clone, Serialize)]
pub struct SafetySnapshot {
    pub prompt_guard: PromptGuardSnapshot,
    pub exfiltration_guard: bool,
    pub sandbox: SandboxSnapshot,
}

#[derive(Clone, Serialize)]
pub struct PromptGuardSnapshot {
    pub enabled: bool,
    pub action: String,
}

#[derive(Clone, Serialize)]
pub struct SandboxSnapshot {
    pub enabled: bool,
    pub block_network: bool,
}

#[derive(Clone, Serialize)]
pub struct GatewaySnapshot {
    pub rate_limit: RateLimitSnapshot,
    pub webhooks: Vec<String>,
    pub a2a: bool,
}

#[derive(Clone, Serialize)]
pub struct RateLimitSnapshot {
    pub enabled: bool,
    pub rps: u32,
    pub burst: u32,
}

/// Tool names grouped by category, taken once at startup.
#[derive(Clone, Serialize)]
pub struct ToolSnapshot {
    pub total: usize,
    pub deferred: usize,
    pub by_category: HashMap<String, Vec<String>>,
}

/// Shared state for the status endpoints.
#[derive(Clone)]
pub struct StatusState {
    pub start_time: Instant,
    pub config_snapshot: Arc<StatusConfigSnapshot>,
    pub tool_snapshot: Arc<ToolSnapshot>,
    pub memory_db: Arc<MemoryDB>,
}

impl StatusConfigSnapshot {
    /// Build from Config, extracting only display-safe fields.
    pub fn from_config(config: &Config) -> Self {
        use crate::config::TaskRouting;

        let routing = &config.agents.defaults.model_routing;

        let mut tasks = HashMap::new();
        let mut chat_routing = None;
        for (key, value) in &routing.tasks {
            match value {
                TaskRouting::Model(m) => {
                    tasks.insert(key.clone(), m.clone());
                }
                TaskRouting::Chat(chat) => {
                    chat_routing = Some(ChatRoutingSnapshot {
                        standard: chat.models.standard.clone(),
                        heavy: chat.models.heavy.clone(),
                        thresholds: ChatThresholdsSnapshot {
                            standard: chat.thresholds.standard,
                            heavy: chat.thresholds.heavy,
                        },
                    });
                }
            }
        }

        let active_webhooks: Vec<String> = config
            .gateway
            .webhooks
            .iter()
            .filter(|(_, v)| v.enabled)
            .map(|(k, _)| k.clone())
            .collect();

        Self {
            models: ModelsSnapshot {
                default: routing.default.clone(),
                tasks,
                fallbacks: routing.fallbacks.clone(),
                chat_routing,
            },
            channels: ChannelsSnapshot {
                telegram: config.channels.telegram.enabled,
                discord: config.channels.discord.enabled,
                slack: config.channels.slack.enabled,
                whatsapp: config.channels.whatsapp.enabled,
                twilio: config.channels.twilio.enabled,
            },
            safety: SafetySnapshot {
                prompt_guard: PromptGuardSnapshot {
                    enabled: config.agents.defaults.prompt_guard.enabled,
                    action: format!("{}", config.agents.defaults.prompt_guard.action),
                },
                exfiltration_guard: config.tools.exfiltration_guard.enabled,
                sandbox: SandboxSnapshot {
                    enabled: config.tools.exec.sandbox.enabled,
                    block_network: config.tools.exec.sandbox.block_network,
                },
            },
            gateway: GatewaySnapshot {
                rate_limit: RateLimitSnapshot {
                    enabled: config.gateway.rate_limit.enabled,
                    rps: config.gateway.rate_limit.requests_per_second,
                    burst: config.gateway.rate_limit.burst,
                },
                webhooks: active_webhooks,
                a2a: config.gateway.a2a.enabled,
            },
            embeddings_enabled: config.agents.defaults.memory.embeddings_enabled,
        }
    }
}

impl ToolSnapshot {
    /// Build from a `ToolRegistry`, grouping tool names by category.
    pub fn from_registry(registry: &ToolRegistry) -> Self {
        let mut by_category: HashMap<String, Vec<String>> = HashMap::new();
        let mut total = 0;

        for (name, tool) in registry.iter() {
            total += 1;
            let category = format!("{:?}", tool.capabilities().category);
            by_category
                .entry(category)
                .or_default()
                .push(name.to_string());
        }

        for tools in by_category.values_mut() {
            tools.sort();
        }

        Self {
            total,
            deferred: registry.deferred_count(),
            by_category,
        }
    }
}

/// GET /api/status — returns full system status as JSON.
pub async fn status_json_handler(State(state): State<HttpApiState>) -> impl IntoResponse {
    let Some(status) = state.status.get() else {
        let mode = if state.echo_mode {
            "echo"
        } else {
            "initializing"
        };
        return Json(serde_json::json!({
            "status": "unavailable",
            "mode": mode,
            "version": crate::VERSION,
        }));
    };

    let uptime = status.start_time.elapsed().as_secs();
    let db = status.memory_db.clone();

    // Run all MemoryDB queries in a blocking task to avoid holding the
    // SQLite mutex on the async runtime.
    let db_result = tokio::task::spawn_blocking(move || {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let tokens = db.get_token_summary(&today).unwrap_or_default();
        let cron_jobs = db.list_cron_jobs(true).unwrap_or_default();
        let dlq_count = db.list_dlq_entries(None).map_or(0, |v| v.len());
        let search_stats = db.get_search_stats().ok();
        (tokens, cron_jobs, dlq_count, search_stats)
    })
    .await;

    let (tokens, cron_jobs, dlq_count, search_stats) = match db_result {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("status handler: spawn_blocking failed: {e}");
            return Json(serde_json::json!({
                "status": "error",
                "error": "db query failed",
                "version": crate::VERSION,
            }));
        }
    };

    // Aggregate today's tokens
    let mut today_input: i64 = 0;
    let mut today_output: i64 = 0;
    let mut today_cache_read: i64 = 0;
    let mut today_cache_create: i64 = 0;
    let mut by_model = Vec::new();

    for row in &tokens {
        today_input += row.total_input_tokens;
        today_output += row.total_output_tokens;
        today_cache_read += row.total_cache_read_tokens;
        today_cache_create += row.total_cache_creation_tokens;
        by_model.push(serde_json::json!({
            "model": row.model,
            "input": row.total_input_tokens,
            "output": row.total_output_tokens,
            "cache_read": row.total_cache_read_tokens,
            "cache_create": row.total_cache_creation_tokens,
            "calls": row.call_count,
        }));
    }

    // Build cron jobs array
    let active_jobs = cron_jobs.iter().filter(|j| j.enabled).count();
    let jobs: Vec<serde_json::Value> = cron_jobs
        .iter()
        .map(|j| {
            serde_json::json!({
                "id": j.id,
                "name": j.name,
                "enabled": j.enabled,
                "next_run_ms": j.state.next_run_at_ms,
            })
        })
        .collect();

    let search = search_stats.map(|s| {
        serde_json::json!({
            "total_searches": s.total_searches,
            "avg_results": s.avg_results_per_search,
        })
    });

    Json(serde_json::json!({
        "version": crate::VERSION,
        "uptime_seconds": uptime,
        "models": status.config_snapshot.models,
        "tools": status.tool_snapshot,
        "channels": status.config_snapshot.channels,
        "tokens": {
            "today": {
                "input": today_input,
                "output": today_output,
                "cache_read": today_cache_read,
                "cache_create": today_cache_create,
            },
            "by_model": by_model,
        },
        "cron": {
            "active_jobs": active_jobs,
            "jobs": jobs,
            "dlq_count": dlq_count,
        },
        "safety": status.config_snapshot.safety,
        "gateway": status.config_snapshot.gateway,
        "memory": {
            "search_stats": search,
            "embeddings_enabled": status.config_snapshot.embeddings_enabled,
        },
    }))
}

/// GET /status — serves the HTML status dashboard.
pub async fn status_html_handler() -> impl IntoResponse {
    axum::response::Html(include_str!("status_page.html"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_snapshot_empty_registry() {
        let registry = ToolRegistry::new();
        let snap = ToolSnapshot::from_registry(&registry);
        assert_eq!(snap.total, 0);
        assert_eq!(snap.deferred, 0);
        assert!(snap.by_category.is_empty());
    }

    #[test]
    fn test_tool_snapshot_groups_by_category() {
        use crate::agent::tools::base::{ExecutionContext, ToolCapabilities, ToolCategory};
        use crate::agent::tools::{Tool, ToolResult};
        use async_trait::async_trait;
        use serde_json::Value;

        struct FakeTool {
            tool_name: &'static str,
            cat: ToolCategory,
        }

        #[async_trait]
        impl Tool for FakeTool {
            fn name(&self) -> &str {
                self.tool_name
            }
            fn description(&self) -> &'static str {
                "test"
            }
            fn parameters(&self) -> Value {
                serde_json::json!({})
            }
            fn capabilities(&self) -> ToolCapabilities {
                ToolCapabilities {
                    category: self.cat,
                    ..Default::default()
                }
            }
            async fn execute(&self, _: Value, _: &ExecutionContext) -> anyhow::Result<ToolResult> {
                Ok(ToolResult::new("ok"))
            }
        }

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(FakeTool {
            tool_name: "shell",
            cat: ToolCategory::Core,
        }));
        registry.register(Arc::new(FakeTool {
            tool_name: "read_file",
            cat: ToolCategory::Core,
        }));
        registry.register(Arc::new(FakeTool {
            tool_name: "web_search",
            cat: ToolCategory::Web,
        }));

        let snap = ToolSnapshot::from_registry(&registry);
        assert_eq!(snap.total, 3);
        assert_eq!(snap.by_category["Core"], vec!["read_file", "shell"]);
        assert_eq!(snap.by_category["Web"], vec!["web_search"]);
    }

    #[test]
    fn test_config_snapshot_serializes_without_secrets() {
        let json = serde_json::to_string(&StatusConfigSnapshot {
            models: ModelsSnapshot {
                default: "provider/model".to_string(),
                tasks: HashMap::new(),
                fallbacks: vec![],
                chat_routing: None,
            },
            channels: ChannelsSnapshot {
                telegram: true,
                discord: false,
                slack: true,
                whatsapp: false,
                twilio: false,
            },
            safety: SafetySnapshot {
                prompt_guard: PromptGuardSnapshot {
                    enabled: true,
                    action: "Block".to_string(),
                },
                exfiltration_guard: false,
                sandbox: SandboxSnapshot {
                    enabled: true,
                    block_network: true,
                },
            },
            gateway: GatewaySnapshot {
                rate_limit: RateLimitSnapshot {
                    enabled: true,
                    rps: 10,
                    burst: 30,
                },
                webhooks: vec!["deploy".to_string()],
                a2a: false,
            },
            embeddings_enabled: true,
        })
        .unwrap();

        assert!(!json.contains("apiKey"));
        assert!(!json.contains("token"));
        assert!(!json.contains("secret"));
        assert!(json.contains("provider/model"));
    }
}
