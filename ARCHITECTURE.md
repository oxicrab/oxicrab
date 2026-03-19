# Architecture

Oxicrab is a crate-based Rust workspace for running a multi-channel bot with layered routing, a shared agent loop, persistent memory, and optional HTTP APIs.

## Workspace

| Crate | Path | Responsibility |
|---|---|---|
| `oxicrab-core` | `crates/oxicrab-core/` | Core traits, config schema, shared bus types, errors, provider/tool/channel base types |
| `oxicrab-memory` | `crates/oxicrab-memory/` | SQLite memory DB, FTS5, embeddings, sessions, hygiene, quality gates |
| `oxicrab-providers` | `crates/oxicrab-providers/` | Anthropic, OpenAI, Gemini, fallback, circuit breaker, prompt-guided wrappers |
| `oxicrab-channels` | `crates/oxicrab-channels/` | Telegram, Discord, Slack, WhatsApp, Twilio adapters |
| `oxicrab-gateway` | `crates/oxicrab-gateway/` | HTTP chat API, webhooks, A2A, status, rate limiting |
| `oxicrab-router` | `crates/oxicrab-router/` | Deterministic router, routing policy, router context state machine, semantic filter |
| `oxicrab-safety` | `crates/oxicrab-safety/` | Leak detector and prompt guard |
| `oxicrab-tools-web` | `crates/oxicrab-tools-web/` | Web search, fetch, HTTP, Reddit |
| `oxicrab-tools-api` | `crates/oxicrab-tools-api/` | GitHub, weather, Todoist, media, image generation |
| `oxicrab-tools-google` | `crates/oxicrab-tools-google/` | Gmail, Calendar, Tasks, Google OAuth-backed tools |
| `oxicrab-tools-system` | `crates/oxicrab-tools-system/` | Filesystem, shell, tmux |
| `oxicrab-tools-browser` | `crates/oxicrab-tools-browser/` | Browser automation |
| `oxicrab-tools-obsidian` | `crates/oxicrab-tools-obsidian/` | Obsidian integration |
| `oxicrab-tools-rss` | `crates/oxicrab-tools-rss/` | RSS ingestion and ranking |
| `oxicrab` | `.` | Binary crate: agent loop, CLI, wiring, MCP integration, cron tool, orchestration |

The root crate composes the system. Most implementation detail now lives in focused library crates.

## Runtime Flow

```text
Channel / HTTP request
  -> MessageBus / direct inbound
  -> MessageRouter
  -> AgentLoop
  -> Tool execution and/or provider call
  -> Session + memory updates
  -> Outbound response
```

For channel users, `oxicrab gateway` is the long-running process that starts enabled channels, the agent loop, cron, and the optional HTTP gateway.

## Core Types

- `Tool` in `crates/oxicrab-core/src/tools/base/mod.rs`
  - Defines `name()`, `description()`, `parameters()`, `execute()`
  - Also carries capability metadata, routing rules, usage examples, timeout, and approval hints
- `BaseChannel` in `crates/oxicrab-core/src/channels/base/mod.rs`
  - Defines the common channel surface: `start()`, `stop()`, `send()`
  - Optional methods cover typing, edits, deletes, and richer channel-specific behavior
- `LLMProvider` in `crates/oxicrab-core/src/providers/base/mod.rs`
  - Defines `chat()`, `default_model()`, `warmup()`
  - Provides default `chat_with_retry()` using the in-tree `RetryConfig`, not the removed `backoff` crate

## Routing Layers

`MessageRouter` in `crates/oxicrab-router/src/` is the first decision point. It is intentionally policy-first:

1. Deterministic dispatch
   - Structured action payloads
   - Live directives from router context
   - Prefixed config rules from `router.rules`
   - Static tool rules
   - Remember fast path
2. Guided LLM
   - Used when the session is in focused tool context
   - Carries a strict `RoutingPolicy` with exact `allowed_tools`, `blocked_tools`, and a `reason`
3. Full LLM
   - Used when deterministic/guided paths do not apply
   - May apply semantic tool filtering first when confidence is high enough

Execution enforces the routing policy again at dispatch time. The router is not advisory.

## Router Context

Router context is persisted in `Session.metadata["router_context"]` and modeled as a small state machine:

- `Idle`
- `ToolFocused(tool, directives, ttl)`

Transitions are explicit:

- direct tool metadata can install a focused tool and directives
- directives expire by TTL
- consuming the final directive can return the session to idle
- user override or unrelated input can fall back to full LLM

This replaced the older implicit stale-context heuristics.

## Agent Loop

`AgentLoop` in `src/agent/loop/` is the orchestration layer. It:

- builds the system prompt and conversation
- applies routing output
- invokes the selected provider
- executes tool calls through a shared gateway
- appends results back into the conversation
- repeats until a final answer or iteration limit

Important current behavior:

- tool execution uses one gateway for LLM tool calls and direct dispatches
- tool policy, schema validation, approvals, exfiltration rules, timeouts, and panic isolation are enforced in one place
- hallucination handling is intentionally minimal: regex-based Layer 1 retry for unsupported action claims
- request-scoped runtime state is isolated per run
  - deferred tool activation from `tool_search`
  - pending interactive buttons
- session-scoped compaction state is isolated per session

## Tool System

`ToolRegistry` in `src/agent/tools/registry/mod.rs` is the execution engine. It is immutable after construction and runs middleware around tool execution:

- cache
- truncation
- logging

Tool behaviors worth knowing:

- deferred tools can be omitted from the initial tool list to save tokens
- `tool_search` can expose matching deferred tools during the current run
- activated deferred tools are request-scoped, not shared across sessions
- large tool results can be preserved in the stash and recovered via `stash_retrieve`
- common schema mismatches are auto-coerced before execution

## Interactive Buttons

Interactive buttons use a unified `metadata["buttons"]` format across Slack and Discord.

- `add_buttons` writes to a request-scoped `PendingButtons` store
- the agent loop drains only the current request’s buttons into response metadata
- Slack converts them to Block Kit
- Discord converts them to action rows
- button clicks can dispatch directly via structured `ActionDispatchPayload`

This replaced the earlier shared global button slot.

## Providers

Provider selection lives in `crates/oxicrab-providers/src/strategy/mod.rs`.

Selection order:

1. explicit `provider/model` prefix when present
2. model-name inference when no explicit provider is given

Routing can then layer on top:

- default model
- task-specific model overrides
- complexity-aware chat routing
- fallback chains across multiple providers

Local providers can optionally use prompt-guided tools, which inject tool definitions into the prompt and parse tool calls from text output.

## Memory and Sessions

`oxicrab-memory` owns persistence:

- SQLite `memory.sqlite3`
- FTS5 search
- optional embeddings
- hybrid keyword/vector retrieval
- session persistence
- cron tables
- pairing tables
- token usage logs

Important current details:

- memory and sessions are SQLite-backed, not file-note backed
- remember fast path can bypass the LLM for simple “remember …” inputs
- embeddings are enabled by default in runtime config
- group chats exclude personal memory from retrieval/system prompt context

## HTTP Gateway and A2A

`oxicrab-gateway` is optional runtime surface for:

- `POST /api/chat`
- `GET /api/health`
- webhook receivers
- A2A discovery and task endpoints
- status page and status API

Security posture:

- `gateway.apiKey` enables auth for chat and A2A task endpoints
- public non-loopback binds without auth emit warnings
- rate limiting supports `trustProxy`, but forwarded headers are only honored for configured trusted proxies
- webhook configs are validated strictly at config-load time

The gateway is part of the runtime, not the primary mental model for most users. Most usage still begins in a channel.

## Security

Important hardening layers:

- bidirectional leak detection
  - inbound redaction before LLM/session persistence
  - outbound redaction before channel delivery
- prompt injection detection
- shell AST analysis
- Landlock/Seatbelt sandboxing
- capability-based filesystem confinement
- DM pairing and sender allowlists
- DNS rebinding protection for outbound HTTP tools
- skill scanning before prompt injection
- exfiltration guard for network tools

## Config

Config is TOML-first and layered:

- `~/.oxicrab/config.toml`
- `~/.oxicrab/config.local.toml`
- `~/.oxicrab/config.d/*.toml`

Layers are merged first, then deserialized once into the canonical `Config` type and validated once. External keys are camelCase. Unknown keys are rejected.

## CLI

Common commands:

- `oxicrab onboard`
- `oxicrab gateway`
- `oxicrab agent -m "..."`
- `oxicrab doctor`
- `oxicrab channels`
- `oxicrab pairing`
- `oxicrab credentials`
- `oxicrab stats`

For public user documentation, prefer the channel-oriented docs in `docs/`. This file is an implementation map, not a getting-started guide.
