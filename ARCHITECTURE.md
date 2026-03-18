# Architecture

Multi-channel AI assistant that connects chat platforms to LLM providers via an agent loop.

## Workspace Structure

The codebase is a Cargo workspace with 14 library crates and 1 binary crate:

| Crate | Path | Purpose |
|-------|------|---------|
| `oxicrab-core` | `crates/oxicrab-core/` | Traits (Tool, LLMProvider, BaseChannel), config schema, bus types, errors, shared utils |
| `oxicrab-memory` | `crates/oxicrab-memory/` | SQLite/FTS5, embeddings, sessions, quality gates, hygiene |
| `oxicrab-providers` | `crates/oxicrab-providers/` | Anthropic, OpenAI, Gemini, circuit breaker, fallback |
| `oxicrab-channels` | `crates/oxicrab-channels/` | Slack, Discord, Telegram, WhatsApp, Twilio |
| `oxicrab-gateway` | `crates/oxicrab-gateway/` | HTTP API, webhooks, A2A, rate limiting |
| `oxicrab-router` | `crates/oxicrab-router/` | Message router, context state machine, semantic filtering |
| `oxicrab-safety` | `crates/oxicrab-safety/` | LeakDetector, PromptGuard |
| `oxicrab-tools-web` | `crates/oxicrab-tools-web/` | HttpTool, RedditTool, WebSearchTool, WebFetchTool |
| `oxicrab-tools-api` | `crates/oxicrab-tools-api/` | GitHubTool, WeatherTool, TodoistTool, MediaTool, ImageGenTool |
| `oxicrab-tools-google` | `crates/oxicrab-tools-google/` | Gmail, Calendar, Tasks + Google OAuth |
| `oxicrab-tools-system` | `crates/oxicrab-tools-system/` | Filesystem, Exec, Tmux |
| `oxicrab-tools-browser` | `crates/oxicrab-tools-browser/` | BrowserTool |
| `oxicrab-tools-obsidian` | `crates/oxicrab-tools-obsidian/` | ObsidianTool + sync cache |
| `oxicrab-tools-rss` | `crates/oxicrab-tools-rss/` | RSS feed reader with LinTS ranking model |
| `oxicrab` | `.` (root) | Agent loop, CLI, CronTool, MCP, tool registry, wiring |

The binary crate (`src/`) has thin re-export stubs for moved modules (`src/providers/mod.rs`, `src/channels/mod.rs`, etc.) so internal `crate::` paths continue working.

## Core Flow

```
Channel (Telegram/Discord/Slack/WhatsApp/Twilio)
  → MessageBus (inbound queue)
    → AgentLoop (iterates: LLM call → tool execution → repeat)
      → MessageBus (outbound queue)
        → Channel (reply)
```

## Key Abstractions (3 traits + middleware)

- **`Tool`** (`crates/oxicrab-core/src/tools/base/mod.rs`): `name()`, `description()`, `parameters()` (JSON Schema), `execute(Value, &ExecutionContext) → ToolResult`. Optional (have defaults): `capabilities()`, `to_schema()`, `cacheable()`, `requires_approval()`, `execution_timeout()`, `routing_rules()`, `usage_examples()`.
- **`ToolMiddleware`** (`crates/oxicrab-core/src/tools/base/mod.rs`): `before_execute()` (can short-circuit), `after_execute()` (can modify result). Built-in: `CacheMiddleware`, `TruncationMiddleware`, `LoggingMiddleware`.
- **`ExecutionContext`** (`crates/oxicrab-core/src/tools/base/mod.rs`): Passed to every `execute()` call. Fields: `channel`, `chat_id`, `context_summary`, `metadata` (channel-specific metadata from the originating inbound message, e.g. Slack `ts` for threading).
- **`BaseChannel`** (`crates/oxicrab-core/src/channels/base/mod.rs`): `start()`, `stop()`, `send()`. Optional: `send_typing()`, `send_and_get_id()`, `edit_message()`, `delete_message()`. Both Discord and Slack support interactive buttons via a unified `metadata["buttons"]` format — each channel converts to its native representation (Discord action rows, Slack Block Kit). Button clicks arrive as `[button:{id}]` inbound messages on both channels. Discord also supports slash commands, embeds, and interaction webhook followups — metadata keys `discord_interaction_token`/`discord_application_id` route responses through webhook API. Interaction tokens have a 15-minute TTL; the channel checks expiry (14-min safety margin) before followup and falls back to a regular channel message if expired. Slack supports configurable reaction emoji lifecycle (`thinkingEmoji` → `doneEmoji`) and structured error classification with retry for transient (5xx) errors.
- **`LLMProvider`** (`crates/oxicrab-core/src/providers/base/mod.rs`): `chat(ChatRequest) → LLMResponse`, `default_model()`, `warmup()`. Has default `chat_with_retry()` using the `backoff` crate (exponential backoff + jitter, honoring `retry_after` hints for rate limits). `warmup()` pre-warms HTTP connections on startup (default no-op, implemented for Anthropic/OpenAI/Gemini).

## Provider Selection

`ProviderFactory` in `crates/oxicrab-providers/src/strategy/mod.rs` uses 2-tier resolution to pick a provider:

1. **Prefix notation** — `provider/model` syntax (e.g. `groq/llama-3.1-70b`). Only recognized prefixes (anthropic, openai, gemini, openrouter, deepseek, groq, minimax, moonshot, zhipu, dashscope, vllm, ollama) are split; unknown prefixes like `meta-llama/` are left intact.
2. **Model-name inference** — `starts_with` patterns: `claude-*`/`claude_*` → Anthropic, `gpt-*`/`o1`/`o3`/`o4` → OpenAI, `gemini*` → Gemini, `deepseek*` → DeepSeek.

For the `anthropic` provider, OAuth is tried first (Claude CLI → OpenClaw → credentials file), falling back to API key. For `openai`/`gemini`, the API key is used directly. All providers support `apiBase` (custom base URL) and `headers` (custom HTTP headers) in their config — first-party providers use `with_config()` constructors, OpenAI-compat providers use `OpenAIProvider::with_config_and_headers()`. When neither is set, the simpler `new()` constructor is used with hardcoded defaults. `promptGuidedTools` is a `LocalProviderConfig`-only field (ollama/vllm) — when true, `PromptGuidedToolsProvider::wrap()` is applied, injecting tool definitions into the system prompt and parsing `<tool_call>` XML blocks from text responses.

## Model Routing

Model routing allows different task types to use different providers and models. `ModelRoutingConfig` (in `crates/oxicrab-core/src/config/schema/agent.rs`) has three fields: `default` (base model), `tasks` (per-task overrides via `TaskRouting` enum), and `fallbacks` (resilience chain). Simple tasks (`daemon`, `cron`, `compaction`, `subagent`) map to a model string. The `chat` task supports a `ChatRoutingConfig` object with complexity-based escalation (`thresholds`, `models`, `weights`). At startup, `Config::create_routed_providers()` pre-creates providers for each task model (deduplicated) and builds `ResolvedChatRouting` with pre-resolved standard/heavy providers. `ResolvedRouting::resolve_overrides(task_type)` does direct task→provider lookup. `ResolvedRouting::resolve_chat(composite)` maps complexity scores to provider overrides. The `FallbackProvider` supports chains of N providers (built from `modelRouting.fallbacks`) — tried in order on error or malformed tool calls. When `tasks` is empty, single-model behavior is preserved.

## Prompt Caching (Anthropic)

Anthropic providers automatically apply `cache_control: {"type": "ephemeral"}` to:
- The last content block in the system message array
- The last tool definition in the tools array

This enables the Anthropic prompt cache (5-minute TTL), reducing input token costs by up to 90% for repeated system prompts and tool definitions across consecutive requests. Cache token usage is reported in `LLMResponse.cache_creation_input_tokens` and `cache_read_input_tokens`.

## Tool System

- **`ToolRegistry`** (`src/agent/tools/registry/mod.rs`): Central execution engine. Runs middleware pipeline: `before_execute` → `execute_with_guards` (timeout + panic isolation via `tokio::task::spawn`) → `after_execute`. Stored as `Arc<ToolRegistry>` (immutable after construction).
- **`ToolBuildContext`** (`src/agent/tools/setup/mod.rs`): Aggregates all config needed for tool construction. `register_all_tools()` calls per-module registration functions.
- **MCP** (`src/agent/tools/mcp/`): `McpManager` connects to external MCP servers via child processes (`rmcp` crate). `McpProxyTool` wraps each discovered tool as `impl Tool`. Config under `tools.mcp.servers`. Each server has a `sandbox` field (`SandboxConfig`) for Landlock kernel-level sandboxing of the child process (enabled by default). `McpManager::new()` takes a workspace path; `McpProxyTool` sanitizes error messages via `path_sanitize`.
- **Deferred tool registry / tool_search** (`src/agent/tools/tool_search/mod.rs`): MCP tools register as "deferred" — their schemas are excluded from LLM requests to save tokens. The `tool_search` built-in meta-tool lets the LLM discover deferred tools by keyword. Activated tools are tracked via a shared `Arc<Mutex<HashSet<String>>>` and included in subsequent LLM requests.
- **Tool output stash** (`src/agent/tools/stash/mod.rs`): In-memory LRU cache (32 entries, 32 MB total) that preserves large tool outputs before truncation. When `TruncationMiddleware` truncates a result, the full content is stashed and a `stash_retrieve` tool lets the LLM recover it with pagination (`offset`/`limit` params).
- **Tool parameter auto-casting** (`src/agent/tools/registry/mod.rs`): `coerce_params_to_schema()` runs before tool execution, fixing common LLM type mismatches (string→integer, string→number, number→string, string→boolean, string→array/object). Saves a full round-trip per mismatch.
- **Schema hint injection** (`src/agent/tools/registry/mod.rs`): `inject_schema_hint()` appends the tool's description and parameter schema to error messages when a tool returns `is_error: true`. Helps the LLM self-correct without needing full schemas in every request.
- **Tool pre-filtering / routing policy**: `ToolCategory` (in `crates/oxicrab-core/src/tools/base/mod.rs`) still classifies tools by domain for broader tooling, but routed turns are now governed by `RoutingPolicy` (`allowed_tools`, `blocked_tools`, `reason`, optional `context_hint`). Execution enforces policy at tool-dispatch time, and unconstrained turns may apply semantic subset selection dynamically from currently visible/activated tools.

## Agent Loop (`src/agent/loop/mod.rs`)

`AgentLoop::new(AgentLoopConfig)` runs up to `max_iterations` (default 20) of: LLM call → parallel tool execution → append to conversation. Tool execution flows through a shared execution gateway (`execute_tool_call`) for LLM calls and direct-dispatch paths, enforcing schema validation, approval checks, exfiltration guard policy, timeout/panic isolation, truncation, and logging consistently. All iterations use `tool_choice=None` (auto). Safety against text-only hallucinations comes from `handle_text_response()` which detects false action claims (Layer 1 only). Hallucination detection runs on final text responses. Responses flow through the loop's return value (no message tool); the caller sends them exactly once. At 70% of `max_iterations`, a system message prompts the LLM to begin wrapping up. Post-compaction recovery instructions include the last user message and a compaction summary. Cognitive `CheckpointTracker` (configured via `agents.defaults.cognitive`) nudges the LLM to self-checkpoint via escalating pressure messages based on tool call volume.

### Complexity-Aware Routing (`src/agent/loop/complexity/mod.rs`)

`ComplexityScorer` scores inbound messages across 7 dimensions (message length, reasoning keywords, technical vocabulary, question complexity, code presence, instruction complexity, conversational simplicity) using AC automata and regex. Composite score maps to standard/heavy model thresholds defined in `ChatRoutingConfig`. Wired in `process_message_unlocked()` after router pre-classification.

### Context Providers (`src/agent/context/providers/mod.rs`)

Dynamic system prompt injection via external commands. Config: `agents.defaults.contextProviders` array with `name`, `command`, `args`, `enabled`, `timeout` (default 5s), `ttl` (default 300s), `requiresBins`, `requiresEnv`. Providers execute via `scrubbed_command()` (env-cleared). Output capped at 100 KB, cached by TTL, injected as `# Dynamic Context` section in the system prompt.

### Reasoning Content (Thinking Models)

`reasoning_content` (thinking/chain-of-thought output from extended thinking models like Claude Opus 4, DeepSeek-R1) is preserved across the full message lifecycle: LLM response → `Message.reasoning_content` field → `ContextBuilder::add_assistant_message()` → Anthropic `convert_messages()` emits `{"type": "thinking"}` content blocks → session history restores from `reasoning_content` key. The OpenAI provider parses DeepSeek-R1's `reasoning_content` response field.

### Group Chat Memory Isolation

When `is_group` metadata is true on an inbound message, `build_system_prompt_inner()` passes `is_group=true` to `MemoryStore::get_memory_context_scoped()`, which excludes personal memory entries from the system prompt and search results. Each channel sets `is_group` in metadata: Telegram (`chat.is_group()/is_supergroup()`), Discord (`guild_id.is_some()`), Slack (channel ID not starting with 'D').

### Message Router (`crates/oxicrab-router/src/`)

`MessageRouter` is a stateless routing engine that sits at the top of `process_message_unlocked()`, making sub-100us decisions about whether a message needs LLM involvement. Checks in priority order: structured action payloads (buttons, webhooks, cron/tool-chain dispatch) -> session action directives -> prefixed config commands -> static tool rules -> remember fast path -> guided LLM (active context, policy-constrained tools) -> full LLM. For full-LLM turns, the loop may apply semantic tool filtering before the LLM call when confidence is sufficient.

`RouterContext` (active tool, action directives with TTL) persists in `Session.metadata["router_context"]`. Tools declare static rules via `routing_rules()` on the `Tool` trait and dynamic directives via `ToolResult.metadata["action_directives"]` + `["active_tool"]`. Directives are case-insensitive whole-message matches with configurable TTL (default 5 min, max 20 per session).

Config: `router.prefix` (default "!") for prefix commands, `router.rules` for user-defined command mappings.

The `GuidedLLM` path derives a strict routing policy (`allowed_tools`, `blocked_tools`, `reason`) and injects an `## Active Interaction` context hint into the system prompt. The `DirectDispatch` path executes tools via the same execution gateway used by LLM tool calls, records synthetic session history, and returns the tool result directly without LLM involvement.

Dispatch types (`ActionDispatch`, `ActionSource`, `ActionDispatchPayload`) live in `src/dispatch/mod.rs`. `DispatchContextStore` (bounded `moka` cache with 15-min TTL) bridges Discord's lack of a button value field.

## Memory System (`crates/oxicrab-memory/src/`)

`MemoryStore` wraps `MemoryDB` (SQLite FTS5) + `EmbeddingService` (fastembed ONNX). Memory entries are stored directly in the SQLite database (no file-based notes or background indexer). The `memory_search` tool and system prompt context injection query the database directly.

### Schema Migrations (`crates/oxicrab-memory/src/memory_db/migrations.rs`)

SQLite schema is versioned via `PRAGMA user_version`. `apply_migrations()` runs at startup and applies each migration block sequentially, gated by `user_version(conn)? < N`. Currently at version 5:

- **v1**: Base schema — `memory_entries` (FTS5), `memory_sources`, `memory_embeddings`, `llm_cost_log`, `intent_metrics`, `sessions`, `cron_jobs` + `cron_job_targets`, `scheduled_task_dlq`, `memory_access_log` + `memory_search_hits`, `pairing_pending` + `pairing_failed_attempts`, `oauth_tokens`, `workspace_files`, `subagent_logs`. Created from `migrations/0001_base.sql` via `include_str!`.
- **v2**: Add `request_id TEXT` column to `llm_cost_log`, `intent_metrics`, `memory_access_log`.
- **v3**: Add composite index on `memory_entries(source_key, created_at)`.
- **v4**: Add index on `sessions(updated_at)`.
- **v5**: Add RSS tables (`rss_feeds`, `rss_articles`, `rss_article_tags`, `rss_profile`, `rss_model`) with indexes.

All migrations are idempotent (`CREATE TABLE/INDEX IF NOT EXISTS`, `add_column_if_missing()`). Column additions are security-hardened via `ensure_allowed_column_addition()` which allowlists exact `(table, column, type)` triples — any other combination is rejected with `bail!()`. FTS5 virtual tables and triggers are created separately in `ensure_fts_objects()` to gracefully degrade on systems without FTS5 support.

### Hybrid Search

`MemoryDB::hybrid_search()` combines FTS5 BM25 keyword matching with vector cosine similarity. Two fusion strategies (configurable via `agents.defaults.memory.searchFusionStrategy`):

- **`WeightedScore`** (default): Normalizes BM25 scores to [0,1] (inverted, since BM25 is more-negative-is-better) and cosine similarity to [0,1]. Blends: `combined = keyword_weight * fts + (1 - keyword_weight) * vec`. `hybridWeight` controls the blend (0.0 = keyword only, 1.0 = vector only).
- **`Rrf`** (reciprocal rank fusion): Ignores raw scores, ranks results from each source independently, then computes `score = 1/(k + fts_rank) + 1/(k + vec_rank)`. More robust to score distribution differences between BM25 and cosine. `rrfK` (default 60) controls emphasis on top ranks.

### Recency-Weighted BM25

`recency_decay()` in `crates/oxicrab-memory/src/memory_db/mod.rs` applies exponential decay (`0.5 ^ (age_days / half_life_days)`) to normalized BM25 scores during hybrid search. Config: `agents.defaults.memory.recencyHalfLifeDays` (default 90, 0 = disabled). Decay only affects keyword scores, not vector similarity. Applied after BM25 normalization, before fusion.

### Remember Fast Path (`crates/oxicrab-memory/src/remember/mod.rs`)

Six trigger patterns (e.g. "remember that ", "please remember ") bypass the LLM entirely, writing directly to daily notes. Rejects content < 8 chars, questions, and interrogative forms. Deduplication via Jaccard similarity (threshold 0.7) against recent DB entries. Classified by `MessageRouter::route()` at priority 6 and dispatched via `handle_direct_dispatch()`.

### Memory Quality Gates (`crates/oxicrab-memory/src/quality/mod.rs`)

`check_quality()` returns `QualityVerdict`: `Pass`, `Reframed(String)`, or `Reject(RejectReason)`. Rejects greetings/filler (~45 patterns) and content < 15 chars. Reframes negative memories unless they contain constructive markers. `filter_lines()` applies quality gates per-line for multi-line LLM output. Used in `try_remember_fast_path()` and pre-compaction flush.

### Embedding Cache

`EmbeddingService` caches `embed_query()` results in an LRU cache (default 10,000 entries, configurable via `agents.defaults.memory.embeddingCacheSize`). Avoids redundant ONNX inference for repeated search queries. `embed_texts()` (batch indexing) is not cached since indexed content changes infrequently and results are stored in SQLite.

## Channel Formatting Hints

Per-channel formatting hints are injected into the system prompt during `build_messages()`:
- **Discord**: Markdown (no tables), URL embed suppression, 2000 char limit
- **Telegram**: Bold/italic/code/lists (no tables), 4096 char limit
- **Slack**: Slack mrkdwn syntax (not standard markdown)
- **WhatsApp**: Concise, bold/italic only
- **Twilio**: Plain text (SMS)

## Feature Flags (channel selection + optional features)

```toml
default = ["channel-telegram", "channel-discord", "channel-slack", "channel-whatsapp", "channel-twilio", "keyring-store", "local-whisper", "embeddings", "tool-rss"]
channel-telegram = ["oxicrab-channels/channel-telegram"]
channel-discord = ["oxicrab-channels/channel-discord"]
channel-slack = ["oxicrab-channels/channel-slack"]
channel-whatsapp = ["oxicrab-channels/channel-whatsapp", ...]
channel-twilio = ["oxicrab-channels/channel-twilio"]
keyring-store = ["dep:keyring"]
local-whisper = ["oxicrab-transcription/local-whisper"]  # local whisper.cpp voice transcription
embeddings = ["dep:fastembed", "oxicrab-memory/embeddings"]  # fastembed ONNX for vector search
tool-rss = ["dep:oxicrab-tools-rss", "oxicrab-memory/rss"]   # RSS feed reader with LinTS ranking
```

Channel features are forwarded to the `oxicrab-channels` crate, which conditionally compiles each channel. Keyring support (`keyring-store`) is default-on for desktop; containers should build with `--no-default-features` and use env vars instead.

## Voice Transcription (`crates/oxicrab-transcription/src/lib.rs`)

`TranscriptionService` supports two backends: local (whisper-rs + ffmpeg) and cloud (Whisper API). Routing controlled by `prefer_local` config flag — tries preferred backend first, falls back to the other. Local inference runs whisper.cpp via `spawn_blocking`; audio converted to 16kHz mono f32 PCM via ffmpeg subprocess. `TranscriptionService::new()` returns `Some` if at least one backend is available.

## Config

JSON at `~/.oxicrab/config.json` (or `OXICRAB_HOME` env var). Uses camelCase in JSON, snake_case in Rust (serde `rename` attrs). Schema in `crates/oxicrab-core/src/config/schema/mod.rs` — 22 structs have custom `Debug` impls (via `redact_debug!` macro) that redact secrets. Validated on startup via `config.validate()`. Notable config fields: `providers.*.headers` (custom HTTP headers for OpenAI-compatible providers), `agents.defaults.cognitive` (`CognitiveConfig` with thresholds for tool-call checkpoint nudges), `tools.exfiltrationGuard` (`ExfiltrationGuardConfig` with `enabled` and `allowTools`), `tools.exec.sandbox` (`SandboxConfig` with `enabled`, `additionalReadPaths`, `additionalWritePaths`, `blockNetwork`), `agents.defaults.promptGuard` (`PromptGuardConfig` with `enabled` and `action`).

## Error Handling

`OxicrabError` in `crates/oxicrab-core/src/errors/mod.rs` — typed variants: `Config`, `Provider { retryable }`, `RateLimit { retry_after }`, `Auth`, `Internal(anyhow::Error)`. See [Code Style & Patterns](CLAUDE.md#code-style--patterns) for usage conventions.

## Token Logging (`crates/oxicrab-memory/src/memory_db/cost.rs`)

Raw token usage logging to the `llm_cost_log` SQLite table via `MemoryDB::record_tokens()`. Tracks model, input/output tokens, cache creation/read tokens, caller, and request_id. No dollar amount estimation — token counts are the ground truth. `get_token_summary()` returns usage grouped by date and model. The old CostGuard pricing system was removed in favor of raw token logging.

## Session Affinity

`session_affinity_id()` in `crates/oxicrab-providers/src/lib.rs` generates a per-process UUID sent as an `x-session-affinity` HTTP header on all LLM provider requests. Load balancers can use this to route requests to the same backend for prompt cache locality.

## LLMResponse Extensions

`LLMResponse` (`crates/oxicrab-core/src/providers/base/mod.rs`) includes `finish_reason: Option<String>` (parsed from all providers: OpenAI `"stop"`/`"length"`/`"tool_calls"`, Anthropic `"end_turn"`/`"max_tokens"`/`"tool_use"`, Gemini `"STOP"`/`"MAX_TOKENS"`) and `reasoning_signature: Option<String>` (provider-specific reasoning trace identifier). Pre-compaction flush checks `finish_reason` and discards truncated output rather than writing corrupted data to memory.

## Circuit Breaker (`crates/oxicrab-providers/src/circuit_breaker/mod.rs`)

`CircuitBreakerProvider::wrap(inner, config)` returns `Arc<dyn LLMProvider>` wrapping the inner provider. Three states: Closed (passes through), Open (rejects immediately after `failure_threshold` consecutive transient failures), HalfOpen (allows `half_open_probes` test requests after `recovery_timeout_secs`). Transient errors: 429, 5xx, timeout, connection refused/reset. Non-transient errors (auth, invalid key, permission, context length) do **not** trip the breaker. Config under `providers.circuitBreaker`: `enabled` (default false), `failureThreshold` (default 5), `recoveryTimeoutSecs` (default 60), `halfOpenProbes` (default 2).

## Cognitive Routines (`src/agent/cognitive/mod.rs`)

`CheckpointTracker` emits escalating pressure messages that nudge the LLM to self-checkpoint during long tool-heavy agent loop runs. Tracks tool call volume with a rolling window (`recent_tools_window`, default 10) and fires three one-shot pressure levels: gentle hint (`gentleThreshold`, default 12), firm warning (`firmThreshold`, default 20), urgent demand (`urgentThreshold`, default 30). Each level emits only once per cycle; counters reset when a periodic checkpoint fires. The tracker is local to each `run_agent_loop()` invocation (not persisted). A `breadcrumb()` method produces a cognitive state summary injected into compaction recovery context. Static cognitive instructions are injected as a system message when enabled. Config under `agents.defaults.cognitive`: `enabled` (default false), thresholds, `recentToolsWindow`.

## Cron System (`src/cron/`)

`CronService` manages scheduled and event-triggered jobs. Supports 4 schedule types: `At` (one-shot absolute or relative via `delay_seconds`), `Every` (interval), `Cron` (5-field expression), `Event` (regex match on inbound messages). The `delay_seconds` parameter (1s–1yr) resolves to an absolute `at_ms` timestamp server-side, avoiding LLM timestamp miscalculation. Jobs are persisted to SQLite `cron_jobs` + `cron_job_targets` tables in MemoryDB. `CronService::new(db: Arc<MemoryDB>)`. `CronPayload` specifies execution semantics: `kind` ("agent_turn" or "echo"), `message`, `targets` (channel + recipient), and `agent_echo`. `EventMatcher` checks inbound messages against event-triggered jobs with regex matching, channel filtering, cooldown enforcement (preserved across rebuilds via `merge_fired_state`), expiry, and max_runs. The cron tool supports actions: add, list, remove, run, dlq_list, dlq_replay, dlq_clear.

### Cron Dead Letter Queue (`crates/oxicrab-memory/src/memory_db/dlq.rs`)

Failed cron job executions are stored in the `scheduled_task_dlq` SQLite table (`DlqEntry` struct). Auto-purge keeps only 100 most recent entries. Three cron tool actions expose it: `dlq_list` (with optional status filter), `dlq_replay` (by ID), `dlq_clear`.

## Doctor (`src/cli/doctor/mod.rs`)

`oxicrab doctor` — system diagnostics command. Checks: config exists/parses/validates, workspace writable, provider API keys configured, provider connectivity (warmup with latency), per-channel status (compiled + enabled + tokens), voice transcription backends, external tools (ffmpeg, git), MCP servers. Includes security audit: config file permissions, directory permissions, empty allowlists, pairing store status. Output: PASS/FAIL/SKIP per check with summary counts. Returns exit code 1 if config file missing.

## Credential Registry (`src/config/credentials/mod.rs`)

Unified credential management via `define_credentials!` macro. Adding a new credential = one line in the macro. All backends (env vars, keyring, credential helper) are generated from a single declarative table of 29 credential slots. Resolution order: env var → credential helper → keyring → config.json.

- **`apply_env_overrides()`**: Checks `OXICRAB_*` env vars for all 29 credential slots
- **`apply_credential_helper()`**: Fetches secrets from external processes (1Password, Bitwarden, custom scripts)
- **`apply_keyring_overrides()`** (behind `keyring-store` feature): Loads from OS keychain
- **`detect_source()`**: Identifies where a credential came from (env/keyring/config/helper/empty)
- **`CredentialHelperConfig`** in `crates/oxicrab-core/src/config/schema/mod.rs`: `command`, `args`, `format` (json/1password/bitwarden/line)

## Security Hardening

- **Credential backends** (`src/config/credentials/mod.rs`): Three-tier credential resolution (env > helper > keyring > config.json). All 29 credential slots covered by `OXICRAB_*` env vars. OS keychain via `keyring` crate (optional, `keyring-store` feature). External helper protocol supports 1Password (`op`), Bitwarden (`bw`), and custom scripts.
- **Default-deny allowlists** (`crates/oxicrab-channels/src/utils/`): Empty `allowFrom` arrays now deny all senders. Use `["*"]` for open access.
- **DM policy** (`crates/oxicrab-channels/src/utils/`): Per-channel `dmPolicy` field controls access for unknown senders: `"allowlist"` (default, silent deny), `"pairing"` (send pairing code), `"open"` (allow all). `check_dm_access()` returns `DmCheckResult` (Allowed/Denied/PairingRequired). Each channel handles pairing replies natively (Telegram sends message, Discord sends ephemeral response, Slack posts via API, Twilio returns TwiML, WhatsApp logs the code). **DM access checks are skipped for group messages** — Telegram checks `is_group()/is_supergroup()`, Discord checks `guild_id.is_some()`, Slack checks channel ID prefix (DMs start with `D`). Discord slash commands and component interactions also skip the check for guild interactions.
- **DM pairing** (`src/pairing/mod.rs`): `PairingStore` provides SQLite-backed per-channel allowlists in the shared `MemoryDB` (tables: `pairing_allowlist`, `pairing_pending`, `pairing_failed_attempts`). 8-char human-friendly codes with 15-min TTL. Per-client lockout tracking prevents brute-force code guessing with bounded client set (1000 max). Code comparison uses `subtle::ConstantTimeEq` in Rust (not SQL) to prevent timing side-channels. CLI: `oxicrab pairing list|approve|revoke`.
- **Leak detection** (`crates/oxicrab-safety/src/leak_detector/`): `LeakDetector` scans messages for API key patterns (Anthropic, OpenAI, Slack, GitHub, Groq, Telegram, Discord, Google, Stripe, SendGrid — 15 pattern types). Three-encoding scanning: plaintext patterns, base64-decoded candidates (20+ chars), and hex-decoded candidates (40+ chars). `add_known_secrets()` registers actual config secret values for exact-match detection across all three encodings. `Config::collect_secrets()` gathers all non-empty API keys and tokens; `setup_message_bus()` passes them to the leak detector at startup via `add_known_secrets()`. Bidirectional scanning: **inbound** scanning in `AgentLoop` (`process_message_unlocked()` and `process_direct_with_overrides()`) redacts secrets before they reach the LLM or get persisted in session history; **outbound** scanning in `MessageBus::publish_outbound()` redacts before sending to channels.
- **DNS rebinding defense** (`crates/oxicrab-core/src/utils/url_security.rs`): `validate_and_resolve()` resolves DNS and returns `ResolvedUrl` with pinned `SocketAddr`s. Callers (http, web_fetch tools) build one-shot reqwest clients with `.resolve()` to pin DNS, preventing TOCTOU rebinding attacks where DNS returns a different IP between validation and fetch. Blocked ranges: RFC 1918 private, loopback, link-local, multicast, documentation (`2001:db8::/32`), 6to4 (`2002::/16`), NAT64 (`64:ff9b::/96`), CGNAT/shared (`100.64.0.0/10`), Teredo tunneling (`2001:0000::/32`), and IPv4-mapped IPv6.
- **Tool capability metadata** (`crates/oxicrab-core/src/tools/base/mod.rs`): Every tool declares `ToolCapabilities` via a `capabilities()` trait method: `built_in` (true for oxicrab tools, false for MCP), `network_outbound` (true if tool makes external network requests), `subagent_access` (`Full`/`ReadOnly`/`Denied`), `category` (`ToolCategory` enum for pre-filtering by task type), and `actions` (vec of `ActionDescriptor` with `name` and `read_only` flag for action-based tools). Defaults are deny-by-default: `built_in: false`, `network_outbound: false`, `subagent_access: Denied`, `category: Core`, `actions: []`. Used by the exfiltration guard, subagent tool builder, and MCP shadow protection.
- **Subagent tool access** (`src/agent/subagent/mod.rs`): `build_subagent_tools()` iterates the main agent's tool registry and checks each tool's `SubagentAccess`. `Full` tools are passed through directly, `ReadOnly` tools are wrapped in `ReadOnlyToolWrapper` (schema filtering hides mutating actions, execution-time rejection blocks attempts), `Denied` tools are excluded. Network-outbound tools are additionally blocked when the exfiltration guard is enabled unless allow-listed.
- **Exfiltration guard** (`crates/oxicrab-core/src/config/schema/`): `ExfiltrationGuardConfig` with `enabled` (default false) and `allowTools` (default: empty). When enabled, tools with `network_outbound` capability are filtered from `tools_defs` before sending to the LLM, AND blocked at dispatch time in `execute_tool_call()`. Use `allowTools` to selectively re-enable specific network tools. Config under `tools.exfiltrationGuard`.
- **Prompt injection detection** (`crates/oxicrab-safety/src/prompt_guard/`): `PromptGuard` with regex patterns across 4 categories: role switching, instruction override, secret extraction, jailbreak. Scans user messages in `process_message_unlocked()` (configurable: warn or block) and tool output in `run_agent_loop()` (warn only). Config under `agents.defaults.promptGuard` with `enabled` (default false) and `action` ("warn" or "block").
- **Subprocess env scrubbing** (`src/utils/subprocess/mod.rs`): `scrubbed_command()` calls `env_clear()` then copies only allowlisted vars (`PATH`, `HOME`, `USER`, `LANG`, `LC_ALL`, `TZ`, `TERM`, `RUST_LOG`, `TMPDIR`, `XDG_RUNTIME_DIR`). Applied to all child processes: shell exec, MCP servers, ffmpeg, tmux.
- **Gateway authentication** (`crates/oxicrab-gateway/src/`): Optional `gateway.apiKey` config field enables bearer token auth for `/api/chat` and A2A task endpoints. Axum middleware checks `Authorization: Bearer <key>` or `X-API-Key: <key>` headers with constant-time comparison. Health, webhook (HMAC), and A2A discovery endpoints are exempt. Startup warning emitted when binding to non-loopback without auth.
- **HTTP tool header blocklist** (`crates/oxicrab-tools-web/src/http/mod.rs`): LLM-supplied request headers are filtered against `BLOCKED_HEADERS` (Host, Authorization, Cookie, Set-Cookie, X-Forwarded-For, X-Forwarded-Host, X-Real-IP, Proxy-Authorization). Blocked headers are silently skipped with a warning log.
- **HTTP body limits** (`crates/oxicrab-core/src/utils/http.rs`): `limited_body()` and `limited_text()` stream response bodies with Content-Length pre-check and chunk-based size cap (default 10 MB). Applied to http tool, web_fetch, and web_search.
- **Shell output cap**: Combined stdout+stderr truncated at 1 MB with `[output truncated at 1MB]` marker. PCM audio capped at 50 MB.
- **Shell AST analysis** (`crates/oxicrab-tools-system/src/utils/shell_ast.rs`): Pre-execution structural analysis via `brush-parser` detects subshells, command/process substitution, `eval`/`source`, interpreter inline execution (`python -c`, `perl -e`), dangerous pipe targets (`| bash`), function definitions, and dangerous device redirections (`> /dev/sda`). Runs before allowlist and regex checks — even allowlisted commands like `python3` are blocked when used with inline exec flags. Unparseable commands return `ViolationKind::Unparseable` (fail-closed) instead of an empty violation list.
- **Shell injection patterns** (`src/utils/regex/mod.rs`): Security blocklist includes patterns for `rm -rf`, raw device access, fork bombs, `eval`, piped downloads, netcat listeners, hex decode to shell, `$VAR` expansion, and input redirection from absolute/home paths.
- **Process sandbox** (`src/utils/sandbox/mod.rs`): Kernel-level sandboxing applied to shell commands and MCP server child processes via `pre_exec`. On Linux, uses Landlock LSM (ABI V5); on macOS, uses Seatbelt (`sandbox_init()` FFI). Default read-only: `/usr`, `/lib`, `/lib64`, `/bin`, `/sbin`, `/etc` (+ `/System`, `/Library`, `/opt/homebrew`, `/usr/local` on macOS). Default read-write: workspace dir, `/tmp`, `/var/tmp` (+ `/private/tmp`, `/private/var/folders` on macOS). Network blocked by default. Config under `tools.exec.sandbox` (shell) and `tools.mcp.servers.*.sandbox` (MCP): `enabled` (default true), `additionalReadPaths`, `additionalWritePaths`, `blockNetwork` (default true). Fail-closed: when sandbox is enabled but fails to apply (e.g. kernel too old, unsupported platform), command execution is blocked with an error rather than running unsandboxed.
- **Capability-based filesystem confinement** (`crates/oxicrab-tools-system/src/filesystem/mod.rs`): When `restrict_to_workspace` is enabled, filesystem tools use `cap_std::fs::Dir` (backed by `openat()`) for TOCTOU-safe confined operations. The root directory is opened once, and all subsequent file operations use relative paths through the capability handle, preventing symlink escape and race conditions between validation and access.
- **Workspace path validation** (`crates/oxicrab-tools-system/src/shell/mod.rs`): When `restrict_to_workspace` is enabled, absolute paths in commands are canonicalized and checked against the workspace boundary.
- **Error path sanitization** (`src/utils/path_sanitize/mod.rs`): `sanitize_path()` and `sanitize_error_message()` redact home directory paths in error messages sent to the LLM. Workspace-relative paths are collapsed to `~/...`, paths outside workspace under home become `<redacted>/filename`, system paths are unchanged. Applied to filesystem tool errors and MCP proxy errors.
- **Config file locking** (`src/config/loader/mod.rs`): `load_config()` acquires a shared (read) lock via `fs2::FileExt`. `save_config()` acquires an exclusive lock via a separate `.json.lock` lockfile (survives atomic renames). Prevents corruption from concurrent config reads/writes.
- **Config permissions**: `check_file_permissions()` warns on startup if config file is world-readable (unix). `save_config()` uses atomic writes via `crate::utils::atomic_write()`.
- **Constant-time comparison**: Twilio webhook signature uses `subtle::ConstantTimeEq` instead of `==`.
- **Skill security scanning** (`src/agent/skills/scanner/mod.rs`): `scan_skill()` scans skill file content for dangerous patterns before injection into the system prompt. Blocked patterns: prompt injection (role override, instruction hijack, secret extraction), credential exfiltration (`curl`+env, `cat /etc/passwd`, `cat .env/.ssh`), reverse shells (`nc -e`, `bash -i /dev/tcp`, mkfifo pipes). Warned patterns: base64-decode piped to shell, Python inline exec with dangerous imports, `eval`/`exec` with command substitution. Patterns compiled once via `LazyLock`. Blocked skills are skipped entirely with a warning log.
- **MCP input sanitization** (`src/agent/tools/mcp/`; MCP remains in the binary crate): Null parameters are stripped from tool calls before forwarding to MCP servers. Environment variable values containing CRLF characters are rejected at MCP server startup to prevent header injection.
- **TruffleHog CI** (`.github/workflows/trufflehog.yml`): Scans for verified secrets on push and pull request.

## A2A Protocol (`crates/oxicrab-gateway/src/a2a/`)

Agent-to-Agent protocol support. Config: `gateway.a2a` with `enabled` (default false), `agentName`, `agentDescription`. Three routes: `GET /.well-known/agent.json` (AgentCard, always public), `POST /a2a/tasks` (submit task, auth-gated), `GET /a2a/tasks/{id}` (get status, auth-gated). Tasks use `channel="http"`, `sender_id="a2a"` and route through the same pending map as the chat API. 120s timeout.

## Workspace Manager (`src/agent/workspace/mod.rs`)

Tracks files written to workspace category directories (`code/`, `documents/`, `data/`, `images/`, `downloads/`, `temp/`) in the `workspace_files` SQLite table. Provides category inference by extension, date-partitioned path resolution (`{category}/{YYYY-MM-DD}/{filename}`), manifest tracking, and TTL-based lifecycle cleanup. `WriteFileTool` auto-registers files; `ReadFileTool` updates `accessed_at`. Hygiene runs at startup.

## CLI Commands

`oxicrab gateway` — full multi-channel daemon. `oxicrab agent -m "message"` — single-turn CLI. `oxicrab onboard` — first-time setup. `oxicrab cron` — manage cron jobs. `oxicrab auth` — OAuth flows. `oxicrab channels` — channel status and WhatsApp login. `oxicrab credentials` — manage credentials (set/get/delete/list/import via OS keychain). `oxicrab status` — quick setup overview. `oxicrab doctor` — system diagnostics. `oxicrab pairing` — manage DM pairing for sender authentication (list/approve/revoke). `oxicrab stats` — cost and search metrics. `oxicrab completion` — shell completion scripts.
