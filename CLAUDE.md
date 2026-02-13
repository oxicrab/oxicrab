# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development

Requires **Rust nightly** (pinned to `nightly-2026-02-06` in CI) and system deps: `libssl-dev pkg-config`.

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

Integration tests need `NANOBOT_HOME` set to a temp directory (CI uses `$RUNNER_TEMP/nanobot-test`). Tests use `MockLLMProvider` from `tests/common/mod.rs` and `TempDir` for isolation.

## Linting

```bash
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

CI treats clippy warnings as errors. No custom rustfmt/clippy config — uses defaults.

## Architecture

Multi-channel AI assistant that connects chat platforms to LLM providers via an agent loop.

### Core Flow

```
Channel (Telegram/Discord/Slack/WhatsApp)
  → MessageBus (inbound queue)
    → AgentLoop (iterates: LLM call → tool execution → repeat)
      → MessageBus (outbound queue)
        → Channel (reply)
```

### Key Abstractions (3 traits)

- **`Tool`** (`src/agent/tools/base.rs`): `name()`, `description()`, `parameters()` (JSON Schema), `execute(Value) → ToolResult`. Optional: `cacheable()`, `set_context()`, `set_context_summary()`.
- **`BaseChannel`** (`src/channels/base.rs`): `start()`, `stop()`, `send()`. Optional: `send_typing()`, `send_and_get_id()`, `edit_message()`, `delete_message()`.
- **`LLMProvider`** (`src/providers/base.rs`): `chat(ChatRequest) → LLMResponse`, `default_model()`. Has default `chat_with_retry()` with exponential backoff.

### Provider Selection

`ProviderFactory` in `src/providers/strategy.rs` picks provider by model name prefix. Tries Anthropic OAuth first, falls back to API key strategy.

### Agent Loop (`src/agent/loop.rs`)

`AgentLoop::new(AgentLoopConfig)` runs up to `max_iterations` (default 20) of: LLM call → parallel tool execution → result truncation (10k chars) → append to conversation. First iteration forces `tool_choice="any"` to prevent text-only hallucinations. Hallucination detection runs on final text responses.

### Feature Flags (channel selection)

```toml
default = ["channel-telegram", "channel-discord", "channel-slack", "channel-whatsapp"]
channel-telegram = ["dep:teloxide"]
channel-discord = ["dep:serenity"]
channel-slack = ["dep:tokio-tungstenite"]
channel-whatsapp = ["dep:whatsapp-rust", ...]
```

Channels are conditionally compiled via `#[cfg(feature = "channel-*")]` in `src/channels/mod.rs`.

### Config

JSON at `~/.nanobot/config.json` (or `NANOBOT_HOME` env var). Uses camelCase in JSON, snake_case in Rust (serde `rename` attrs). Schema in `src/config/schema.rs` — 10 structs have custom `Debug` impls that redact secrets. Validated on startup via `config.validate()`.

### Error Handling

`NanobotError` in `src/errors.rs` — typed variants: `Config`, `Provider { retryable }`, `RateLimit { retry_after }`, `Auth`, `Tool { tool }`, `Session`, `Channel { channel }`, `Internal(anyhow::Error)`. Internal functions use `anyhow::Result`; module boundaries use `NanobotError`.

### CLI Commands

`nanobot gateway` — full multi-channel daemon. `nanobot agent -m "message"` — single-turn CLI. `nanobot onboard` — first-time setup. `nanobot cron` — manage cron jobs. `nanobot auth` — OAuth flows.

## Common Pitfalls

- **Adding fields to `AgentLoopConfig`**: must update `src/cli/commands.rs` (both `setup_agent` and `agent()` functions), destructure in `AgentLoop::new()`, AND update `tests/message_flow.rs` `create_test_agent()`.
- **Adding fields to config structs with manual `Default` impl**: update both the struct definition and `Default::default()`.
- **YAML parsing**: uses `serde_yaml_ng` (not the deprecated `serde_yaml`).
- **`main.rs` and `lib.rs` both declare `mod errors`**: binary has its own module tree.
- **UTF-8 string slicing**: always use `is_char_boundary()` or `chars()` before slicing.
- **Tool execution**: wrapped in `tokio::task::spawn` for panic isolation — panics in tools don't crash the agent.
- **MemoryDB**: holds a persistent `std::sync::Mutex<Connection>`, not per-operation connections.
- **Cron 5-field expressions**: `compute_next_run()` normalizes by prepending "0 " for the seconds field.
