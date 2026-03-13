use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;

use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::tools::ToolRegistry;
use crate::config::Config;

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
                    action: format!("{:?}", config.agents.defaults.prompt_guard.action),
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
