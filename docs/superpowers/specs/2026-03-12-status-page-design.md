# Status Page Design

## Overview

Add a status page to the HTTP gateway exposing runtime state: models, tools, channels, token usage, cron jobs, safety config, gateway config, and memory stats. Serves both a JSON API (`GET /api/status`) and a self-contained HTML dashboard (`GET /status`).

## Routes

| Route | Method | Auth | Rate Limited |
|---|---|---|---|
| `/api/status` | GET | API key (same as `/api/chat`) | No (exempt, like `/api/health`) |
| `/status` | GET | Public (HTML fetches JSON with stored key) | No |

## Data Sources

| Section | Source | Live/Snapshot |
|---|---|---|
| `version` | `crate::VERSION` | Static |
| `uptime_seconds` | `Instant` stored at startup | Live |
| `models` | `Config.agents.defaults.model_routing` | Snapshot |
| `tools` | Tool registry summary (names + categories) | Snapshot |
| `channels` | `Config.channels` | Snapshot |
| `tokens` | `MemoryDB::get_token_summary(today)` | Live |
| `cron` | `MemoryDB::list_cron_jobs()`, `list_dlq_entries(None)` | Live |
| `safety` | `Config` (prompt guard, exfiltration guard, sandbox) | Snapshot |
| `gateway` | `Config.gateway` (rate limit, webhooks, A2A) | Snapshot |
| `memory` | `MemoryDB` (search stats, embeddings config) | Live |

## JSON Response Shape

```json
{
  "version": "0.14.5",
  "uptime_seconds": 3621,
  "models": {
    "default": "moonshot/kimi-k2.5",
    "tasks": { "daemon": "minimax/MiniMax-M2.5", "cron": "...", "subagent": "...", "compaction": "..." },
    "fallbacks": ["openrouter/google/gemini-2.5-pro"],
    "chat_routing": { "standard": "model-a", "heavy": "model-b", "thresholds": { "standard": 0.3, "heavy": 0.65 } }
  },
  "tools": {
    "total": 28,
    "deferred": 12,
    "by_category": { "Core": ["shell", "read_file"], "Web": ["web_search", "browser"] }
  },
  "channels": {
    "telegram": true, "discord": true, "slack": true, "whatsapp": true, "twilio": true
  },
  "tokens": {
    "today": { "input": 45200, "output": 12300, "cache_read": 8000, "cache_create": 2100 },
    "by_model": [{ "model": "...", "input": 1000, "output": 500, "cache_read": 200, "cache_create": 100, "calls": 5 }]
  },
  "cron": {
    "active_jobs": 3,
    "jobs": [{ "id": "abc", "name": "daily-summary", "enabled": true, "next_run_ms": 1741784400000 }],
    "dlq_count": 0
  },
  "safety": {
    "prompt_guard": { "enabled": true, "action": "block" },
    "exfiltration_guard": false,
    "sandbox": { "enabled": true, "block_network": true }
  },
  "gateway": {
    "rate_limit": { "enabled": true, "rps": 10, "burst": 30 },
    "webhooks": ["deploy", "alerts"],
    "a2a": false
  },
  "memory": {
    "search_stats": { "total_searches": 89, "avg_results": 3.2 },
    "embeddings_enabled": true
  }
}
```

### Token query scope

`get_token_summary()` is called with today's date (UTC). The `by_model` array aggregates today's usage grouped by model. The HTML page renders this as a simple table.

### Cron next_run

`CronJobState.next_run_at_ms` (epoch millis) is used directly. For disabled jobs, `next_run_ms` is `null`. The HTML page formats this client-side.

### Embeddings enabled

Read from `Config.agents.defaults.memory.embeddings_enabled` (snapshot).

## StatusState Struct

```rust
pub struct StatusState {
    start_time: Instant,
    config_snapshot: Arc<StatusConfigSnapshot>,
    tool_snapshot: Arc<ToolSnapshot>,
    memory_db: Arc<MemoryDB>,
}
```

`StatusState` is embedded as `Option<StatusState>` inside `HttpApiState`, avoiding another parameter to `start()`.

`StatusConfigSnapshot` holds pre-extracted, secret-free config data (model routing, channel enabled flags, safety settings, gateway settings). `ToolSnapshot` holds tool names grouped by category plus deferred count, constructed by iterating `ToolRegistry` and calling `capabilities().category` on each tool.

## HTML Page

- Self-contained: inline CSS + JS, served via `include_str!` from `src/gateway/status_page.html`
- Dark theme, monospace font, card-based grid layout
- Auto-refresh every 60 seconds with countdown indicator
- API key input: on 401 response, a `prompt()` dialog asks for the key. Stored in `localStorage` as `oxicrab_api_key`, sent as `Authorization: Bearer` header on fetch. A "Change API Key" link in the header allows re-entry.
- Color coding: green (enabled/healthy), red (disabled/error), amber (warnings like DLQ > 0)

## Router Integration

`StatusState` is added as `Option<StatusState>` inside `HttpApiState`. Routes `/api/status` and `/status` are added to the authed and public route groups respectively in `build_router()`. The status JSON handler extracts `StatusState` from `HttpApiState`; if `None` (echo mode), returns `{"status": "unavailable", "mode": "echo"}`.

## Echo Mode

In echo mode (`oxicrab gateway --echo`), `StatusState` is `None` in `HttpApiState`. The HTML page shows "Status unavailable — running in echo mode." The JSON endpoint returns a minimal response with `"mode": "echo"`.

## MemoryDB Threading

All MemoryDB queries run inside a single `tokio::task::spawn_blocking()` call to avoid blocking the async runtime. The mutex is acquired once, all queries execute, then the lock is released.

## Files Changed

| File | Change |
|---|---|
| `src/gateway/status.rs` | New: `StatusState`, JSON handler, HTML handler, snapshot types |
| `src/gateway/status_page.html` | New: self-contained HTML dashboard |
| `src/gateway/mod.rs` | Add `status` module, wire routes, add `StatusState` to `HttpApiState` |
| `src/cli/commands/gateway_setup.rs` | Build `StatusState` from config + memorydb + tool registry, pass into `HttpApiState` |

## Testing

Gateway routes are tested using `tower::ServiceExt::oneshot()` pattern (per CLAUDE.md). Tests:
- Status JSON returns expected shape with `make_state()` (which has `status: None` → echo response)
- Status JSON with a populated `StatusState` returns all sections
- HTML endpoint returns `text/html` content type
- Auth gating: 401 without API key when key is configured

## Auth Model

`/api/status` uses the same API key auth as `/api/chat`. The HTML page at `/status` is public (unauthenticated) but fetches data via the authenticated JSON endpoint — the page itself contains no sensitive data, only the JS-fetched JSON does. If no API key is configured on the gateway, the JSON endpoint is also public.

## Implementation Notes

- Tool snapshot is taken once after `register_all_tools()` — tools don't change at runtime. Constructed by iterating `ToolRegistry::iter()`, calling `capabilities().category` on each, and grouping names by category.
- Config snapshot extracts only display-safe fields (no secrets, tokens, or API keys)
- `uptime_seconds` uses `std::time::Instant` (monotonic, no clock drift)
- Both status routes are exempt from rate limiting (same as `/api/health`)
