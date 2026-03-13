# Status Page Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a status dashboard to the HTTP gateway exposing models, tools, channels, tokens, cron, safety, gateway, and memory info as both JSON API and HTML page.

**Architecture:** `StatusState` struct holds config snapshot + tool snapshot + MemoryDB ref. The `status` field in `HttpApiState` uses `Arc<OnceLock<StatusState>>` — initialized empty, then set after agent setup completes. Since `HttpApiState` is cloned into the axum router, all clones share the same `Arc<OnceLock>`. Two routes: `/api/status` (JSON, auth-gated) and `/status` (HTML, public). MemoryDB queries run in `spawn_blocking`. HTML is `include_str!` from a separate file.

**Tech Stack:** Rust, axum, serde_json, tokio::task::spawn_blocking

**Spec:** `docs/superpowers/specs/2026-03-12-status-page-design.md`

---

## Chunk 1: Core Types and Snapshots

### Task 1: Add tool_registry() accessor to AgentLoop

**Files:**
- Modify: `src/agent/loop/mod.rs:476` (next to existing `memory_db()` accessor)

- [ ] **Step 1: Add the accessor method**

Add after the `memory_db()` method at line 476:

```rust
pub fn tool_registry(&self) -> Arc<ToolRegistry> {
    self.tools.clone()
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished`

- [ ] **Step 3: Commit**

```bash
git add src/agent/loop/mod.rs
git commit -m "refactor(agent): expose tool_registry() accessor"
```

### Task 2: Create status module with types and snapshot structs

**Files:**
- Create: `src/gateway/status.rs`
- Modify: `src/gateway/mod.rs:1` (add `pub mod status;`)

- [ ] **Step 1: Add module declaration**

At the top of `src/gateway/mod.rs`, after `pub mod a2a;`, add:

```rust
pub mod status;
```

- [ ] **Step 2: Create status.rs with snapshot types**

Create `src/gateway/status.rs` with the core types:

```rust
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
        use crate::config::schema::agent::TaskRouting;

        let routing = &config.agents.defaults.model_routing;

        // Extract simple task overrides (skip chat routing variant)
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
    /// Build from a ToolRegistry, grouping tool names by category.
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

        // Sort tool names within each category
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
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished`

- [ ] **Step 4: Commit**

```bash
git add src/gateway/status.rs src/gateway/mod.rs
git commit -m "feat(gateway): add status page snapshot types"
```

### Task 3: Write tests for snapshot construction

**Files:**
- Modify: `src/gateway/status.rs` (add test module)

- [ ] **Step 1: Add unit tests for ToolSnapshot and StatusConfigSnapshot**

Append to `src/gateway/status.rs`:

```rust
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
        use crate::agent::tools::base::{
            ActionDescriptor, ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory,
        };
        use crate::agent::tools::{Tool, ToolResult};
        use async_trait::async_trait;
        use serde_json::Value;

        struct FakeTool {
            tool_name: &'static str,
            cat: ToolCategory,
        }

        #[async_trait]
        impl Tool for FakeTool {
            fn name(&self) -> &'static str {
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

        // Verify no secret-like fields leaked
        assert!(!json.contains("apiKey"));
        assert!(!json.contains("token"));
        assert!(!json.contains("secret"));
        assert!(json.contains("provider/model"));
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test --lib gateway::status::tests -- -v 2>&1 | tail -15`
Expected: 3 tests pass

- [ ] **Step 3: Commit**

```bash
git add src/gateway/status.rs
git commit -m "test(gateway): add status snapshot unit tests"
```

---

## Chunk 2: JSON Handler and Router Wiring

### Task 4: Add JSON status handler

**Files:**
- Modify: `src/gateway/status.rs` (add handler function)

- [ ] **Step 1: Add the JSON handler**

Add to `src/gateway/status.rs`, after the `impl ToolSnapshot` block and before `#[cfg(test)]`:

```rust
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;

use super::HttpApiState;

/// GET /api/status — returns full system status as JSON.
pub async fn status_json_handler(State(state): State<HttpApiState>) -> impl IntoResponse {
    let Some(status) = state.status.get() else {
        return Json(serde_json::json!({
            "status": "unavailable",
            "mode": "echo",
            "version": crate::VERSION,
        }));
    };

    let uptime = status.start_time.elapsed().as_secs();
    let db = status.memory_db.clone();

    // Run all MemoryDB queries in a blocking task to avoid holding the
    // SQLite mutex on the async runtime.
    let (tokens, cron_jobs, dlq_count, search_stats) =
        tokio::task::spawn_blocking(move || {
            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
            let tokens = db.get_token_summary(&today).unwrap_or_default();
            let cron_jobs = db.list_cron_jobs(true).unwrap_or_default();
            let dlq_count = db
                .list_dlq_entries(None)
                .map(|v| v.len())
                .unwrap_or(0);
            let search_stats = db.get_search_stats().ok();
            (tokens, cron_jobs, dlq_count, search_stats)
        })
        .await
        .unwrap_or_default();

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
```

- [ ] **Step 2: Add `chrono` usage check**

`chrono` is likely already a dependency. Verify:

Run: `grep chrono Cargo.toml`
Expected: chrono listed as dependency. If not, add it.

- [ ] **Step 3: Verify it compiles**

This will fail until we wire `status` into `HttpApiState` and create `status_page.html`. That's the next task.

- [ ] **Step 4: Commit** (deferred to after Task 5 since it won't compile alone)

### Task 5: Wire StatusState into HttpApiState and router

**Files:**
- Modify: `src/gateway/mod.rs` (add `status` field to `HttpApiState`, update `build_router`)
- Create: `src/gateway/status_page.html` (placeholder)

- [ ] **Step 1: Add status field to HttpApiState**

In `src/gateway/mod.rs`, add `use std::sync::OnceLock;` to the imports, then add to the `HttpApiState` struct after the `leak_detector` field:

```rust
    /// Status page state. Uses `OnceLock` so it can be set after the router is
    /// built (tool registry is only available after agent setup, which runs in
    /// parallel with gateway startup). Empty in echo mode.
    pub(crate) status: Arc<OnceLock<status::StatusState>>,
```

- [ ] **Step 2: Update HttpApiState construction in start()**

In the `start()` function where `HttpApiState` is constructed (around line 835), add the new field. Also add a `status: Arc<OnceLock<status::StatusState>>` parameter to `start()`.

Update the `start()` signature to add `status` param after `known_secrets`:

```rust
    status: Arc<OnceLock<status::StatusState>>,
```

And in the `HttpApiState` construction:

```rust
    let state = HttpApiState {
        inbound_tx: inbound_tx.clone(),
        pending: pending.clone(),
        webhooks: Arc::new(webhook_map),
        outbound_tx,
        leak_detector: Arc::new(detector),
        status,
    };
```

- [ ] **Step 3: Add routes in build_router()**

In `build_router()`, add `/api/status` to the authed routes and `/status` to the public routes.

In the authed routes section (after the `/api/chat` route):

```rust
    let mut authed_routes = Router::new()
        .route("/api/chat", post(chat_handler))
        .route("/api/status", get(status::status_json_handler))
        .with_state(state.clone());
```

In the public routes section (after `/api/health`):

```rust
    let public_routes = Router::new()
        .route("/api/health", get(health_handler))
        .route("/status", get(status::status_html_handler))
        .route("/api/webhook/{name}", post(webhook_handler))
        .with_state(state);
```

- [ ] **Step 4: Exempt /api/status and /status from rate limiting**

In the `rate_limit_middleware` function, update the health check exemption (around line 244):

```rust
    let path = request.uri().path();
    if path == "/api/health" || path == "/api/status" || path == "/status" {
        return next.run(request).await;
    }
```

- [ ] **Step 5: Create placeholder HTML file**

Create `src/gateway/status_page.html` with minimal content:

```html
<!DOCTYPE html>
<html><head><title>Oxicrab Status</title></head>
<body><p>Loading...</p></body></html>
```

- [ ] **Step 6: Update all HttpApiState constructions in tests**

In `src/gateway/tests.rs`, update `make_state()` and `make_state_with_webhooks()` to include `status: Arc::new(OnceLock::new())`. Also update every inline `HttpApiState { ... }` in the test file — use `replace_all` on the leak_detector line to append the status field.

Use this pattern — add after `leak_detector`:

```rust
        status: Arc::new(OnceLock::new()),
```

Add `use std::sync::OnceLock;` import at the top of the test file if not already available via `use super::*`.

- [ ] **Step 7: Update gateway_setup.rs to wire status with OnceLock**

In `src/cli/commands/gateway_setup.rs`:

**Before the `tokio::join!`**, create the shared `OnceLock`:

```rust
use crate::gateway::status;
use std::sync::OnceLock;

// Shared OnceLock — set after agent setup, read by status handlers.
let status_lock = Arc::new(OnceLock::new());
```

Pass `status_lock.clone()` to `crate::gateway::start()` in `gateway_fut`.

**After the `tokio::join!`** (once agent is available):

```rust
    // Build status page state now that agent (and its tool registry) is ready
    let tool_snap = status::ToolSnapshot::from_registry(&agent.tool_registry());
    let config_snap = status::StatusConfigSnapshot::from_config(&config);
    let _ = status_lock.set(status::StatusState {
        start_time: std::time::Instant::now(),
        config_snapshot: Arc::new(config_snap),
        tool_snapshot: Arc::new(tool_snap),
        memory_db: agent.memory_db(),
    });
```

For `gateway_echo()`, create an empty `Arc::new(OnceLock::new())` and pass it — it will never be set, so handlers return the echo-mode response.

- [ ] **Step 8: Pass status_lock in both start() call sites**

In `gateway_setup.rs`, pass the `OnceLock` as the last argument to both `crate::gateway::start()` calls (in `gateway_fut` and in `gateway_echo()`).

- [ ] **Step 9: Verify it compiles**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished`

- [ ] **Step 10: Run all tests**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: All tests pass

- [ ] **Step 11: Commit**

```bash
git add src/gateway/status.rs src/gateway/mod.rs src/gateway/status_page.html src/cli/commands/gateway_setup.rs
git commit -m "feat(gateway): wire status endpoint into router"
```

### Task 6: Write tests for status JSON handler

**Files:**
- Modify: `src/gateway/tests.rs` (add handler tests)

- [ ] **Step 1: Add test for echo mode (status: None)**

```rust
#[tokio::test]
async fn test_status_json_echo_mode() {
    use axum::http::Request;
    use tower::ServiceExt;

    let state = make_state(); // status: None
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("GET")
        .uri("/api/status")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["mode"], "echo");
    assert_eq!(json["status"], "unavailable");
}
```

- [ ] **Step 2: Add test for HTML endpoint**

```rust
#[tokio::test]
async fn test_status_html_endpoint() {
    use axum::http::Request;
    use tower::ServiceExt;

    let state = make_state();
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("GET")
        .uri("/status")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.contains("text/html"));
}
```

- [ ] **Step 3: Add test for auth gating**

```rust
#[tokio::test]
async fn test_status_json_requires_auth_when_key_set() {
    use axum::http::Request;
    use tower::ServiceExt;

    let state = make_state();
    let app = build_router(state, None, Some(Arc::new("test-key".to_string())), None);

    let req = Request::builder()
        .method("GET")
        .uri("/api/status")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 4: Add test for populated StatusState**

```rust
#[tokio::test]
async fn test_status_json_with_populated_state() {
    use axum::http::Request;
    use tower::ServiceExt;

    let db = Arc::new(
        crate::agent::memory::memory_db::MemoryDB::new(":memory:")
            .expect("in-memory DB"),
    );
    let status = crate::gateway::status::StatusState {
        start_time: std::time::Instant::now(),
        config_snapshot: Arc::new(crate::gateway::status::StatusConfigSnapshot {
            models: crate::gateway::status::ModelsSnapshot {
                default: "test/model".to_string(),
                tasks: HashMap::new(),
                fallbacks: vec![],
                chat_routing: None,
            },
            channels: crate::gateway::status::ChannelsSnapshot {
                telegram: true,
                discord: false,
                slack: false,
                whatsapp: false,
                twilio: false,
            },
            safety: crate::gateway::status::SafetySnapshot {
                prompt_guard: crate::gateway::status::PromptGuardSnapshot {
                    enabled: true,
                    action: "Block".to_string(),
                },
                exfiltration_guard: false,
                sandbox: crate::gateway::status::SandboxSnapshot {
                    enabled: true,
                    block_network: true,
                },
            },
            gateway: crate::gateway::status::GatewaySnapshot {
                rate_limit: crate::gateway::status::RateLimitSnapshot {
                    enabled: false,
                    rps: 10,
                    burst: 30,
                },
                webhooks: vec![],
                a2a: false,
            },
            embeddings_enabled: false,
        }),
        tool_snapshot: Arc::new(crate::gateway::status::ToolSnapshot {
            total: 2,
            deferred: 0,
            by_category: {
                let mut m = HashMap::new();
                m.insert("Core".to_string(), vec!["shell".to_string()]);
                m
            },
        }),
        memory_db: db,
    };

    let lock = Arc::new(OnceLock::new());
    let _ = lock.set(status);

    let mut state = make_state();
    state.status = lock;
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("GET")
        .uri("/api/status")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["version"], crate::VERSION);
    assert!(json["uptime_seconds"].is_number());
    assert_eq!(json["models"]["default"], "test/model");
    assert_eq!(json["channels"]["telegram"], true);
    assert_eq!(json["tools"]["total"], 2);
    assert!(json["tokens"]["today"]["input"].is_number());
    assert!(json["cron"]["jobs"].is_array());
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --lib gateway::tests -- -v 2>&1 | tail -20`
Expected: All tests pass (including new ones)

- [ ] **Step 6: Commit**

```bash
git add src/gateway/tests.rs
git commit -m "test(gateway): add status endpoint tests"
```

---

## Chunk 3: HTML Dashboard

### Task 7: Build the HTML status page

**Files:**
- Modify: `src/gateway/status_page.html` (replace placeholder)

- [ ] **Step 1: Write the full HTML dashboard**

Replace `src/gateway/status_page.html` with the complete self-contained HTML page. Key requirements:

- Dark theme, monospace font (`JetBrains Mono` from Google Fonts with system monospace fallback)
- CSS grid layout with cards for each section
- API key input: on 401, `prompt()` for key, store in `localStorage` as `oxicrab_api_key`
- "Change API Key" link in header
- Auto-fetch every 60 seconds with countdown in header
- Color coding: green for enabled/true, red for disabled/false, amber for DLQ > 0
- Sections: Version/Uptime header, Models, Tools, Channels, Tokens, Cron, Safety, Gateway, Memory
- Token table showing by-model breakdown
- Cron jobs list with next run formatted from epoch ms
- Error state shown if fetch fails (network error, 401, etc.)

The page should be ~200-300 lines of HTML/CSS/JS. No external JS dependencies — just `fetch()` API.

Structure:
```
<style>...</style>
<div id="header">Oxicrab Status · v{version} · uptime · refresh countdown</div>
<div id="auth-bar">API Key: [Change]</div>
<div id="error" hidden>Error message</div>
<div id="grid">
  <div class="card" id="models-card">...</div>
  <div class="card" id="tools-card">...</div>
  ...
</div>
<script>
  const KEY = 'oxicrab_api_key';
  let refreshTimer;
  async function fetchStatus() { ... }
  function render(data) { ... }
  function formatUptime(secs) { ... }
  ...
  fetchStatus();
  setInterval(fetchStatus, 60000);
</script>
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished` (the HTML is included via `include_str!`)

- [ ] **Step 3: Run all tests**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: All tests pass

- [ ] **Step 4: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -5`
Expected: Clean

- [ ] **Step 5: Commit**

```bash
git add src/gateway/status_page.html
git commit -m "feat(gateway): add HTML status dashboard"
```

---

## Chunk 4: Final Verification

### Task 8: Full test suite and cleanup

- [ ] **Step 1: Run full unit test suite**

Run: `cargo test --lib 2>&1 | tail -10`
Expected: All tests pass (1792+ tests)

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -5`
Expected: Clean

- [ ] **Step 3: Run fmt check**

Run: `cargo fmt -- --check 2>&1 | tail -5`
Expected: Clean

- [ ] **Step 4: Final commit if any cleanup needed**

Only if previous steps required fixes.
