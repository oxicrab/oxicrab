# Workspace Split Design

**Goal:** Split the single oxicrab crate into a Cargo workspace with 6 crates to improve compile times, enforce dependency boundaries, and clarify module ownership.

## Prerequisites (3 refactors before any crate split)

### 1a. Move StaticRule + DirectiveTrigger to core types

`Tool::routing_rules()` returns `Vec<StaticRule>` which depends on `DirectiveTrigger`. Move both type definitions from `src/router/` to `src/agent/tools/base/` (alongside the Tool trait that references them). The router imports them from the trait module. Matching logic stays in the router.

### 1b. Extract OAuthTokenStore trait

Define in `src/agent/tools/base/mod.rs` (or a new `src/types/oauth.rs`):
```rust
pub trait OAuthTokenStore: Send + Sync {
    fn load_token(&self, provider: &str) -> Result<Option<OAuthTokenRow>>;
    fn save_token(&self, provider: &str, row: &OAuthTokenRow) -> Result<()>;
    fn delete_token(&self, provider: &str) -> Result<()>;
}
```
Implement on `MemoryDB`. Change providers to accept `Arc<dyn OAuthTokenStore>` instead of `Arc<MemoryDB>`.

### 1c. Move provider factory out of Config

Move `Config::create_provider()`, `Config::create_routed_providers()`, and `ProvidersConfig::get_api_key_for_model()` / `get_temperature_for_model()` out of `src/config/schema/` into the binary crate (`src/provider_factory.rs` or inline in `gateway_setup.rs`). Config becomes pure data with no provider imports.

## Crate Structure

### oxicrab-core

Foundational traits, types, and safety modules. Zero dependencies on other oxicrab crates.

```
crates/oxicrab-core/src/
  lib.rs
  tools/              (Tool trait, ToolMiddleware, ToolCapabilities, ToolResult, ExecutionContext, ToolExample, StaticRule, DirectiveTrigger, OAuthTokenStore)
  providers/           (LLMProvider trait, ChatRequest, LLMResponse, Message, ToolCallRequest, ToolDefinition, ResponseFormat)
  channels/            (BaseChannel trait, split_message)
  bus/                 (InboundMessage, OutboundMessage, meta constants)
  config/              (all schema structs — Config, AgentDefaults, ChannelConfigs, etc.)
  errors/              (OxicrabError)
  safety/
    leak_detector/     (LeakDetector, Aho-Corasick + regex patterns)
    prompt_guard/      (PromptGuard, injection patterns, homoglyph transliteration)
  utils/
    http/              (default_http_client, build_pinned_client, limited_body)
    url_security/      (validate_and_resolve, SSRF protection)
    regex/             (compiled regex patterns)
    sandbox/           (Landlock/Seatbelt sandboxing)
    shell_ast/         (shell command AST analysis)
    time/              (now_ms)
    path_sanitize/     (sanitize_error_message)
    subprocess/        (scrubbed_command)
    io_safe/           (locked JSON read/write)
    media/             (MIME detection, download)
    task_tracker/      (async task lifecycle)
```

### oxicrab-memory

SQLite database layer, memory storage, embeddings. Depends on `oxicrab-core`.

```
crates/oxicrab-memory/src/
  lib.rs
  memory_db/
    mod.rs             (MemoryDB, lock_conn, OAuthTokenStore impl)
    migrations.rs      (apply_migrations, 0001_base.sql)
    search.rs          (FTS5, hybrid search, BM25)
    indexing.rs         (insert_memory, purge)
    embeddings.rs       (embedding cache, generation counter)
    cost.rs            (token logging)
    stats.rs           (complexity, intent metrics purge)
    cron.rs            (cron job persistence)
    dlq.rs             (dead letter queue)
    rss.rs             (RSS tables)
    oauth.rs           (OAuth token storage)
    pairing.rs         (DM pairing)
    obsidian.rs        (Obsidian sync)
    subagent_log.rs
    workspace.rs       (workspace file tracking)
  memory_store/        (MemoryStore wrapping MemoryDB + EmbeddingService)
  embeddings/          (EmbeddingService, cosine_similarity)
  quality/             (quality gates for memory writes)
  remember/            (fast path extraction)
  hygiene/             (startup cleanup)
```

### oxicrab-providers

LLM provider implementations. Depends on `oxicrab-core` (for LLMProvider trait + OAuthTokenStore).

```
crates/oxicrab-providers/src/
  lib.rs
  anthropic/           (API key provider)
  anthropic_oauth/     (OAuth provider)
  anthropic_common/    (shared message/tool conversion)
  openai/              (OpenAI-compatible provider)
  gemini/              (Gemini provider)
  circuit_breaker/     (CircuitBreakerProvider)
  fallback/            (FallbackProvider)
  prompt_guided/       (PromptGuidedToolsProvider)
  errors/              (ProviderErrorHandler)
  mod.rs               (session_affinity_id, warmup)
```

### oxicrab-channels

Channel implementations. Depends on `oxicrab-core` (for BaseChannel trait + bus types).

```
crates/oxicrab-channels/src/
  lib.rs
  slack/
  discord/
  telegram/
  whatsapp/
  twilio/
  manager/             (ChannelManager)
  utils/               (check_dm_access, check_group_access, exponential_backoff)
  base/                (re-export from core or local BaseChannel extras)
```

Feature flags: `channel-telegram`, `channel-discord`, `channel-slack`, `channel-whatsapp`, `channel-twilio`.

### oxicrab-gateway

HTTP API and webhook handling. Depends on `oxicrab-core` (for bus types + safety).

```
crates/oxicrab-gateway/src/
  lib.rs
  mod.rs               (build_router, chat_handler, health_handler, route_response)
  webhook.rs           (webhook_handler, apply_template, validate_webhook_signature)
  a2a/                 (Agent-to-Agent protocol)
  status.rs            (HTML status page)
```

### oxicrab (binary crate — workspace root)

Orchestration layer. Depends on all other crates.

```
src/
  main.rs
  lib.rs
  cli/                 (CLI commands, gateway_setup)
  agent/
    loop/              (AgentLoop, iteration, processing, hallucination, metadata, compaction, complexity, model_gateway, replay)
    tools/             (all 29 tool implementations + registry + setup + MCP + stash + interactive + read_only_wrapper + tool_search)
    context/           (ContextBuilder, system prompt)
    cognitive/         (CheckpointTracker)
    compaction/        (MessageCompactor)
    subagent/          (SubagentManager)
    skills/            (SkillManager)
  router/              (MessageRouter, RouterContext, rules, semantic, metrics)
  dispatch/            (ActionDispatch, DispatchContextStore)
  session/             (SessionManager, SessionStore)
  cron/                (CronService, EventMatcher)
  auth/                (Google OAuth flow)
  config/
    routing/           (ResolvedRouting — imports providers)
    credentials/       (define_credentials! macro)
    loader/            (load_config)
  pairing/             (pairing service)
  provider_factory.rs  (moved from Config — creates providers from config)
  fuzz_api.rs          (re-exports for fuzz targets)
```

## Workspace Cargo.toml

```toml
[workspace]
members = [
    "crates/oxicrab-core",
    "crates/oxicrab-memory",
    "crates/oxicrab-providers",
    "crates/oxicrab-channels",
    "crates/oxicrab-gateway",
    ".",
]
resolver = "2"

[workspace.dependencies]
# Shared dependency versions declared once, used by all crates
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
regex = "1"
async-trait = "0.1"
```

## Execution Order

All done incrementally on a single branch, one step at a time:

1. **Prereq 1a**: Move StaticRule + DirectiveTrigger types to tools/base
2. **Prereq 1b**: Extract OAuthTokenStore trait, implement on MemoryDB
3. **Prereq 1c**: Move provider factory out of Config
4. **Create workspace**: Add workspace Cargo.toml, create `crates/` directory
5. **Extract oxicrab-core**: Move traits, types, safety, utils, config schema, bus, errors
6. **Extract oxicrab-memory**: Move memory_db, memory_store, embeddings, quality, remember, hygiene
7. **Extract oxicrab-providers**: Move all providers
8. **Extract oxicrab-channels**: Move all channels with feature flags
9. **Extract oxicrab-gateway**: Move gateway + A2A
10. **Final verification**: `cargo test --workspace`, clippy, fmt, CI, fuzz targets

## Testing

- `cargo test --workspace` replaces `cargo test`
- Integration tests stay in the binary crate (they test the assembled system)
- Unit tests move with their code to the appropriate crate
- Fuzz targets update to depend on `oxicrab-core` directly instead of `fuzz_api` re-exports
- `deny.toml` and `rust-toolchain.toml` stay at workspace root

## What Does NOT Change

- No behavioral changes — this is a pure structural refactor
- No API changes for tools, providers, or channels
- Feature flags transfer to per-crate features
- CI pipeline uses `cargo test --workspace` and `cargo clippy --workspace`
- The binary crate remains the integration layer that assembles everything
