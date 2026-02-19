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

`ProviderFactory` in `src/providers/strategy.rs` picks provider by model name prefix. Tries Anthropic OAuth first, falls back to API key strategy. Within the API key strategy, OpenAI-compatible providers (OpenRouter, DeepSeek, Groq, Moonshot, Zhipu, DashScope, Ollama, vLLM) are matched first by keyword in the model name, then native providers (Anthropic, OpenAI, Gemini). OpenAI-compat providers use `OpenAIProvider::with_config()` with a configurable base URL (defaulting per-provider) and provider name for error messages. Custom HTTP headers can be injected into all requests via `ProviderConfig.headers` — passed through to `OpenAIProvider::with_config_and_headers()`. When `ProviderConfig.prompt_guided_tools` is true (currently checked for Ollama/vLLM), `PromptGuidedToolsProvider::wrap()` is applied — this injects tool definitions into the system prompt and parses `<tool_call>` XML blocks from text responses, enabling tool use with local models that lack native function calling support.

### Tool System

- **`ToolRegistry`** (`src/agent/tools/registry.rs`): Central execution engine. Runs middleware pipeline: `before_execute` → `execute_with_guards` (timeout + panic isolation via `tokio::task::spawn`) → `after_execute`. Stored as `Arc<ToolRegistry>` (immutable after construction).
- **`ToolBuildContext`** (`src/agent/tools/setup.rs`): Aggregates all config needed for tool construction. `register_all_tools()` calls per-module registration functions.
- **MCP** (`src/agent/tools/mcp/`): `McpManager` connects to external MCP servers via child processes (`rmcp` crate). `McpProxyTool` wraps each discovered tool as `impl Tool`. Config under `tools.mcp.servers`. Each server has a `sandbox` field (`SandboxConfig`) for Landlock kernel-level sandboxing of the child process (enabled by default). `McpManager::new()` takes a workspace path; `McpProxyTool` sanitizes error messages via `path_sanitize`.

### Agent Loop (`src/agent/loop/mod.rs`)

`AgentLoop::new(AgentLoopConfig)` runs up to `max_iterations` (default 20) of: LLM call → parallel tool execution → append to conversation. Tool execution is delegated to `ToolRegistry::execute()` which handles caching, truncation (10k chars), timeout, panic isolation, and logging via the middleware pipeline. First iteration forces `tool_choice="any"` to prevent text-only hallucinations. Tools nudge (up to 2 retries) catches subsequent iterations where the LLM returns text without having called any tools. Hallucination detection runs on final text responses. Responses flow through the loop's return value (no message tool); the caller sends them exactly once. At 70% of `max_iterations`, a system message prompts the LLM to begin wrapping up. Post-compaction recovery instructions include the last user message and most recent checkpoint. Periodic checkpoints (configurable via `CompactionConfig.checkpoint`) snapshot conversation state every N iterations for recovery after compaction.

### Feature Flags (channel selection + optional features)

```toml
default = ["channel-telegram", "channel-discord", "channel-slack", "channel-whatsapp", "channel-twilio", "keyring-store"]
channel-telegram = ["dep:teloxide"]
channel-discord = ["dep:serenity"]
channel-slack = ["dep:tokio-tungstenite"]
channel-whatsapp = ["dep:whatsapp-rust", ...]
channel-twilio = ["dep:axum", "dep:hmac", "dep:sha1"]
keyring-store = ["dep:keyring"]
```

Channels are conditionally compiled via `#[cfg(feature = "channel-*")]` in `src/channels/mod.rs`. Keyring support (`keyring-store`) is default-on for desktop; containers should build with `--no-default-features` and use env vars instead.

### Voice Transcription (`src/utils/transcription.rs`)

`TranscriptionService` supports two backends: local (whisper-rs + ffmpeg) and cloud (Whisper API). Routing controlled by `prefer_local` config flag — tries preferred backend first, falls back to the other. Local inference runs whisper.cpp via `spawn_blocking`; audio converted to 16kHz mono f32 PCM via ffmpeg subprocess. `TranscriptionService::new()` returns `Some` if at least one backend is available.

### Config

JSON at `~/.oxicrab/config.json` (or `OXICRAB_HOME` env var). Uses camelCase in JSON, snake_case in Rust (serde `rename` attrs). Schema in `src/config/schema.rs` — 15 structs have custom `Debug` impls (via `redact_debug!` macro) that redact secrets. Validated on startup via `config.validate()`. Notable config fields: `providers.*.headers` (custom HTTP headers for OpenAI-compatible providers), `agents.defaults.compaction.checkpoint` (`CheckpointConfig` with `enabled` and `intervalIterations`), `tools.exfiltrationGuard` (`ExfiltrationGuardConfig` with `enabled` and `blockedTools`), `tools.exec.sandbox` (`SandboxConfig` with `enabled`, `additionalReadPaths`, `additionalWritePaths`, `blockNetwork`), `agents.defaults.promptGuard` (`PromptGuardConfig` with `enabled` and `action`).

### Error Handling

`OxicrabError` in `src/errors.rs` — typed variants: `Config`, `Provider { retryable }`, `RateLimit { retry_after }`, `Auth`, `Internal(anyhow::Error)`. See [Code Style & Patterns](#code-style--patterns) for usage conventions.

### CostGuard (`src/agent/cost_guard.rs`)

Pre-flight budget gating and post-flight cost tracking. `CostGuard::check_allowed()` blocks if daily budget exceeded or hourly rate limit hit. `record_llm_call()` updates counters after each LLM call. Embedded `pricing_data.json` covers 34 models; config overrides via `agents.defaults.costGuard.modelCosts`. Daily budget resets at midnight UTC. AtomicBool fast-path skips mutex when budget already exceeded. Config fields (all optional): `dailyBudgetCents` (u64), `maxActionsPerHour` (u64), `modelCosts` (HashMap of prefix → {input_per_million, output_per_million}).

### Circuit Breaker (`src/providers/circuit_breaker.rs`)

`CircuitBreakerProvider::wrap(inner, config)` returns `Arc<dyn LLMProvider>` wrapping the inner provider. Three states: Closed (passes through), Open (rejects immediately after `failure_threshold` consecutive transient failures), HalfOpen (allows `half_open_probes` test requests after `recovery_timeout_secs`). Transient errors: 429, 5xx, timeout, connection refused/reset. Non-transient errors (auth, invalid key, permission, context length) do **not** trip the breaker. Config under `providers.circuitBreaker`: `enabled` (default false), `failureThreshold` (default 5), `recoveryTimeoutSecs` (default 60), `halfOpenProbes` (default 2).

### Cognitive Routines (`src/agent/cognitive.rs`)

`CheckpointTracker` emits escalating pressure messages that nudge the LLM to self-checkpoint during long tool-heavy agent loop runs. Tracks tool call volume with a rolling window (`recent_tools_window`, default 10) and fires three one-shot pressure levels: gentle hint (`gentleThreshold`, default 12), firm warning (`firmThreshold`, default 20), urgent demand (`urgentThreshold`, default 30). Each level emits only once per cycle; counters reset when a periodic checkpoint fires. The tracker is local to each `run_agent_loop()` invocation (not persisted). A `breadcrumb()` method produces a cognitive state summary injected into compaction recovery context. Static cognitive instructions are injected as a system message when enabled. Config under `agents.defaults.cognitive`: `enabled` (default false), thresholds, `recentToolsWindow`.

### Doctor (`src/cli/doctor.rs`)

`oxicrab doctor` — system diagnostics command. Checks: config exists/parses/validates, workspace writable, provider API keys configured, provider connectivity (warmup with latency), per-channel status (compiled + enabled + tokens), voice transcription backends, external tools (ffmpeg, git), MCP servers. Includes security audit: config file permissions, directory permissions, empty allowlists, pairing store status. Output: PASS/FAIL/SKIP per check with summary counts. Returns exit code 1 if config file missing.

### Credential Registry (`src/config/credentials/mod.rs`)

Unified credential management via `define_credentials!` macro. Adding a new credential = one line in the macro. All backends (env vars, keyring, credential helper) are generated from a single declarative table of 28 credential slots. Resolution order: env var → credential helper → keyring → config.json.

- **`apply_env_overrides()`**: Checks `OXICRAB_*` env vars for all 28 credential slots
- **`apply_credential_helper()`**: Fetches secrets from external processes (1Password, Bitwarden, custom scripts)
- **`apply_keyring_overrides()`** (behind `keyring-store` feature): Loads from OS keychain
- **`detect_source()`**: Identifies where a credential came from (env/keyring/config/helper/empty)
- **`CredentialHelperConfig`** in `src/config/schema.rs`: `command`, `args`, `format` (json/1password/bitwarden/line)

### Security Hardening

- **Credential backends** (`src/config/credentials/mod.rs`): Three-tier credential resolution (env > helper > keyring > config.json). All 28 credential slots covered by `OXICRAB_*` env vars. OS keychain via `keyring` crate (optional, `keyring-store` feature). External helper protocol supports 1Password (`op`), Bitwarden (`bw`), and custom scripts.
- **Default-deny allowlists** (`src/channels/utils.rs`): Empty `allowFrom` arrays now deny all senders. Use `["*"]` for open access.
- **DM policy** (`src/channels/utils.rs`): Per-channel `dmPolicy` field controls access for unknown senders: `"allowlist"` (default, silent deny), `"pairing"` (send pairing code), `"open"` (allow all). `check_dm_access()` returns `DmCheckResult` (Allowed/Denied/PairingRequired). Each channel handles pairing replies natively (Telegram sends message, Discord sends ephemeral response, Slack posts via API, Twilio returns TwiML, WhatsApp logs the code).
- **DM pairing** (`src/pairing/mod.rs`): `PairingStore` provides file-backed per-channel allowlists at `~/.oxicrab/pairing/`. 8-char human-friendly codes with 15-min TTL. Per-client lockout tracking (`HashMap<String, Vec<u64>>`) prevents brute-force code guessing with bounded map (1000 clients max). CLI: `oxicrab pairing list|approve|revoke`.
- **Leak detection** (`src/safety/leak_detector.rs`): `LeakDetector` scans outbound messages for API key patterns (Anthropic, OpenAI, Slack, GitHub, Groq, Telegram, Discord). Three-encoding scanning: plaintext patterns, base64-decoded candidates (20+ chars), and hex-decoded candidates (40+ chars). `add_known_secrets()` registers actual config secret values for exact-match detection across all three encodings. `Config::collect_secrets()` gathers all non-empty API keys and tokens; `setup_message_bus()` passes them to the leak detector at startup via `add_known_secrets()`. Integrated into `MessageBus::publish_outbound()` — redacts before sending.
- **DNS rebinding defense** (`src/utils/url_security/mod.rs`): `validate_and_resolve()` resolves DNS and returns `ResolvedUrl` with pinned `SocketAddr`s. Callers (http, web_fetch tools) build one-shot reqwest clients with `.resolve()` to pin DNS, preventing TOCTOU rebinding attacks where DNS returns a different IP between validation and fetch.
- **Exfiltration guard** (`src/config/schema.rs`): `ExfiltrationGuardConfig` with `enabled` (default false) and `blockedTools` (default: http, web_fetch, browser). When enabled, blocked tools are filtered from `tools_defs` before sending to the LLM, AND blocked at dispatch time in `execute_tool_call()`. Config under `tools.exfiltrationGuard`.
- **Prompt injection detection** (`src/safety/prompt_guard.rs`): `PromptGuard` with regex patterns across 4 categories: role switching, instruction override, secret extraction, jailbreak. Scans user messages in `process_message_unlocked()` (configurable: warn or block) and tool output in `run_agent_loop()` (warn only). Config under `agents.defaults.promptGuard` with `enabled` (default false) and `action` ("warn" or "block").
- **Subprocess env scrubbing** (`src/utils/subprocess.rs`): `scrubbed_command()` calls `env_clear()` then copies only allowlisted vars (`PATH`, `HOME`, `USER`, `LANG`, `LC_ALL`, `TZ`, `TERM`, `RUST_LOG`, `TMPDIR`, `XDG_RUNTIME_DIR`). Applied to all child processes: shell exec, MCP servers, ffmpeg, tmux.
- **HTTP body limits** (`src/utils/http.rs`): `limited_body()` and `limited_text()` stream response bodies with Content-Length pre-check and chunk-based size cap (default 10 MB). Applied to http tool, web_fetch, and web_search.
- **Shell output cap**: Combined stdout+stderr truncated at 1 MB with `[output truncated at 1MB]` marker. PCM audio capped at 50 MB.
- **Shell AST analysis** (`src/utils/shell_ast/mod.rs`): Pre-execution structural analysis via `brush-parser` detects subshells, command/process substitution, `eval`/`source`, interpreter inline execution (`python -c`, `perl -e`), dangerous pipe targets (`| bash`), function definitions, and dangerous device redirections (`> /dev/sda`). Runs before allowlist and regex checks — even allowlisted commands like `python3` are blocked when used with inline exec flags.
- **Shell injection patterns** (`src/utils/regex/mod.rs`): Security blocklist includes patterns for `rm -rf`, raw device access, fork bombs, `eval`, piped downloads, netcat listeners, hex decode to shell, `$VAR` expansion, and input redirection from absolute/home paths.
- **Landlock sandbox** (`src/utils/sandbox/mod.rs`): Linux kernel-level Landlock LSM sandboxing applied to shell commands and MCP server child processes via `pre_exec`. Default read-only: `/usr`, `/lib`, `/lib64`, `/bin`, `/sbin`, `/etc`. Default read-write: workspace dir, `/tmp`, `/var/tmp`. Network blocked by default. Config under `tools.exec.sandbox` (shell) and `tools.mcp.servers.*.sandbox` (MCP): `enabled` (default true), `additionalReadPaths`, `additionalWritePaths`, `blockNetwork` (default true). Degrades gracefully (BestEffort) on older kernels; no-op on non-Linux.
- **Capability-based filesystem confinement** (`src/agent/tools/filesystem/mod.rs`): When `restrict_to_workspace` is enabled, filesystem tools use `cap_std::fs::Dir` (backed by `openat()`) for TOCTOU-safe confined operations. The root directory is opened once, and all subsequent file operations use relative paths through the capability handle, preventing symlink escape and race conditions between validation and access.
- **Workspace path validation** (`src/agent/tools/shell.rs`): When `restrict_to_workspace` is enabled, absolute paths in commands are canonicalized and checked against the workspace boundary.
- **Error path sanitization** (`src/utils/path_sanitize.rs`): `sanitize_path()` and `sanitize_error_message()` redact home directory paths in error messages sent to the LLM. Workspace-relative paths are collapsed to `~/...`, paths outside workspace under home become `<redacted>/filename`, system paths are unchanged. Applied to filesystem tool errors and MCP proxy errors.
- **Config file locking** (`src/config/loader/mod.rs`): `load_config()` acquires a shared (read) lock via `fs2::FileExt`. `save_config()` acquires an exclusive lock via a separate `.json.lock` lockfile (survives atomic renames). Prevents corruption from concurrent config reads/writes.
- **Config permissions**: `check_file_permissions()` warns on startup if config file is world-readable (unix). `save_config()` uses atomic writes via `crate::utils::atomic_write()`.
- **Constant-time comparison**: Twilio webhook signature uses `subtle::ConstantTimeEq` instead of `==`.
- **TruffleHog CI** (`.github/workflows/trufflehog.yml`): Scans for verified secrets on push and pull request.

### CLI Commands

`oxicrab gateway` — full multi-channel daemon. `oxicrab agent -m "message"` — single-turn CLI. `oxicrab onboard` — first-time setup. `oxicrab cron` — manage cron jobs. `oxicrab auth` — OAuth flows. `oxicrab channels` — channel status and WhatsApp login. `oxicrab credentials` — manage credentials (set/get/delete/list/import via OS keychain). `oxicrab status` — quick setup overview. `oxicrab doctor` — system diagnostics. `oxicrab pairing` — manage DM pairing for sender authentication (list/approve/revoke).

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
