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

## Releasing

Tag-based releases via `scripts/release.sh`. Pushing a `v*` tag triggers `.github/workflows/release.yml` which builds multi-platform binaries (Linux x86_64, macOS x86_64, macOS ARM64), pushes a Docker image to GHCR, generates a changelog with git-cliff, and creates a GitHub Release.

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

Multi-channel AI assistant that connects chat platforms to LLM providers via an agent loop.

### Core Flow

```
Channel (Telegram/Discord/Slack/WhatsApp/Twilio)
  → MessageBus (inbound queue)
    → AgentLoop (iterates: LLM call → tool execution → repeat)
      → MessageBus (outbound queue)
        → Channel (reply)
```

### Key Abstractions (3 traits + middleware)

- **`Tool`** (`src/agent/tools/base.rs`): `name()`, `description()`, `parameters()` (JSON Schema), `execute(Value, &ExecutionContext) → ToolResult`. Optional: `cacheable()`.
- **`ToolMiddleware`** (`src/agent/tools/base.rs`): `before_execute()` (can short-circuit), `after_execute()` (can modify result). Built-in: `CacheMiddleware`, `TruncationMiddleware`, `LoggingMiddleware`.
- **`ExecutionContext`** (`src/agent/tools/base.rs`): Passed to every `execute()` call. Fields: `channel`, `chat_id`, `context_summary`.
- **`BaseChannel`** (`src/channels/base.rs`): `start()`, `stop()`, `send()`. Optional: `send_typing()`, `send_and_get_id()`, `edit_message()`, `delete_message()`. Discord supports slash commands, button component interactions, embeds, and interaction webhook followups — metadata keys `discord_interaction_token`/`discord_application_id` route responses through webhook API instead of channel messages.
- **`LLMProvider`** (`src/providers/base.rs`): `chat(ChatRequest) → LLMResponse`, `default_model()`, `warmup()`. Has default `chat_with_retry()` with exponential backoff. `warmup()` pre-warms HTTP connections on startup (default no-op, implemented for Anthropic/OpenAI/Gemini).

### Provider Selection

`ProviderFactory` in `src/providers/strategy.rs` picks provider by model name prefix. Tries Anthropic OAuth first, falls back to API key strategy. Within the API key strategy, OpenAI-compatible providers (OpenRouter, DeepSeek, Groq, Moonshot, Zhipu, DashScope, vLLM) are matched first by keyword in the model name, then native providers (Anthropic, OpenAI, Gemini). OpenAI-compat providers use `OpenAIProvider::with_config()` with a configurable base URL (defaulting per-provider) and provider name for error messages.

### Tool System

- **`ToolRegistry`** (`src/agent/tools/registry.rs`): Central execution engine. Runs middleware pipeline: `before_execute` → `execute_with_guards` (timeout + panic isolation via `tokio::task::spawn`) → `after_execute`. Stored as `Arc<ToolRegistry>` (immutable after construction).
- **`ToolBuildContext`** (`src/agent/tools/setup.rs`): Aggregates all config needed for tool construction. `register_all_tools()` calls per-module registration functions.
- **MCP** (`src/agent/tools/mcp/`): `McpManager` connects to external MCP servers via child processes (`rmcp` crate). `McpProxyTool` wraps each discovered tool as `impl Tool`. Config under `tools.mcp.servers`.

### Agent Loop (`src/agent/loop.rs`)

`AgentLoop::new(AgentLoopConfig)` runs up to `max_iterations` (default 20) of: LLM call → parallel tool execution → append to conversation. Tool execution is delegated to `ToolRegistry::execute()` which handles caching, truncation (10k chars), timeout, panic isolation, and logging via the middleware pipeline. First iteration forces `tool_choice="any"` to prevent text-only hallucinations. Tools nudge (up to 2 retries) catches subsequent iterations where the LLM returns text without having called any tools. Hallucination detection runs on final text responses. Responses flow through the loop's return value (no message tool); the caller sends them exactly once.

### Feature Flags (channel selection)

```toml
default = ["channel-telegram", "channel-discord", "channel-slack", "channel-whatsapp", "channel-twilio"]
channel-telegram = ["dep:teloxide"]
channel-discord = ["dep:serenity"]
channel-slack = ["dep:tokio-tungstenite"]
channel-whatsapp = ["dep:whatsapp-rust", ...]
channel-twilio = ["dep:axum", "dep:hmac", "dep:sha1"]
```

Channels are conditionally compiled via `#[cfg(feature = "channel-*")]` in `src/channels/mod.rs`.

### Voice Transcription (`src/utils/transcription.rs`)

`TranscriptionService` supports two backends: local (whisper-rs + ffmpeg) and cloud (Whisper API). Routing controlled by `prefer_local` config flag — tries preferred backend first, falls back to the other. Local inference runs whisper.cpp via `spawn_blocking`; audio converted to 16kHz mono f32 PCM via ffmpeg subprocess. `TranscriptionService::new()` returns `Some` if at least one backend is available.

### Config

JSON at `~/.oxicrab/config.json` (or `OXICRAB_HOME` env var). Uses camelCase in JSON, snake_case in Rust (serde `rename` attrs). Schema in `src/config/schema.rs` — 11 structs have custom `Debug` impls that redact secrets. Validated on startup via `config.validate()`.

### Error Handling

`OxicrabError` in `src/errors.rs` — typed variants: `Config`, `Provider { retryable }`, `RateLimit { retry_after }`, `Auth`, `Internal(anyhow::Error)`. See [Code Style & Patterns](#code-style--patterns) for usage conventions.

### CostGuard (`src/agent/cost_guard.rs`)

Pre-flight budget gating and post-flight cost tracking. `CostGuard::check_allowed()` blocks if daily budget exceeded or hourly rate limit hit. `record_llm_call()` updates counters after each LLM call. Embedded `pricing_data.json` covers 50+ models; config overrides via `agents.defaults.costGuard.modelCosts`. Daily budget resets at midnight UTC. AtomicBool fast-path skips mutex when budget already exceeded. Config fields (all optional): `dailyBudgetCents` (u64), `maxActionsPerHour` (u64), `modelCosts` (HashMap of prefix → {input_per_million, output_per_million}).

### Circuit Breaker (`src/providers/circuit_breaker.rs`)

`CircuitBreakerProvider::wrap(inner, config)` returns `Arc<dyn LLMProvider>` wrapping the inner provider. Three states: Closed (passes through), Open (rejects immediately after `failure_threshold` consecutive transient failures), HalfOpen (allows `half_open_probes` test requests after `recovery_timeout_secs`). Transient errors: 429, 5xx, timeout, connection refused/reset. Non-transient errors (auth, invalid key, permission, context length) do **not** trip the breaker. Config under `providers.circuitBreaker`: `enabled` (default false), `failureThreshold` (default 5), `recoveryTimeoutSecs` (default 60), `halfOpenProbes` (default 2).

### Doctor (`src/cli/doctor.rs`)

`oxicrab doctor` — system diagnostics command. Checks: config exists/parses/validates, workspace writable, provider API keys configured, provider connectivity (warmup with latency), per-channel status (compiled + enabled + tokens), voice transcription backends, external tools (ffmpeg, git), MCP servers. Includes security audit: config file permissions, directory permissions, empty allowlists, pairing store status. Output: PASS/FAIL/SKIP per check with summary counts. Returns exit code 1 if config file missing.

### Security Hardening

- **Env var overrides** (`src/config/loader.rs`): `apply_env_overrides()` checks `OXICRAB_*` env vars after deserialization, before validation. Env vars take precedence over config file values. Supported: `OXICRAB_ANTHROPIC_API_KEY`, `OXICRAB_OPENAI_API_KEY`, `OXICRAB_OPENROUTER_API_KEY`, `OXICRAB_GEMINI_API_KEY`, `OXICRAB_DEEPSEEK_API_KEY`, `OXICRAB_GROQ_API_KEY`, `OXICRAB_TELEGRAM_TOKEN`, `OXICRAB_DISCORD_TOKEN`, `OXICRAB_SLACK_BOT_TOKEN`, `OXICRAB_SLACK_APP_TOKEN`, `OXICRAB_TWILIO_ACCOUNT_SID`, `OXICRAB_TWILIO_AUTH_TOKEN`, `OXICRAB_GITHUB_TOKEN`.
- **Default-deny allowlists** (`src/channels/utils.rs`): Empty `allowFrom` arrays now deny all senders. Use `["*"]` for open access.
- **DM pairing** (`src/pairing/mod.rs`): `PairingStore` provides file-backed per-channel allowlists at `~/.oxicrab/pairing/`. 8-char human-friendly codes with 15-min TTL. CLI: `oxicrab pairing list|approve|revoke`.
- **Leak detection** (`src/safety/leak_detector.rs`): `LeakDetector` scans outbound messages for API key patterns (Anthropic, OpenAI, Slack, GitHub, Groq, Telegram, Discord). Integrated into `MessageBus::publish_outbound()` — redacts before sending.
- **Config permissions**: `check_file_permissions()` warns on startup if config file is world-readable (unix). `save_config()` uses atomic writes via `crate::utils::atomic_write()`.
- **Constant-time comparison**: Twilio webhook signature uses `subtle::ConstantTimeEq` instead of `==`.

### CLI Commands

`oxicrab gateway` — full multi-channel daemon. `oxicrab agent -m "message"` — single-turn CLI. `oxicrab onboard` — first-time setup. `oxicrab cron` — manage cron jobs. `oxicrab auth` — OAuth flows. `oxicrab channels` — channel status and WhatsApp login. `oxicrab status` — quick setup overview. `oxicrab doctor` — system diagnostics. `oxicrab pairing` — manage DM pairing for sender authentication (list/approve/revoke).

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
- Action-based tools use `params["action"].as_str()` dispatch pattern (e.g. GitHub tool has 11 actions: list_issues, create_issue, list_prs, get_pr, get_issue, get_pr_files, create_pr_review, get_file_content, trigger_workflow, get_workflow_runs, search_repos)
- Registration: Each module has a `register_*()` function in `src/agent/tools/setup.rs`

### Error Handling
- Internal functions use `anyhow::Result`; module boundaries use `OxicrabError`
- Return `ToolResult::error(...)` for user-facing tool errors (not `Err(...)`)
- Use `Err(anyhow::anyhow!(...))` or `anyhow::bail!(...)` for internal failures

## Common Pitfalls

- **Adding fields to `AgentLoopConfig`**: must update `src/cli/commands.rs` (`setup_agent`), destructure in `AgentLoop::new()`, add to `ToolBuildContext` if tool-related, AND update `tests/common/mod.rs` `create_test_agent()` AND `tests/compaction_integration.rs` `create_compaction_agent()`.
- **Adding a new tool**: Add a `register_*()` function in `src/agent/tools/setup.rs`, call it from `register_all_tools()`. Update `README.md` and the workspace files (`AGENTS.md`, `MEMORY.md`) if they exist.
- **Keeping docs in sync**: When making user-facing changes (new features, config changes, CLI changes, new tools, channels), update `README.md`, `CLAUDE.md`, and the relevant docs pages (`docs/*.html`). The docs site has pages for channels (`docs/channels.html`), tools (`docs/tools.html`), and deployment (`docs/deploy.html`).
- **Adding fields to config structs with manual `Default` impl**: update both the struct definition and `Default::default()`.
- **YAML parsing**: uses `serde_yaml_ng` (not the deprecated `serde_yaml`).
- **`main.rs` and `lib.rs` both declare `mod errors`**: binary has its own module tree.
- **UTF-8 string slicing**: always use `is_char_boundary()` or `chars()` before slicing.
- **Tool execution**: wrapped in `tokio::task::spawn` for panic isolation via `ToolRegistry::execute_with_guards()`.
- **MemoryDB**: holds a persistent `std::sync::Mutex<Connection>`, not per-operation connections.
- **Cron 5-field expressions**: `compute_next_run()` normalizes by prepending "0 " for the seconds field.
- **No `#[allow(dead_code)]`**: Do not add `#[allow(dead_code)]` or `#![allow(dead_code)]` anywhere. If code is unused, remove it. CI runs `clippy -D warnings` which catches dead code.
- **Empty `allowFrom` is now deny-all**: Channels with empty `allowFrom` will reject all senders. Add `["*"]` for the old behavior, or use the pairing system.
