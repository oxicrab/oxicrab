# Cross-Review: 6 Repos — Feb 23, 2026

Reviewed: MicroClaw, Moltis, Carapace, OpenCrust, RustyClaw, femtoclaw.

Features already in Oxicrab have been removed. What remains are genuinely new ideas.

---

## Tier 1: High-Priority Adoptions

### 1. Pre-Compaction Memory Flush (RustyClaw + Moltis)
Inject a silent agent turn *before* compaction fires. The LLM gets a system prompt asking it to persist important context (user preferences, decisions, project state) to memory files. The response text is discarded. Prevents subtle knowledge loss when older messages are compacted away.

Neither has been seen in IronClaw, nanobot, or PicoClaw. Unique pattern.

**Effort**: 1 day. Self-contained — check `should_flush()` in agent loop before compaction threshold.

### 2. Tool Result Blob Sanitization (Moltis)
Strip base64 data-URI blobs (>=200 chars) and long hex sequences from tool results before feeding to the LLM. Replace with `[screenshot captured, N bytes]` or `[{mime_type} data removed]`. Oxicrab's `TruncationMiddleware` caps at 10k chars total but doesn't distinguish binary blobs from useful text — a 9k base64 blob eats the budget while the actual text gets truncated.

**Effort**: Half day. String processing in the middleware pipeline.

### 3. PII Detection in Output (Carapace + RustyClaw)
Extend existing `LeakDetector` (which already scans for API keys) with PII patterns: credit cards (with Luhn validation to reduce false positives), SSNs, phone numbers. Carapace's single-pass approach (collect all matches, sort, merge overlaps, redact in one sweep) is cleanest.

**Effort**: 1-2 days. Extends existing `src/safety/leak_detector/`.

### 4. Memory Quality Gates (MicroClaw)
Filter low-signal content before writing to memory: reject greetings, uncertain statements ("I think", "maybe"), content < 8 chars. Includes precision/recall regression test suite to prevent gate drift.

**Effort**: Half day. Add to memory write path.

### 5. Memory Poisoning Prevention (MicroClaw)
When the agent tries to remember "tool calls were broken", reframe as corrective "TODO: ensure tool calls execute via tool system." Prevents the agent from learning broken behavior from its own history. Novel — not seen in any other reviewed project.

**Effort**: Half day. Filter in memory write path.

### 6. Orphan ToolResult Cleanup Post-Compaction (MicroClaw)
After compaction, remove ToolResult blocks whose `tool_use_id` doesn't match any ToolUse block in the remaining messages. Orphaned results cause API errors with some providers (especially Anthropic).

**Effort**: 1-2 hours. Add to compaction output processing.

### 7. Enhance cargo-deny Configuration (Carapace + OpenCrust)
Oxicrab already has `deny.toml` and CI step, but the config is minimal (advisories only). Carapace and OpenCrust add: license allowlisting (MIT, Apache-2.0, BSD, ISC), dependency bans (no wildcard versions), and source restrictions (crates.io only, no git deps).

**Effort**: 30 minutes. Update existing `deny.toml`.

---

## Tier 2: Medium-Priority Adoptions

### 8. Layered Tool Policy System (Moltis + Carapace)
Oxicrab's `ExfiltrationGuardConfig` is a single global deny list. These repos have per-context tool access control with 6 layers (global → provider → agent → group → sender → sandbox). Deny always accumulates; allow is replaced by higher layers. Glob-pattern matching on tool names.

**Effort**: 2-3 days. Replaces/extends `ExfiltrationGuardConfig`.

### 9. Log Redaction via Tracing Subscriber (OpenCrust + Carapace)
Oxicrab's `LeakDetector` redacts outbound *messages* but not *log output*. If an error message accidentally contains an API key (e.g., from a failed provider response), it appears in tracing output unredacted. Wrap the tracing subscriber with regex replacement for known key patterns (sk-ant-*, sk-*, xoxb-*, xapp-*).

**Effort**: Half day. `MakeWriter` impl wrapping stderr.

### 10. Retry Engine with Retry-After Parsing (RustyClaw)
Oxicrab has `chat_with_retry()` with exponential backoff, but no `Retry-After` header parsing (delta-seconds and HTTP-date formats), no error classification enum with `should_failover()`, and no generic retry function reusable across providers. A centralized `retry_with_backoff()` function with jitter and configurable policy.

**Effort**: 1 day. Generic utility wrapping provider calls.

### 11. Echo/Dry-Run Gateway Mode (femtoclaw)
`oxicrab gateway --echo` — test channel connectivity without an LLM. Returns formatted echo of inbound message. Useful for testing Telegram webhook, Slack socket, Discord bot setup without burning API tokens.

**Effort**: 1-2 hours. Skip agent loop, return echo.

### 12. Prometheus/OTLP Metrics Export (Moltis + MicroClaw + Carapace)
Three repos independently built metrics export. Best metric definitions (Moltis): HTTP requests, LLM completions (input/output/cache tokens, TTFT, tokens/sec), tool executions, memory searches, active sessions. Carapace has a zero-dependency Prometheus text format exporter using only `std::sync::atomic`.

**Effort**: 3-4 days. Feature-gated. Surface existing CostGuard + search tracking data as Prometheus gauges.

### 13. A2A Protocol (OpenCrust)
Google's Agent-to-Agent protocol: `/.well-known/agent.json` AgentCard with capabilities, `POST /a2a/tasks` with task lifecycle (submitted → working → completed/failed). Positions for multi-agent interoperability. Oxicrab already has the gateway infrastructure to host these endpoints.

**Effort**: 1-2 weeks.

### 14. Config Hot-Reload with File Watcher (OpenCrust + Carapace)
Watch config file via `notify` crate, debounce (300-500ms), broadcast via `tokio::sync::watch`. Watch parent directory (not file itself) to handle editor write-to-temp-then-rename patterns.

**Effort**: 1-2 weeks.

### 15. Structured Audit Logging (Carapace + MicroClaw)
JSONL audit trail for security-relevant events: tool executions, tool denials, config changes, session creation/deletion, auth events. Non-blocking via bounded mpsc channel (10k capacity). File rotation at 50MB.

**Effort**: 3-5 days.

### 16. LLM Reranking for Search Results (Moltis)
Optional post-processing step after hybrid search: send top-N results to LLM for relevance scoring (0.0-1.0), reorder. Improves relevance at cost of one cheap LLM call.

**Effort**: 1-2 days.

### 17. Task Dead Letter Queue (MicroClaw)
When cron tasks fail, write to a `scheduled_task_dlq` table. Tools for viewing and replaying failed tasks. Prevents silent task failure.

**Effort**: Half day.

### 18. Fuzz Testing for Security Parsers (Carapace)
`cargo-fuzz` targets for: URL validation, webhook signature verification, config parsing, prompt guard patterns.

**Effort**: 2-3 days.

### 19. Session Integrity via HMAC Sidecars (Carapace)
HMAC-SHA256 over session data with `.hmac` sidecar files. Key derivation via HKDF-SHA256 from server secret. Configurable action on tampering (Warn/Reject).

**Effort**: 3-5 days.

---

## Tier 3: Lower Priority / Watch

| Feature | Source | Notes | Effort |
|---------|--------|-------|--------|
| Plugin context providers | MicroClaw | External plugins injecting dynamic content into system prompt per turn (weather, calendar, project status). Novel extension point. | Medium |
| Explicit "remember" fast path | MicroClaw | Bypass LLM for "remember that..." — instant write with Jaccard dedup. | Half day |
| Per-IP gateway rate limiting | OpenCrust, Carapace | tower-governor or token bucket. Important for internet-facing deployments. | 1-2 days |
| Link understanding pipeline | Carapace | URL extraction (code-block-aware) + SSRF-protected fetch + HTML-to-text + LRU cache. Previously ID'd from IronClaw review. | 1 week |
| SSRF extensions | RustyClaw, Carapace | Cloud metadata endpoint blocking, Unicode homograph detection, custom CIDR blocks. Extends existing `validate_and_resolve()`. | 1-2 days |
| Prompt guard extensions | RustyClaw, Carapace | Tool injection detection, exfiltration markers (markdown image injection with data params). Extends existing `PromptGuard`. | 1 day |
| Generated docs from code | MicroClaw | Auto-generate tool docs from `Tool` trait impls. CI `--check` mode detects drift. Prevents docs staleness. | Half day |
| Sigstore release signing | Moltis | Keyless signing with cosign for release artifacts. Supply chain security. | 1 day |
| Voice reply suffix prompt | Moltis | When user is on voice, append speech-friendly prompt modifier. | 1 hour |
| Working directory isolation | MicroClaw | Per-chat working directories to prevent file operation cross-contamination. | Half day |
| Encrypted credential vault | OpenCrust, Moltis | AES-256-GCM or XChaCha20 vault for at-rest credential encryption. Extends existing credential registry. | 1 week |
| WASM plugin system | Carapace, OpenCrust | Ed25519-signed wasmtime plugins with WIT interfaces. High effort but high extensibility value. | 3-8 weeks |
| Multi-agent personas | Moltis | Named agents with independent identity, memory, workspace. | 3-4 days |
| Per-model prompt profiles | Moltis | Different system prompt sections for different models via glob patterns. | 2-3 days |
| Domain-filtering network proxy | Moltis | HTTP CONNECT proxy filtering outbound sandbox connections against domain allowlist. | 3-4 days |
| Canvas / Agent-controlled UI | Moltis | Agent pushes interactive HTML to mobile. Novel but niche. | High |

---

## Novel Ideas (Not Seen in Previous Reviews)

1. **Pre-compaction memory flush** — Give agent a "last chance" to save context. Not in IronClaw, nanobot, PicoClaw, or any prior review.
2. **Memory poisoning prevention** — Reframe bug memories as corrective TODOs. Unique to MicroClaw.
3. **Canvas / A2UI** — Agent-generated interactive HTML UIs pushed to mobile (Moltis). Forward-thinking.
4. **Domain-filtering proxy** — Middle ground between no-network and full-network sandbox (Moltis).
5. **Per-model prompt profiles** — Different system prompts for different models via glob patterns (Moltis).
6. **Plugin context providers** — Dynamic per-turn system prompt injection from external sources (MicroClaw).
7. **Tool result blob sanitization** — Strip binary noise while preserving text (Moltis). Simple but no other project does this.

---

## Summary Table

| # | Feature | Effort | Priority |
|---|---------|--------|----------|
| 1 | Pre-compaction memory flush | 1 day | High |
| 2 | Tool result blob sanitization | 0.5 day | High |
| 3 | PII detection (extend LeakDetector) | 1-2 days | High |
| 4 | Memory quality gates | 0.5 day | High |
| 5 | Memory poisoning prevention | 0.5 day | High |
| 6 | Orphan ToolResult cleanup | 1-2 hours | High |
| 7 | Enhance cargo-deny config | 30 min | High |
| 8 | Layered tool policy | 2-3 days | Medium |
| 9 | Log redaction (tracing) | 0.5 day | Medium |
| 10 | Retry engine | 1 day | Medium |
| 11 | Echo gateway mode | 1-2 hours | Medium |
| 12 | Prometheus metrics | 3-4 days | Medium |
| 13 | A2A protocol | 1-2 weeks | Medium |
| 14 | Config hot-reload | 1-2 weeks | Medium |
| 15 | Structured audit logging | 3-5 days | Medium |
| 16 | LLM reranking | 1-2 days | Medium |
| 17 | Task DLQ | 0.5 day | Medium |
| 18 | Fuzz testing | 2-3 days | Medium |
| 19 | Session HMAC integrity | 3-5 days | Medium |
