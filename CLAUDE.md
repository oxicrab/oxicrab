# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development

Requires **Rust nightly** (pinned in `rust-toolchain.toml`) and system deps: `cmake`. Voice transcription also requires `ffmpeg`. TLS uses rustls (pure Rust, no OpenSSL dependency).

```bash
# Build (all channels)
cargo build

# Build without optional channels (agent CLI only)
cargo build --no-default-features

# Selective channels
cargo build --no-default-features --features channel-telegram,channel-slack

# Release
cargo build --release
```

## Testing

```bash
# Unit tests
cargo test --lib

# Integration tests (must run single-threaded)
cargo test --test session_management --test cron_jobs --test tool_registry --test message_flow -- --test-threads=1

# Single test
cargo test --lib test_name
cargo test --test tool_registry test_name -- --test-threads=1

# All tests
cargo test -- --test-threads=1
```

Integration tests need `OXICRAB_HOME` set to a temp directory (CI uses `$RUNNER_TEMP/oxicrab-test`). Tests use `MockLLMProvider` from `tests/common/mod.rs` and `TempDir` for isolation.

```bash
# Fuzz testing (requires cargo-fuzz)
cargo fuzz run fuzz_webhook_signature -- -max_total_time=30
cargo fuzz run fuzz_config_parse -- -max_total_time=30
# Targets: fuzz_webhook_signature, fuzz_config_parse, fuzz_prompt_guard, fuzz_leak_detector, fuzz_url_validation
```

## Linting

```bash
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

CI treats clippy warnings as errors. No custom rustfmt/clippy config — uses defaults.

## Git Commits

- Do **not** add `Co-Authored-By` trailers to commits.
- Use conventional commit style (`fix:`, `feat:`, `chore:`, `refactor:`, etc.).
- A `commit-msg` hook in `.githooks/` enforces this. Set up with: `git config core.hooksPath .githooks`
- Allowed types: `feat fix docs style refactor perf test build ci chore revert`
- Format: `type(scope): lowercase description` — e.g. `fix(cron): prevent duplicate job names`

## Releasing

Tag-based releases via `scripts/release.sh`. Pushing a `v*` tag triggers `.github/workflows/release.yml` which builds multi-platform binaries (Linux x86_64, Linux ARM64, macOS ARM64), pushes a Docker image to GHCR, generates a changelog with git-cliff, signs all artifacts with Sigstore cosign, and creates a GitHub Release. Cosign uses keyless OIDC signing (no static keys) — the `id-token: write` permission enables GitHub Actions to mint OIDC tokens for Sigstore's Fulcio CA. Release artifacts get `.bundle` files (signature + certificate + Rekor transparency log entry). Docker images are signed by digest. Users verify with `cosign verify-blob --bundle <file>.bundle --certificate-identity-regexp` or `cosign verify` for container images.

```bash
# Bump and tag (does not push)
./scripts/release.sh patch      # 0.9.5 → 0.9.6
./scripts/release.sh minor      # 0.9.5 → 0.10.0
./scripts/release.sh major      # 0.9.5 → 1.0.0
./scripts/release.sh 1.0.0-rc.1 # explicit version

# Review, then push
git push origin main --follow-tags
```

The script updates `Cargo.toml`, runs `cargo check` to sync `Cargo.lock`, generates `CHANGELOG.md`, commits, and creates an annotated tag. It requires a clean working tree on `main`, up to date with origin.

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed implementation docs.

## Code Style & Patterns

### Logging
- Use explicit imports: `use tracing::{debug, error, info, warn};` — not inline `tracing::info!()`
- Log messages: lowercase start, no trailing periods — `info!("agent loop started")`
- Error messages (`anyhow!`, `bail!`): no trailing periods — `bail!("missing config key")`

### HTTP Clients
- Network-based tools store their own `reqwest::Client` as a struct field (connection pooling). `crate::utils::http::default_http_client()` provides a shared helper with standard timeouts.
- Google tools share a `GoogleApiClient` wrapper (`src/agent/tools/google_common.rs`). `GoogleConfig` has per-tool flags (`gmail`, `calendar`, `tasks` booleans, all default true). `is_configured()` checks `clientId`/`clientSecret` are present, `any_tool_enabled()` checks flags, `required_scopes()` derives OAuth scopes automatically from enabled flags.
- Standard timeouts: **10s connect**, **30s overall**, with `unwrap_or_else(|_| Client::new())` fallback:
  ```rust
  client: Client::builder()
      .connect_timeout(Duration::from_secs(10))
      .timeout(Duration::from_secs(30))
      .build()
      .unwrap_or_else(|_| Client::new()),
  ```
- Per-request overrides use `.timeout(Duration::from_secs(15))` on the request builder

### Tool Structs
- Constructor: `pub fn new(...)` builds the client with timeouts
- Test constructor: `#[cfg(test)] fn with_base_url(...)` for mock server testing
- Implement `Tool` trait: `name()`, `description()`, `parameters()`, `capabilities()`, `execute(params, ctx)`. Additional trait methods with defaults: `to_schema()` (builds OpenAI-style function schema), `cacheable()` (default `false`), `requires_approval()` (default `false`), `execution_timeout()` (default 2 min).
- **`capabilities()`** returns `ToolCapabilities` with: `built_in` (true for core tools), `network_outbound` (true if tool makes external network calls), `subagent_access` (`Full`/`ReadOnly`/`Denied`), `actions` (vec of `ActionDescriptor` with `name` and `read_only` for action-based tools), `category` (`ToolCategory` enum: `Core`, `Web`, `Communication`, `Development`, `Scheduling`, `Media`, `Productivity`, `System`). Defaults: `built_in=false, network_outbound=false, subagent_access=Denied, actions=[], category=Core`. Used by exfiltration guard, subagent builder, and MCP shadow protection.
- Action-based tools use `params["action"].as_str()` dispatch pattern (e.g. GitHub tool has 11 actions: list_issues, create_issue, get_issue, list_prs, get_pr, get_pr_files, create_pr_review, get_file_content, trigger_workflow, get_workflow_runs, notifications). Each action-based tool declares `ActionDescriptor` entries matching its action enum — enforced by completeness tests.
- `ReadOnlyToolWrapper` (`src/agent/tools/read_only_wrapper/mod.rs`) wraps action-based tools to expose only read-only actions to subagents. Dual enforcement: schema filtering (removes mutating actions from enum) + execution-time rejection.
- Registration: Each module has a `register_*()` function in `src/agent/tools/setup/mod.rs`

### Error Handling
- Internal functions use `anyhow::Result`; module boundaries use `OxicrabError`
- Return `ToolResult::error(...)` for user-facing tool errors (not `Err(...)`)
- Use `Err(anyhow::anyhow!(...))` or `anyhow::bail!(...)` for internal failures

### Unit Test Organization
Two patterns, one convention: **inline** for small test suites, **directory module** for extracted tests.

- **Inline** (small tests): `#[cfg(test)] mod tests { ... }` at the bottom of the source file. Use this when tests are ≤25% of the file's total lines.
- **Directory module** (extracted tests): Convert `foo.rs` to `foo/mod.rs` + `foo/tests.rs` with `#[cfg(test)] mod tests;` in `mod.rs`. Use this when tests exceed 25% of the file's total lines.

Do **not** use `#[path = "foo_tests.rs"]` — this was previously used in 4 modules but has been standardized to the directory module pattern. The module path (`crate::config::loader`) is unchanged whether `loader` is a file or directory.

## Common Pitfalls

- **Docs are part of the implementation**: No feature, tool change, config change, or CLI change is complete until docs are updated. **Only edit `docs/_pages/*.html`** (the source files) — `docs/*.html` are generated by `python3 docs/build.py`. The docs site is the **source of truth for detail** (tool actions, CLI flags, config fields). README is the **summary** — it lists tool/command names and one-line descriptions, linking to the docs for full reference. This means:
  - **Tool action lists** → only in `docs/_pages/tools.html` (NOT in README, NOT in `_pages/index.html` data-detail attributes)
  - **CLI flag tables** → only in `docs/_pages/cli.html` (README shows example commands only)
  - **Config field tables** → only in `docs/_pages/config.html` (models, credentials, agent defaults, circuit breaker, logging)
  - **README** → update the tool/command name lists and one-line descriptions; keep it concise
  - **`_pages/index.html`** → update feature rows and tool grid short descriptions (no action lists)
  - **CLAUDE.md** → update architecture/patterns sections if internal behavior changed
- **Adding fields to `AgentLoopConfig`**: Tool-specific configs go in `ToolConfigs` (forwarded to `ToolBuildContext`). Lifecycle fields (TTLs, intervals) go in `LifecycleConfig`. Safety fields (exfiltration guard, prompt guard) go in `SafetyConfig`. Other non-tool fields go in `AgentLoopConfig` directly. Must update `from_config()`, `test_defaults()`, destructure in `AgentLoop::new()`, AND update `tests/common/mod.rs` `create_test_agent_with()`. The `run_agent_loop_with_overrides()` method returns `AgentLoopResult` (named struct, not a tuple).
- **Adding a new tool**: Add a `register_*()` function in `src/agent/tools/setup/mod.rs`, call it from `register_all_tools()`. Update `README.md` and the workspace file `AGENTS.md` if it exists.
- **Adding fields to config structs with manual `Default` impl**: update both the struct definition and `Default::default()`. If the field affects `config.example.json`, update it too — a unit test (`test_config_example_is_up_to_date`) compares Config::default() + credential overlays against the committed file. Add credential placeholders to `credential_overlays()` in `src/config/schema/tests.rs`.
- **Docs staleness is enforced**: A pre-commit hook checks `docs/*.html` freshness when `docs/_pages/` or `docs/_layout.html` are staged. CI also checks via `python3 docs/build.py && git diff --quiet -- docs/`. Always run `python3 docs/build.py` after editing source pages.
- **CI skips heavy jobs for non-code changes**: Docs-only, README-only, and config-example-only PRs run only the `check` and `ci-gate` jobs (~30s). Code paths (`src/`, `tests/`, `Cargo.*`, `fuzz/`, etc.) trigger the full pipeline. Managed by `dorny/paths-filter` in the `changes` job.
- **YAML parsing**: uses `serde_yaml_ng` (not the deprecated `serde_yaml`).
- **`main.rs` is a thin entry point**: it calls `oxicrab::cli::run()`. All module declarations are in `lib.rs`.
- **UTF-8 string slicing**: always use `is_char_boundary()` or `chars()` before slicing.
- **Tool execution**: wrapped in `tokio::task::spawn` for panic isolation via `ToolRegistry::execute_with_guards()`.
- **MemoryDB**: holds a persistent `std::sync::Mutex<Connection>`, not per-operation connections.
- **Cron storage is SQLite-backed**: Cron jobs are stored in `cron_jobs` + `cron_job_targets` tables in MemoryDB (not a JSON file). `CronService::new(db: Arc<MemoryDB>)`. CRUD via `db.insert_cron_job()`, `db.list_cron_jobs()`, `db.get_cron_job()`, `db.delete_cron_job()`, `db.update_cron_job_state()`, `db.update_cron_job_enabled()`, `db.update_cron_job()`. Schedule fields are denormalized columns (`schedule_type`, `at_ms`, `every_ms`, `cron_expr`, `cron_tz`, `event_pattern`, `event_channel`). Targets are in a separate table with `ON DELETE CASCADE`. No file locking, no mtime polling, no `CronStore` type.
- **Cron 5-field expressions**: `compute_next_run()` normalizes by prepending "0 " for the seconds field.
- **Cron `delay_seconds`**: The cron tool `add` action accepts `delay_seconds` (integer, 1–31536000) as an alternative to `at_time` for one-shot scheduling. Resolves to an absolute `at_ms` timestamp server-side via `SystemTime::now()`, avoiding LLM timestamp miscalculation. Mutually exclusive with `at_time`, `every_seconds`, `cron_expr`, `event_pattern`.
- **Cron self-scheduling guard**: The cron `add` action checks `ctx.metadata` for `IS_CRON_JOB` (set in `gateway_setup.rs` via `AgentRunOverrides.metadata`) and rejects new job creation during cron execution, preventing infinite feedback loops. `AgentRunOverrides.metadata` is merged into `ExecutionContext` in `process_direct_with_overrides()`.
- **Process group kill on timeout**: The shell tool uses `cmd.process_group(0)` to run commands in their own process group. On timeout, `libc::killpg()` kills the entire group (not just the top-level shell), preventing orphan child processes. The PID is saved before `wait_with_output()` consumes the child handle.
- **Deferred tool registry / tool_search**: MCP tools are registered as "deferred" — their schemas are excluded from LLM requests to save tokens. The `tool_search` built-in meta-tool lets the LLM discover deferred tools by keyword search. When found, they're "activated" via a shared `Arc<Mutex<HashSet<String>>>` between the tool and the agent loop, which rebuilds tool definitions in subsequent iterations to include activated schemas. `ToolRegistry` methods: `register_deferred()`, `is_deferred()`, `deferred_count()`, `get_tool_definitions_with_activated()`, `get_filtered_definitions_with_activated()`.
- **Session affinity header**: All LLM provider requests include an `x-session-affinity` header with a per-process UUID (`providers::session_affinity_id()`). Load balancers can use this to route requests to the same backend for prompt cache locality.
- **No `#[allow(dead_code)]`**: Do not add `#[allow(dead_code)]` or `#![allow(dead_code)]` anywhere. If code is unused, remove it. CI runs `clippy -D warnings` which catches dead code.
- **No `tool_choice` forcing**: The agent loop uses `tool_choice=None` (auto) for all iterations. Do not re-add `tool_choice="any"` — it breaks conversational flow and disables hallucination detection by setting `any_tools_called=true`. Hallucination safety comes from `handle_text_response()` in `src/agent/loop/hallucination.rs`.
- **Empty `allowFrom` is now deny-all**: Channels with empty `allowFrom` will reject all senders. Add `["*"]` for the old behavior, set `"dmPolicy": "pairing"` to let unknown senders request access, or set `"dmPolicy": "open"` to allow everyone.
- **Adding a new credential**: Add one line to `define_credentials!` in `src/config/credentials/mod.rs`. This auto-generates env var override, keyring access, credential helper lookup, CLI listing, and source detection.
- **Anthropic prompt caching is fully implemented**: `cache_control: {"type": "ephemeral"}` is injected on the system prompt block (via `system_to_content_blocks()`) and the last tool definition (via `convert_tools()`) in `src/providers/anthropic_common/mod.rs`. Both the API-key and OAuth providers use these functions. Cache token usage is parsed from responses (`cache_creation_input_tokens`, `cache_read_input_tokens`) and persisted to the `llm_cost_log` SQLite table via `record_tokens()`.
- **Token logging (no dollar amounts)**: `MemoryDB::record_tokens()` logs model, input/output/cache tokens, caller, and request_id to the `llm_cost_log` table. The `cost_cents` column is written as 0.0 for backward compatibility. `get_token_summary()` returns usage grouped by date and model. The old CostGuard pricing system was removed — token counts are the ground truth.
- **Memory search tracking**: All searches (keyword and hybrid) are logged to `memory_access_log` + `memory_search_hits` tables. Use `db.get_source_hit_count()` to check utility.
- **Embedding back-fill**: Embeddings are back-filled inline after `insert_memory()` via `MemoryStore::backfill_embeddings()`, which calls `get_entries_missing_embeddings()` and generates embeddings in batch.
- **CLI `stats` command**: `oxicrab stats tokens|search|intent|complexity` queries the memory database for token usage and search metrics.
- **Cron execution context**: `ExecutionContext.metadata` carries inbound message metadata to tools.
- **`reasoning_content` preserved across message lifecycle**: The `Message` struct has `reasoning_content: Option<String>` and `reasoning_signature: Option<String>` fields. Anthropic thinking blocks are parsed in `parse_response()`, carried through the agent loop, converted back to `{"type": "thinking"}` content blocks in `convert_messages()`, and restored from session history in `build_messages()`. OpenAI provider parses DeepSeek-R1's `reasoning_content` field. Use `Message::assistant_with_thinking(content, tool_calls, reasoning_content, reasoning_signature)` to construct messages with reasoning content.
- **Group chat memory isolation**: Channels set `is_group` in inbound message metadata (Telegram: `chat.is_group()/is_supergroup()`, Discord: `guild_id.is_some()`, Slack: channel not starting with 'D'). `build_messages()` accepts `is_group: bool` and delegates to `build_system_prompt_inner()` which calls `get_memory_context_scoped(query, true)`. In group mode: `daily:` prefixed entries are excluded from search results at query time via the exclude set.
- **Hybrid search fusion strategy**: `FusionStrategy` enum in `src/config/schema/agent.rs` with `WeightedScore` (default, linear blend) and `Rrf` (reciprocal rank fusion). Config fields: `searchFusionStrategy` ("weighted_score" or "rrf"), `rrfK` (default 60). Threaded through `MemoryStore` → `MemoryDB::hybrid_search()`.
- **Recency-weighted BM25**: `recency_decay()` in `src/agent/memory/memory_db/mod.rs` applies exponential decay (`0.5 ^ (age_days / half_life_days)`) to normalized BM25 scores during hybrid search. Config: `agents.defaults.memory.recencyHalfLifeDays` (default 90, 0 = disabled). Decay only affects keyword (BM25) scores, not vector similarity. Applied after BM25 normalization, before fusion with vector scores.
- **Embedding query cache**: `EmbeddingService` has an LRU cache for `embed_query()` results. Default 10,000 entries, configurable via `agents.defaults.memory.embeddingCacheSize`. `EmbeddingService::with_cache_size()` constructor accepts custom size. `embed_texts()` (batch indexing) is not cached.
- **JSON mode / structured output**: `ResponseFormat` enum in `src/providers/base/mod.rs` with `JsonObject` and `JsonSchema { name, schema }` variants. `ChatRequest` has `response_format: Option<ResponseFormat>`. Provider handling: OpenAI sets `response_format` payload field (`json_object` or `json_schema` with strict mode). Gemini sets `generationConfig.responseMimeType` to `application/json` (+ `responseSchema` for `JsonSchema`). Anthropic (both API key and OAuth) appends a system prompt hint since there is no native JSON mode parameter. Passthrough providers (fallback, prompt-guided, circuit breaker) forward the field. Currently set to `None` at all call sites — tools or future features can opt in per-request.
- **PDF/document support**: `load_and_encode_images()` in `src/agent/loop/helpers.rs` accepts `.pdf` files (validates `%PDF` magic bytes, same 20MB limit as images). `ImageData` struct carries any MIME type. Anthropic provider uses `"type": "document"` for non-image media (vs `"type": "image"`). OpenAI uses `"type": "file"` with data URI. Gemini uses same `inline_data` format for all types. Agent loop strips `[document: ...]` tags via `strip_document_tags()` after encoding. Channels (Telegram, WhatsApp) already download PDFs to `~/.oxicrab/media/`.
- **Model routing**: `ModelRoutingConfig` in `src/config/schema/agent.rs` with `default`, `tasks`, `fallbacks`. `default` is the base `provider/model` string (replaces `agents.defaults.model`). `tasks` maps task types to `TaskRouting` enum: `Model(String)` for simple overrides, `Chat(ChatRoutingConfig)` for complexity escalation. `ResolvedRouting` in `src/config/routing.rs` holds direct `tasks: HashMap<String, (Arc<dyn LLMProvider>, String)>` and optional `ResolvedChatRouting` with pre-resolved standard/heavy providers + thresholds. `resolve_overrides(task_type)` does direct task lookup. `resolve_chat(composite)` maps complexity score to provider override. `task_count()`, `has_chat_routing()`, `chat_weights()`, `chat_thresholds()` accessors.
- **Complexity-aware message routing**: `ComplexityScorer` in `src/agent/loop/complexity/mod.rs`. Constructor: `new(&ComplexityWeights)`. Activated when `modelRouting.tasks.chat` is a `ChatRoutingConfig` object with `thresholds` (`standard`/`heavy`), `models` (`standard`/`heavy`), and optional `weights` (7 dimensions). Scores each inbound message using AC automata + regex (sub-millisecond, zero API calls). Dimensions: message length (sigmoid), reasoning keywords (AC, saturates at 3), technical vocabulary (AC, saturates at 5), question complexity (regex tiers), code presence, instruction complexity, conversational simplicity (negative weight). Force overrides: 2+ reasoning keywords → heavy, pure greeting/filler → default, >50KB → heavy. Composite via `sigmoid(weighted_sum - 0.35, 6.0)`. Wired in `process_message_unlocked()` after intent classification. Band name (light/standard/heavy) derived from thresholds for analytics.
- **Temperature is optional**: `ChatRequest.temperature: Option<f32>`, `AgentDefaults.temperature: Option<f32>` (default `Some(0.7)`). When `None`, providers omit the temperature field from API payloads (lets the provider use its own default). `ProviderConfig.temperature: Option<f32>` adds per-provider override. Resolution chain: **per-provider** → **global** → **omit**. Internal temperatures (tool 0.0, compaction 0.3, extraction 0.0) always use `Some(value)`. `ProvidersConfig::get_temperature_for_model()` resolves the per-provider override using the same provider-resolution logic as `get_api_key()`.
- **FallbackProvider is Vec-based**: `FallbackProvider::new(Vec<(Arc<dyn LLMProvider>, String)>)` for chains, `FallbackProvider::pair()` for legacy two-provider cases. Built from `modelRouting.fallbacks`.
- **Gateway rate limiting**: `gateway.rateLimit` config with `enabled`, `requestsPerSecond`, `burst`. Uses `governor` crate with per-IP keyed rate limiter. Returns 429 with `Retry-After` header.
- **Gateway authentication**: `gateway.apiKey` in config enables bearer token auth on `/api/chat` and A2A task endpoints. Requests must include `Authorization: Bearer <key>` or `X-API-Key: <key>`. Exempt: `/api/health` (always public), `/.well-known/agent.json` (A2A discovery, always public), `/api/webhook/{name}` (uses its own HMAC auth). When `apiKey` is empty and `host` is non-loopback, a startup warning is emitted. Comparison uses constant-time `subtle::ConstantTimeEq`.
- **Gateway HTTP API**: `src/gateway/mod.rs` provides an axum-based REST server with `POST /api/chat`, `GET /api/health`, and `POST /api/webhook/{name}`. `GatewayConfig.enabled` (default `true`) gates whether the HTTP server starts in the `gateway` command. `WebhookConfig.enabled` (default `true`) gates individual webhook endpoints (disabled returns 404). Both use `default_true()` serde default. `HttpApiState` holds `inbound_tx` (to publish to the agent), `pending` map for oneshot response channels, `webhooks` config map, optional `outbound_tx` for target delivery, and a shared `LeakDetector` (with known secrets registered) for webhook target delivery. `chat_handler` creates a oneshot channel, stores the sender in the pending map keyed by request ID (`http-{uuid}`), publishes an `InboundMessage` with `channel="http"`, and awaits the receiver with a 120s timeout. `route_response()` intercepts outbound messages where `channel=="http"`, routes them to the matching pending oneshot, and returns `true` (consumed). Called in `start_channels_loop` before channel dispatch. `start()` takes `inbound_tx`, optional `outbound_tx`, webhooks config, and `known_secrets` for the leak detector; returns `(JoinHandle, HttpApiState)`. Axum and `hmac` are non-optional dependencies (used by gateway, webhooks, and Twilio).
- **Knowledge entries**: Entries with `knowledge:` prefixed source keys appear in hybrid search results, are NOT subject to archive/purge (hygiene skips `knowledge:` prefixed entries), and ARE included in group chats (shared reference, not personal). Knowledge entries are inserted via `insert_memory()` with a `knowledge:` source key prefix.
- **Webhook receiver**: Named webhooks configured in `gateway.webhooks` (`WebhookConfig` in `src/config/schema/mod.rs`). Each webhook has a `secret` (HMAC-SHA256), `template` (`{{key}}` substitution from JSON payload, `{{body}}` for raw), `targets` (channel + `chatId` pairs), and optional `agentTurn` flag. Signature validated via constant-time comparison (`subtle::ConstantTimeEq`); checks `X-Signature-256`, `X-Hub-Signature-256`, and `X-Webhook-Signature` headers, supports `sha256=` prefix. Max payload 1MB. When `agentTurn` is true, message routes through agent loop then delivers response to targets via `outbound_tx`. When false, templated message delivers directly to targets.
- **Resource limits (OOM prevention)**: Context files (USER.md, TOOLS.md, AGENTS.md): 500KB max. Skill files (SKILL.md): 1MB max. Audio uploads (cloud transcription): 25MB max. Base64 images (image generation): 30MB pre-decode check. HTML content (browser tool): 500KB max. Browser screenshot: 10080px height clamp. HTTP response bodies: 10MB max via `limited_body()`. Context provider output: 100KB max. Gateway body: 1MB `DefaultBodyLimit` on all routes (chat, webhook, A2A). Inbound messages: 1MB truncation in `MessageBus::publish_inbound()`. Compaction summary: 2000 chars max (prevents unbounded growth across cycles).
- **Tool name constraints**: Tool names must be ≤256 chars with no null, newline, or control characters. Enforced at registration time in `ToolRegistry`.
- **Tool cache key format**: `len#name:params` — length-prefixed to prevent collision between `tool("ab")` and `tool_a("b")`.
- **Tool output stash**: `ToolOutputStash` in `src/agent/tools/stash/mod.rs` is an in-memory LRU cache (32 entries, 32MB total) that preserves large tool outputs before truncation. When `TruncationMiddleware` truncates a result, the full content is stashed and a note with the stash key is appended. The `stash_retrieve` tool lets the LLM recover the full output with pagination (`offset`/`limit` params, default 50K bytes). `stash_retrieve` results bypass truncation middleware. Shared `Arc<ToolOutputStash>` between middleware and tool, created in `register_all_tools()`. `ToolRegistry::with_stash()` constructor wires it into `TruncationMiddleware`.
- **Tool parameter auto-casting**: `coerce_params_to_schema()` in `src/agent/tools/registry/mod.rs` runs before tool execution in `ToolRegistry::execute()`. Handles common LLM type mismatches: string→integer (`"5"` → `5`), string→number (`"3.14"` → `3.14`), number→string (`42` → `"42"`), string→boolean (`"true"` → `true`), string→array/object (JSON string parsed). No-op when types already match or coercion fails. Saves a full LLM round-trip per mismatch.
- **Schema hint injection on tool errors**: When a tool returns `is_error: true`, `ToolRegistry::inject_schema_hint()` appends the tool's description (capped at 500 chars) and parameter schema (capped at 3000 chars) to the error message. Helps the LLM self-correct without needing full schemas in every request. Especially useful for deferred/MCP tools.
- **`finish_reason` in `LLMResponse`**: All providers (OpenAI, Anthropic, Gemini) now parse the stop reason into `LLMResponse.finish_reason`. OpenAI: `"stop"`, `"length"`, `"tool_calls"`. Anthropic: `"end_turn"`, `"max_tokens"`, `"tool_use"`. Gemini: `"STOP"`, `"MAX_TOKENS"`. Pre-compaction flush checks `finish_reason` and discards truncated output (`"length"`, `"max_tokens"`, `"MAX_TOKENS"`) rather than writing corrupted data to memory.
- **Leak detection uses two-phase Aho-Corasick + regex**: `LeakDetector` in `src/safety/leak_detector/mod.rs` builds an `AhoCorasick` automaton from literal prefixes of each secret pattern (e.g. `sk-ant-api`, `xoxb-`, `ghp_`, `AKIA`, `AIza`, `sk_live_`, `pk_live_`, `SG.`). Phase 1: single-pass AC scan with `find_overlapping_iter()` identifies which patterns have candidate matches. Phase 2: full regex validation runs only on patterns whose prefix was found. `find_overlapping_iter` (not `find_iter`) is required because shorter prefixes like `sk-` would shadow longer ones like `sk-ant-api` at the same position. Patterns with no usable AC prefix (e.g. Discord tokens) use `ac_index: None` and always run regex. Adding a new pattern requires adding a `(name, regex, literal_prefix)` tuple to `pattern_defs` in `LeakDetector::new()`.
- **Inbound secret scanning**: `AgentLoop` has its own `LeakDetector` instance that scans user messages **before** they reach the LLM or get persisted. Scans at two entry points: `process_message_unlocked()` (after audio transcription, before prompt guard) and `process_direct_with_overrides()` (cron/subagent direct calls, before prompt guard). Detected secrets are redacted with `[REDACTED]`. The `MessageBus` separately scans **outbound** messages. Together these form a bidirectional defense: inbound scanning prevents secrets from entering the system, outbound scanning prevents the agent from leaking them. The gateway's `deliver_to_targets()` also runs `LeakDetector::redact()` since it sends through raw `outbound_tx` (bypassing `MessageBus`).
- **Tool result prompt injection**: When `prompt_guard` is configured to block, detected injection in tool output (e.g. malicious web page, MCP response) is redacted — the tool result content is replaced with `[tool output redacted: prompt injection detected in '{name}']`.
- **Per-session processing locks**: `AgentLoop` uses per-session `Mutex<()>` locks (keyed by session key in a `HashMap`), so messages from independent sessions are processed concurrently while messages within the same session are serialized. The lock map uses `std::sync::Mutex<HashMap>` (held briefly for lookup) wrapping `tokio::sync::Mutex<()>` (held during processing).
- **Metadata key constants**: `bus::meta` module defines constants for well-known metadata keys (`IS_GROUP`, `TS`, `STATUS`, `SESSION_ID`, `RESPONSE_FORMAT`, etc.). Use these instead of string literals when reading/writing `InboundMessage.metadata` or `OutboundMessage.metadata`.
- **Tool result secret scanning**: After tool execution, tool result content is scanned through the `LeakDetector` and redacted before entering the LLM context. This closes the gap where secrets in tool output (e.g. `read_file` on a `.env`) could persist in session history.
- **Group access control**: Channels support `allowGroups` config (camelCase in JSON). Empty list = all groups allowed (backward compatible). Non-empty list restricts to listed group/channel IDs. Shared `check_group_access()` in `channels/utils`. Currently supported on Telegram, Discord, and Slack.
- **Webhook replay protection**: Webhook handler checks `X-Webhook-Timestamp` header. Payloads older than 5 minutes are rejected (403). Compatible with providers that include timestamps; no-op for those that don't.
- **Browser SSRF post-action check**: After `eval`, `click`, `type_text`, `fill`, and `navigate` browser actions, the page URL is validated through `validate_and_resolve()` to block JS-initiated navigation to internal IPs.
- **Shell sandbox fail-closed**: When sandbox is enabled but fails to apply (e.g. `bwrap` not found), command execution is blocked with an error instead of running unsandboxed.
- **Tool result blob sanitization**: `strip_binary_blobs()` in `src/agent/truncation/mod.rs` replaces base64 data URIs, long base64 sequences (with `+/=` markers), and long hex sequences (with mixed digits+letters) with descriptive placeholders like `[image/png data, N bytes]`. Applied after ANSI stripping but before size truncation, so text content gets priority over binary blobs. Regex patterns in `src/utils/regex/mod.rs`: `data_uri()`, `long_base64()`, `long_hex()`.
- **Skill security scanning**: `scan_skill()` in `src/agent/skills/scanner/mod.rs` scans skill content for dangerous patterns before injection into the system prompt. Blocks: prompt injection (role override, instruction hijack, secret extraction), credential exfiltration (`curl`+env, `cat .env/.ssh`), reverse shells (`nc -e`, `bash -i /dev/tcp`). Warns: base64-decode piped to shell, Python inline exec, `eval`/`exec` with command substitution. Patterns compiled once via `LazyLock`. Blocked skills are skipped entirely with a warning log. Called from `SkillManager::load_skills_for_context()`.
- **Discord interaction token TTL**: Discord interaction tokens expire after 15 minutes. The Discord channel stores the token creation timestamp in metadata and checks for expiry (14-min safety margin) before attempting a followup. If expired, falls back to sending a regular channel message.
- **Input validation patterns (hardening)**: Strip `\r`/`\n` from email headers (Gmail). Reject URLs with embedded credentials. Filter control characters from sender IDs. Sanitize API error messages before returning to LLM (GitHub). Validate file paths against traversal (Reddit, Todoist, Skills). Reject `system` role messages in conversation history.
- **Gateway router testing**: `tower` (dev dependency, `features = ["util"]`) provides `ServiceExt::oneshot()` for handler-level tests without starting a TCP server. Pattern: `build_router(state).oneshot(Request::builder()...build()).await`. Response type needs annotation: `let resp: axum::http::Response<_> = ...`. Use `axum::body::to_bytes(resp.into_body(), limit)` to read response bodies.
- **MCP param sanitization**: `McpProxyTool` strips null-valued parameters from tool call arguments before forwarding to MCP servers (some servers reject null params). `McpManager` rejects environment variable values containing `\r` or `\n` at server startup to prevent header injection.
- **MCP timeouts**: Server handshake: 30s. Tool discovery: 10s per server. Applied in `McpManager`.
- **A2A protocol (Agent-to-Agent)**: `src/gateway/a2a/mod.rs`. Config: `gateway.a2a` with `enabled` (default false), `agentName`, `agentDescription`. Three routes: `GET /.well-known/agent.json` (AgentCard, always public), `POST /a2a/tasks` (submit task, auth-gated), `GET /a2a/tasks/{id}` (get status, auth-gated). Tasks use `channel="http"`, `sender_id="a2a"` — routed through the same `pending` map and `route_response()` as the chat API. 120s timeout. `gateway::start()` accepts `a2a_config: Option<A2aConfig>` and `api_key: Option<String>`. Body size limited by `DefaultBodyLimit`.
- **System prompt datetime prominence**: `get_identity()` in `src/agent/context/mod.rs` prepends `"The current date and time is {natural_language_datetime}."` as the very first line of the system prompt, before the identity content. This ensures LLMs reliably pick up temporal context. Format: `"Friday, March 6, 2026 at 14:30 UTC"`. The structured `**Date**:` field in `## Current Context` is retained for machine reference. Each user message also gets a `[HH:MM]` prefix in `build_messages()`.
- **Context providers (dynamic system prompt)**: `src/agent/context/providers/mod.rs` (module path unchanged). Config: `agents.defaults.contextProviders` array of `ContextProviderConfig` with fields: `name`, `command`, `args`, `enabled` (default true), `timeout` (default 5s), `ttl` (default 300s), `requiresBins`, `requiresEnv`. Providers execute via `scrubbed_command()` (env-cleared, allowlisted vars only — secrets NOT inherited). Output capped at 100KB, cached by TTL, injected into system prompt as `# Dynamic Context` section. `context_providers: Vec<ContextProviderConfig>` was added to `AgentLoopConfig`.
- **Cron dead letter queue**: Failed cron job executions are stored in `scheduled_task_dlq` SQLite table (`DlqEntry` struct in `src/agent/memory/memory_db/dlq.rs`). Auto-purge keeps only 100 most recent entries. Three cron tool actions: `dlq_list` (with optional `dlq_status` filter), `dlq_replay` (by `dlq_id`), `dlq_clear`. Both cron jobs and DLQ entries live in the same MemoryDB.
- **Pre-compaction memory flush**: `CompactionConfig.pre_flush_enabled` (camelCase: `preFlushEnabled`, default false). When enabled, before compaction removes messages, an LLM call (800 max tokens, temperature 0.0) extracts important context and writes it to the memory DB under a `daily:{date}:Pre-compaction context` source key. Session metadata tracks `pre_flush_msg_count` to prevent double-flush.
- **Orphan tool message cleanup**: `strip_orphaned_tool_messages()` in `src/agent/compaction/mod.rs` runs after `get_compacted_history()` builds the final message list. Removes `role="tool"` messages whose `tool_call_id` has no matching assistant `tool_calls`/`tool_use` block, and counts (but doesn't remove) assistant tool_calls with no matching tool result. Handles both OpenAI-style `tool_calls` arrays and Anthropic-style `content` arrays with `tool_use` blocks. Returns `(orphaned_results_removed, orphaned_calls_found)`.
- **Remember fast path**: `src/agent/memory/remember/mod.rs`. Six trigger patterns (case-insensitive): "remember that ", "remember: ", "please remember ", "don't forget ", "note that ", "keep in mind ". Bypasses LLM entirely — writes directly to daily notes. Rejects: content < 8 chars, questions ending with `?`, interrogative forms (when/how/what/why/if/whether). Deduplication via Jaccard similarity (threshold 0.7) against recent DB entries. Intercepts messages in `process_message()` before image encoding.
- **Memory quality gates**: `src/agent/memory/quality/mod.rs`. `check_quality()` returns `QualityVerdict`: `Pass`, `Reframed(String)`, or `Reject(RejectReason)`. Rejects greetings/filler (exact match after punctuation stripping, ~45 patterns), content < 15 chars. Reframes negative memories ("was broken", "crashed", etc.) unless they already contain constructive markers ("fixed by", "workaround:", "TODO:"). `filter_lines()` applies quality gates per-line for multi-line LLM output. Integrated in `try_remember_fast_path()` and pre-compaction flush.
- **Echo gateway mode**: `oxicrab gateway --echo` starts all channels and HTTP API without an LLM provider. Responds with `[echo] channel={} | sender={} | message: {}` format. Useful for testing channel connectivity. A2A is not available in echo mode.
- **Fuzz testing**: `fuzz/` directory with 5 `cargo-fuzz` targets: `fuzz_webhook_signature`, `fuzz_config_parse`, `fuzz_prompt_guard`, `fuzz_leak_detector`, `fuzz_url_validation`. Run with `cargo fuzz run <target> -- -max_total_time=30`. CI runs each for 30s (informational, `continue-on-error`). `pub mod fuzz_api` in `src/lib.rs` re-exports `validate_and_resolve` and `validate_webhook_signature` for fuzz access — this module is `#[doc(hidden)]` and not public API.
- **Workspace file routing**: Files written to workspace category directories (`code/`, `documents/`, `data/`, `images/`, `downloads/`, `temp/`) are tracked in the `workspace_files` SQLite table. `WorkspaceManager` provides category inference (by extension), path resolution (`{category}/{YYYY-MM-DD}/{filename}`), manifest tracking, and lifecycle cleanup. Reserved dirs (`memory/`, `knowledge/`, `skills/`, `sessions/`) are NOT managed by workspace manager. TTL config in `agents.defaults.workspaceTtl`. `WriteFileTool` auto-registers files, `ReadFileTool` updates `accessed_at`. Hygiene runs at startup (search log purge + workspace file cleanup).
- **Interactive buttons (unified)**: `add_buttons` tool in `src/agent/tools/interactive/mod.rs`. `PendingButtons = Arc<Mutex<Option<Vec<ButtonSpec>>>>` shared between the tool and agent loop. Tool stores button specs (max 5); after the loop completes, `take_pending_buttons_metadata()` in `iteration.rs` drains them into `AgentLoopResult.response_metadata["buttons"]`. `processing.rs` merges response_metadata into the outbound message via `OutboundMessageBuilder::merge_metadata()`. Both Slack and Discord channels read `metadata["buttons"]` (unified format: `[{id, label, style}]`). `bus::meta::BUTTONS` constant for the key. Registration: `register_interactive()` in `setup/mod.rs`.
- **Slack Block Kit buttons**: `convert_buttons_to_blocks()` in `src/channels/slack/mod.rs` converts unified `metadata["buttons"]` to Block Kit JSON: a `section` block with message text + an `actions` block with button elements. Style mapping: `"primary"` → `"primary"`, `"danger"` → `"danger"`, others → omitted (Slack only supports primary/danger). When blocks are present, `send()` uses `send_slack_api_json_with_retry()` (JSON body, not form encoding) since nested `blocks` objects require JSON. Buttons attach to the last message chunk.
- **Slack interactive payloads**: Socket Mode handler processes `type: "interactive"` envelopes alongside `events_api`. `handle_interactive_payload()` parses `block_actions` payloads, extracts `action_id` from `actions[0]`, creates `InboundMessage` with content `[button:{action_id}]` and appropriate metadata (`is_group`, `ts`, `user_id`). Same access control checks (`check_dm_access`/`check_group_access`) as regular messages.
- **Slack reaction emoji lifecycle**: Configurable via `SlackConfig.thinking_emoji` (default `"eyes"`, camelCase: `thinkingEmoji`) and `done_emoji` (default `"white_check_mark"`, camelCase: `doneEmoji`). Inbound: thinking emoji added via `reactions.add` when message received. Outbound: after successful send, thinking emoji removed via `reactions.remove` and done emoji added via `reactions.add`. Both reaction calls are fire-and-forget spawns. Requires inbound message `ts` in metadata.
- **Slack error classification**: `SlackApiError` enum in `src/channels/slack/mod.rs` with variants: `RateLimited { retry_after_secs }`, `InvalidAuth`, `MissingScope(String)`, `ChannelNotFound`, `ServerError(u16)`, `Other(String)`. `classify_slack_error(http_status, error_field)` classifies responses. `is_retryable()` returns true only for `ServerError(5xx)`. `send_slack_api_with_retry()` and `send_slack_api_json_with_retry()` wrap API calls with up to 3 retries for transient errors. Rate limit (429) logs warning but doesn't retry.
- **Slack subtype filtering**: `IGNORED_SUBTYPES` const (14 entries) replaces the old overly-restrictive filter. Ignored: `bot_message`, `message_changed`, `message_deleted`, `channel_join/leave/topic/purpose/name/archive/unarchive`, `group_join/leave`, `ekm_access_denied`, `me_message`. Unknown subtypes pass through (safe default = process), allowing `file_share`, `thread_broadcast`, etc.
- **Discord unified button fallback**: `parse_components_from_metadata()` checks `discord_components` first (backward-compatible), then falls back to `parse_unified_buttons()` which converts unified `metadata["buttons"]` to Discord `CreateActionRow`s. Same fallback in `components_to_api_json()` for interaction followups. Style mapping: `"primary"` → Primary, `"success"` → Success, `"danger"` → Danger, default → Secondary.
