# Competitive Review: 6 Nanobot Rust Clones

**Date:** 2026-02-13
**Repos reviewed:** ferrum-bot, open-vibe/nanobot-rs, supleed2/nanobot, sorcerai/nanobot-rust, yukihamada/nanobot, lichuanghan/openat

---

## HIGH PRIORITY — Clear Wins

### 1. Tool Parameter JSON Schema Validation
**Seen in:** ferrum-bot, open-vibe/nanobot-rs

Both repos validate tool arguments against the JSON schema **before** dispatching to `execute()`. This catches missing required fields and type mismatches centrally, giving consistent error messages back to the LLM and eliminating duplicated validation in each of the 24 tools.

ferrum-bot uses the `jsonschema` crate; open-vibe does it with a hand-rolled recursive validator as a default trait method. Either approach eliminates per-tool boilerplate.

### 2. Release Profile Optimization
**Seen in:** sorcerai, yukihamada, openat (3/6 repos)

```toml
[profile.release]
lto = "fat"
codegen-units = 1
strip = true
panic = "abort"
```

Expected 30-50% binary size reduction and better runtime perf. 5 minutes to add.

### 3. Interactive REPL Mode (`nanobot repl`)
**Seen in:** ferrum-bot (full `rustyline` impl), sorcerai (basic stdin loop)

ferrum-bot has a polished REPL with persistent history, `/retry`, `/session`, `/multi` (multi-line input), colorized prompts, and a spinner. Our `nanobot agent -m "message"` is single-turn only. A REPL transforms the CLI from one-shot to daily-driver.

### 4. Generic OpenAI-Compatible Provider
**Seen in:** ferrum-bot, openat, yukihamada (3/6 repos)

All three use a single provider implementation that handles DeepSeek, Groq, Moonshot, vLLM, OpenRouter, etc. by model-name routing. ferrum-bot supports 11 backends with one HTTP client. openat uses a `make_openai_provider!` macro. We'd keep our dedicated Anthropic/Gemini providers for native features but add one generic provider that massively expands model support.

### 5. MessageBus: Replace Mutex+Polling
**Seen in:** sorcerai (`async-channel`), openat (`tokio::sync::broadcast`)

Our `MessageBus` uses `Arc<Mutex<VecDeque>>` with 100ms polling intervals. Both alternatives eliminate polling — receivers wake instantly on message arrival. `async-channel` is simpler (MPMC); `broadcast` supports multiple subscribers. Either reduces latency and CPU.

---

## MEDIUM PRIORITY — Strong Improvements

### 6. Health Check HTTP Endpoint
**Seen in:** supleed2 (Axum), open-vibe (health/doctor CLI)

Adding a lightweight Axum server to gateway mode enables:
- `/health` for monitoring/container health checks
- Admin API for session info, tool status
- Future webhook receivers (GitHub, Slack HTTP mode)
- Prometheus metrics endpoint

supleed2 also demonstrates `CancellationToken` from `tokio-util` for coordinated graceful shutdown across all channels.

### 7. Environment Variable Config Overrides
**Seen in:** sorcerai, yukihamada, openat (3/6 repos)

All three support `NANOBOT_*` env vars overlaying the JSON config. yukihamada also supports `NANOBOT_CONFIG` as a full JSON blob (great for Docker/Lambda). Essential for containerized deployments where secrets shouldn't be in config files. The `dotenvy` crate (sorcerai, supleed2) adds `.env` file support.

### 8. Voice Message Transcription
**Seen in:** open-vibe, openat

Both use Groq's free Whisper API for audio transcription. Since we already handle media downloads from Telegram/Discord/WhatsApp, auto-transcribing voice messages before passing to the agent loop would be a high-value feature with low effort.

### 9. Provider Parallel Failover
**Seen in:** yukihamada/nanobot

Elegant 3-phase failover: primary gets 3s head start -> race all alternatives -> local fallback. With a cross-family model equivalence table (Claude <-> GPT <-> Gemini), the agent never fully fails even when one provider is down. Our `chat_with_retry()` currently only retries the same provider.

### 10. Pairing System for Unknown Senders
**Seen in:** open-vibe/nanobot-rs

Instead of silently dropping messages from non-allowed senders, issue a 6-digit pairing code. Owner approves via CLI (`nanobot pairing approve telegram ABC123`), which adds the sender to `allowFrom`. Much better UX than editing config files.

### 11. Docker Support
**Seen in:** supleed2 (cargo-chef + multi-arch), openat (docker-compose)

`cargo-chef` caches dependency compilation in Docker builds (dramatically faster rebuilds). supleed2 shows multi-arch (amd64+arm64) native builds. openat has a clean docker-compose with volume mounts separating data (rw) from config (ro).

### 12. Runtime Facts System Message
**Seen in:** open-vibe/nanobot-rs

Inject a system message listing available tools explicitly on every turn:
> "Runtime facts (authoritative): available tools are: read_file, web_search, exec... If a user asks for external actions, do not claim tools are unavailable."

Defensive pattern against models hallucinating about tool availability. Complements our existing first-iteration `tool_choice="any"` forcing.

---

## LOWER PRIORITY — Nice-to-Haves

| Item | Source | Notes |
|------|--------|-------|
| Model pricing/cost tracking | yukihamada | Static pricing table, log estimated $ per conversation |
| Channel linking (`/link` code) | yukihamada | Cross-channel session sync via 6-digit codes |
| `CancellationToken` shutdown | supleed2 | Cleaner coordinated shutdown than individual `stop()` calls |
| Dual-layer tracing subscriber | supleed2 | Per-crate log filtering (silence chatty deps) |
| `notify` crate for hot-reload | sorcerai | Watch SKILL.md files, reload without restart |
| stdin pipe support | sorcerai | `echo "summarize" \| nanobot agent` |
| `clippy::pedantic` | supleed2 | Catches more issues (missing `must_use`, redundant closures) |
| `indoc` crate | supleed2 | Cleaner multi-line string literals (zero runtime cost) |
| Session introspection tools | open-vibe | `sessions_list` and `sessions_history` tools for agent |
| Local LLM fallback (candle) | yukihamada | Offline-capable degraded mode with Qwen 0.6B |
| Fork bomb shell pattern | openat | Add `:\(\)\s*\{.*\};\s*:` to exec blocklist |
| Channel lifecycle events | openat | Connect/Disconnect/Error events in MessageBus |
| Workspace crate split | ferrum-bot | Split tools/config into own crates if compile times hurt |
| Tool context as `execute()` param | ferrum-bot | Cleaner than `set_context()` mutation; future refactor |

---

## Patterns to AVOID (things they do worse)

| Anti-pattern | Where seen | Why bad |
|---|---|---|
| `eprintln!` instead of tracing | open-vibe | No structured logging, no filtering |
| `unsafe { set_var() }` at runtime | open-vibe | Unsound in multi-threaded Rust |
| All-JSON persistence (no SQLite) | open-vibe, openat | Can't query, doesn't scale |
| No custom error types | sorcerai, open-vibe | Loses type info at boundaries |
| No SSRF/path traversal protection | all 6 repos | We're ahead on security |
| No tool result caching | all 6 repos | Our LRU cache is unique |
| No hallucination detection | 5/6 repos | Our regex detection is unique |
| Sequential tool execution | ferrum-bot | We already `join_all` for parallel |
| Separate thread + runtime for web | open-vibe | Should use existing tokio runtime |

---

## What We Have That None of Them Do

- LRU tool result caching (128 entries, 5min TTL)
- Regex-based hallucination detection + first-iteration tool_choice forcing
- SSRF protection (private IP blocking, DNS rebinding)
- Config secret redaction (custom Debug impls)
- OAuth credential chmod 0o600
- Tool execution panic isolation (`tokio::task::spawn`)
- MessageCompactor with LLM-based summarization + fact extraction
- SQLite FTS5 memory index with background indexer
- 24 production tools (vs 6-10 in the others)
