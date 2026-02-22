# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development

Requires **Rust nightly** (pinned to `nightly-2026-02-06` in CI) and system deps: `cmake`. Voice transcription also requires `ffmpeg`. TLS uses rustls (pure Rust, no OpenSSL dependency).

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

Tag-based releases via `scripts/release.sh`. Pushing a `v*` tag triggers `.github/workflows/release.yml` which builds multi-platform binaries (Linux x86_64, Linux ARM64, macOS ARM64), pushes a Docker image to GHCR, generates a changelog with git-cliff, and creates a GitHub Release.

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
- Each tool stores its own `reqwest::Client` as a struct field (connection pooling)
- Google tools share a `GoogleApiClient` wrapper (`src/agent/tools/google_common.rs`)
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
- Implement `Tool` trait: `name()`, `description()`, `version()`, `parameters()`, `execute(params, ctx)`
- Action-based tools use `params["action"].as_str()` dispatch pattern (e.g. GitHub tool has 11 actions: list_issues, create_issue, get_issue, list_prs, get_pr, get_pr_files, create_pr_review, get_file_content, trigger_workflow, get_workflow_runs, notifications)
- Registration: Each module has a `register_*()` function in `src/agent/tools/setup.rs`

### Error Handling
- Internal functions use `anyhow::Result`; module boundaries use `OxicrabError`
- Return `ToolResult::error(...)` for user-facing tool errors (not `Err(...)`)
- Use `Err(anyhow::anyhow!(...))` or `anyhow::bail!(...)` for internal failures

### Unit Test Organization
Two patterns, one convention: **inline** for small test suites, **directory module** for extracted tests.

- **Inline** (small tests): `#[cfg(test)] mod tests { ... }` at the bottom of the source file. Use this when tests are short and closely tied to the source.
- **Directory module** (extracted tests): Convert `foo.rs` to `foo/mod.rs` + `foo/tests.rs` with `#[cfg(test)] mod tests;` in `mod.rs`. Use this when the test suite is large enough to warrant a separate file.

Do **not** use `#[path = "foo_tests.rs"]` — this was previously used in 4 modules but has been standardized to the directory module pattern. The module path (`crate::config::loader`) is unchanged whether `loader` is a file or directory.

## Common Pitfalls

- **Docs are part of the implementation**: No feature, tool change, config change, or CLI change is complete until docs are updated. **Only edit `docs/_pages/*.html`** (the source files) — `docs/*.html` are generated by `python3 docs/build.py`. The docs site is the **source of truth for detail** (tool actions, CLI flags, config fields). README is the **summary** — it lists tool/command names and one-line descriptions, linking to the docs for full reference. This means:
  - **Tool action lists** → only in `docs/_pages/tools.html` (NOT in README, NOT in `_pages/index.html` data-detail attributes)
  - **CLI flag tables** → only in `docs/_pages/cli.html` (README shows example commands only)
  - **Config field tables** → only in `docs/_pages/config.html` (models, credentials, agent defaults, cost guard, circuit breaker, logging)
  - **README** → update the tool/command name lists and one-line descriptions; keep it concise
  - **`_pages/index.html`** → update feature rows and tool grid short descriptions (no action lists)
  - **CLAUDE.md** → update architecture/patterns sections if internal behavior changed
- **Adding fields to `AgentLoopConfig`**: must update `src/cli/commands.rs` (`setup_agent`), destructure in `AgentLoop::new()`, add to `ToolBuildContext` if tool-related, AND update `tests/common/mod.rs` `create_test_agent_with()` AND `tests/compaction_integration.rs` `create_compaction_agent()`.
- **Adding a new tool**: Add a `register_*()` function in `src/agent/tools/setup.rs`, call it from `register_all_tools()`. Update `README.md` and the workspace files (`AGENTS.md`, `MEMORY.md`) if they exist.
- **Adding fields to config structs with manual `Default` impl**: update both the struct definition and `Default::default()`.
- **YAML parsing**: uses `serde_yaml_ng` (not the deprecated `serde_yaml`).
- **`main.rs` is a thin entry point**: it calls `oxicrab::cli::commands::run()`. All module declarations are in `lib.rs`.
- **UTF-8 string slicing**: always use `is_char_boundary()` or `chars()` before slicing.
- **Tool execution**: wrapped in `tokio::task::spawn` for panic isolation via `ToolRegistry::execute_with_guards()`.
- **MemoryDB**: holds a persistent `std::sync::Mutex<Connection>`, not per-operation connections.
- **Cron 5-field expressions**: `compute_next_run()` normalizes by prepending "0 " for the seconds field.
- **No `#[allow(dead_code)]`**: Do not add `#[allow(dead_code)]` or `#![allow(dead_code)]` anywhere. If code is unused, remove it. CI runs `clippy -D warnings` which catches dead code.
- **Empty `allowFrom` is now deny-all**: Channels with empty `allowFrom` will reject all senders. Add `["*"]` for the old behavior, set `"dmPolicy": "pairing"` to let unknown senders request access, or set `"dmPolicy": "open"` to allow everyone.
- **Adding a new credential**: Add one line to `define_credentials!` in `src/config/credentials/mod.rs`. This auto-generates env var override, keyring access, credential helper lookup, CLI listing, and source detection.
- **Anthropic prompt caching is fully implemented**: `cache_control: {"type": "ephemeral"}` is injected on the system prompt block (via `system_to_content_blocks()`) and the last tool definition (via `convert_tools()`) in `src/providers/anthropic_common/mod.rs`. Both the API-key and OAuth providers use these functions. Cache token usage is parsed from responses (`cache_creation_input_tokens`, `cache_read_input_tokens`), tracked in `CostGuard` (reads at 10%, creation at 125% of input rate), and persisted to the `llm_cost_log` SQLite table.
- **`record_llm_call()` takes cache token params**: `cache_creation_input_tokens` and `cache_read_input_tokens` (both `Option<u64>`) for Anthropic prompt caching cost tracking.
- **CostGuard persists to SQLite**: Use `CostGuard::with_db()` (not `::new()`) to enable cost persistence. Daily cost is restored on startup. Records go to `llm_cost_log` table in `memory.sqlite3`.
- **Memory search tracking**: All searches (keyword and hybrid) are logged to `memory_access_log` + `memory_search_hits` tables. Use `db.get_source_hit_count()` to check utility. The `archive_old_notes()` function takes an optional `db` parameter for utility-based early archiving.
- **Embedding back-fill**: `MemoryIndexer` automatically back-fills embeddings for entries that were indexed before embeddings were enabled, via `get_entries_missing_embeddings()`.
- **CLI `stats` command**: `oxicrab stats today|costs|search` queries the memory database for cost and search metrics.
- **Cron metadata propagation**: `CronPayload.origin_metadata` captures the originating inbound message's metadata (e.g. Slack `ts`, WhatsApp `message_id`) when jobs are created. Propagated to `OutboundMessage.metadata` when jobs fire, so responses land in the correct thread/context. `ExecutionContext.metadata` carries inbound message metadata to tools.
- **`reasoning_content` preserved across message lifecycle**: The `Message` struct has a `reasoning_content: Option<String>` field. Anthropic thinking blocks are parsed in `parse_response()`, carried through the agent loop, converted back to `{"type": "thinking"}` content blocks in `convert_messages()`, and restored from session history in `build_messages()`. OpenAI provider parses DeepSeek-R1's `reasoning_content` field. Use `Message::assistant_with_thinking()` to construct messages with reasoning content.
- **Group chat memory isolation**: Channels set `is_group` in inbound message metadata (Telegram: `chat.is_group()/is_supergroup()`, Discord: `guild_id.is_some()`, Slack: channel not starting with 'D'). `build_messages()` accepts `is_group: bool` and delegates to `build_system_prompt_inner()` which calls `get_memory_context_scoped(query, true)`. In group mode: MEMORY.md excluded from search results and content, daily notes excluded from search results and content, `is_daily_note_key()` helper identifies `YYYY-MM-DD.md` patterns.
- **Hybrid search fusion strategy**: `FusionStrategy` enum in `src/config/schema/agent.rs` with `WeightedScore` (default, linear blend) and `Rrf` (reciprocal rank fusion). Config fields: `searchFusionStrategy` ("weighted_score" or "rrf"), `rrfK` (default 60). Threaded through `MemoryStore` → `MemoryDB::hybrid_search()`.
- **Embedding query cache**: `EmbeddingService` has an LRU cache for `embed_query()` results. Default 10,000 entries, configurable via `agents.defaults.memory.embeddingCacheSize`. `EmbeddingService::with_cache_size()` constructor accepts custom size. `embed_texts()` (batch indexing) is not cached.
- **JSON mode / structured output**: `ResponseFormat` enum in `src/providers/base.rs` with `JsonObject` and `JsonSchema { name, schema }` variants. `ChatRequest` has `response_format: Option<ResponseFormat>`. Provider handling: OpenAI sets `response_format` payload field (`json_object` or `json_schema` with strict mode). Gemini sets `generationConfig.responseMimeType` to `application/json` (+ `responseSchema` for `JsonSchema`). Anthropic (both API key and OAuth) appends a system prompt hint since there is no native JSON mode parameter. Passthrough providers (fallback, prompt-guided, circuit breaker) forward the field. Currently set to `None` at all call sites — tools or future features can opt in per-request.
- **PDF/document support**: `load_and_encode_images()` in `src/agent/loop/helpers.rs` accepts `.pdf` files (validates `%PDF` magic bytes, same 20MB limit as images). `ImageData` struct carries any MIME type. Anthropic provider uses `"type": "document"` for non-image media (vs `"type": "image"`). OpenAI uses `"type": "file"` with data URI. Gemini uses same `inline_data` format for all types. Agent loop strips `[document: ...]` tags via `strip_document_tags()` after encoding. Channels (Telegram, WhatsApp) already download PDFs to `~/.oxicrab/media/`.
- **Gateway HTTP API**: `src/gateway/mod.rs` provides an axum-based REST server with `POST /api/chat`, `GET /api/health`, and `POST /api/webhook/{name}`. `HttpApiState` holds `inbound_tx` (to publish to the agent), `pending` map for oneshot response channels, `webhooks` config map, and optional `outbound_tx` for target delivery. `chat_handler` creates a oneshot channel, stores the sender in the pending map keyed by request ID (`http-{uuid}`), publishes an `InboundMessage` with `channel="http"`, and awaits the receiver with a 120s timeout. `route_response()` intercepts outbound messages where `channel=="http"`, routes them to the matching pending oneshot, and returns `true` (consumed). Called in `start_channels_loop` before channel dispatch. `start()` takes `inbound_tx`, optional `outbound_tx`, and webhooks config; returns `(JoinHandle, HttpApiState)`. Axum and `hmac` are non-optional dependencies (used by gateway, webhooks, and Twilio).
- **Webhook receiver**: Named webhooks configured in `gateway.webhooks` (`WebhookConfig` in `src/config/schema/mod.rs`). Each webhook has a `secret` (HMAC-SHA256), `template` (`{{key}}` substitution from JSON payload, `{{body}}` for raw), `targets` (channel + `chatId` pairs), and optional `agentTurn` flag. Signature validated via constant-time comparison (`subtle::ConstantTimeEq`); checks `X-Signature-256`, `X-Hub-Signature-256`, and `X-Webhook-Signature` headers, supports `sha256=` prefix. Max payload 1MB. When `agentTurn` is true, message routes through agent loop then delivers response to targets via `outbound_tx`. When false, templated message delivers directly to targets.
- **Resource limits (OOM prevention)**: Context files (USER.md, TOOLS.md, AGENTS.md): 500KB max. Skill files (SKILL.md): 1MB max. Audio uploads (cloud transcription): 25MB max. Base64 images (image generation): 30MB pre-decode check. HTML content (browser tool): 500KB max. Browser screenshot: 10080px height clamp. HTTP response bodies: 10MB max via `limited_body()`.
- **Tool name constraints**: Tool names must be ≤256 chars with no null, newline, or control characters. Enforced at registration time in `ToolRegistry`.
- **Tool cache key format**: `name#len:params` — length-prefixed to prevent collision between `tool("ab")` and `tool_a("b")`.
- **Input validation patterns (hardening)**: Strip `\r`/`\n` from email headers (Gmail). Reject URLs with embedded credentials. Filter control characters from sender IDs. Sanitize API error messages before returning to LLM (GitHub). Validate file paths against traversal (Reddit, Todoist, Skills). Reject `system` role messages in conversation history.
- **MCP timeouts**: Server handshake: 30s. Tool discovery: 10s per server. Applied in `McpManager`.
