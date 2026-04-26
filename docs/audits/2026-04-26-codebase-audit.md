# Codebase audit — 2026-04-26

23 subsystems reviewed by parallel bug-hunt agents. All findings are
verified against source with `file:line` references. Severity reflects
production impact, not how interesting the bug is.

Headline counts: ~150 verified findings across all subsystems. The
densest concentrations are streaming pipeline (10), reflection /
trajectory (11), compaction (11), external-service tools (11), and
provider impls (7).

Skip list at the end of each section maps to the recently-fixed
items already addressed in the last week of commits.

---

## §1 — Agent loop core (`#39`)

Files: `src/agent/loop/{iteration,processing,helpers,mod,model_gateway,judge,reflection,replay}.rs`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| LOW | `iteration.rs:1755` | Misplaced docstring — block above `inject_queued_message` actually documents `fingerprint_tool_call` (line 1813); both functions end up effectively undocumented. |
| MEDIUM | `iteration.rs:510-517` | `consecutive_empty_responses` resets on a tool-only response. Currently safe because `last_was_tool_only` separately gates the next iteration, but the dual-state coupling is fragile. |
| MEDIUM | `iteration.rs:527-530` & `1481-1482` | Duplicate-call counter increments on every call but only resets on a *successful* result. A tool that fails twice then succeeds keeps a non-zero counter. The 30 s time gate masks this in practice. |

Verified-clean (recent fixes confirmed): phantom-guard ordering,
`build_clean_summary_messages`, mid-turn injection payload preservation,
cancel-token RAII drop, streaming end-event emission with buttons.

---

## §2 — Tool registry + middleware (`#40`)

Files: `src/agent/tools/registry/mod.rs`, `base.rs`, `setup/`, `stash/`, `tool_search/`, `interactive/`, `read_only_wrapper/`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `registry/mod.rs:85-92` | Boolean coerce only matches lowercase `"true"`/`"false"`. Uppercase variants from Python-serialized models are not coerced. |
| HIGH | `registry/mod.rs:804-830` | `TruncationMiddleware` extracts media paths into metadata BEFORE the stash call. If `stash()` returns `None` (failure), the metadata claims paths exist but the stash-key hint is never appended to truncated content. |
| HIGH | `registry/mod.rs:365-378, 423-429` | `is_deferred()` and `get()` lock `runtime` non-atomically. Cache reads at line 423-429 happen outside any lock — concurrent `register_runtime_deferred` while another thread reads cached definitions can return stale tool sets. |
| HIGH | `read_only_wrapper/mod.rs:52-62` | Schema filter only modifies the `action` enum; it doesn't strip `action` from `required` when ALL actions are read-only-filtered. A tool with no remaining read-only actions exposes a `required: ["action"]` schema with an empty enum — every LLM call validates-fails. |
| MEDIUM | `registry/mod.rs:524-544` | `inject_schema_hint` marker (`"\n\nTool description: "`) is a substring check. If a real tool's output contains the marker text, idempotency falsely skips injection on later retries. |
| MEDIUM | `registry/mod.rs:177-215` | `find_nonfinite_number_strings` recurses into objects and arrays of primitives but NOT into arrays of objects. `[{value: "NaN"}]` for `array<object<number>>` schema slips through. |
| MEDIUM | `interactive/mod.rs:26-55` | `PendingButtons` HashMap has no eviction policy. Long-running agent runs accumulate stale request-scoped entries. |
| LOW | `registry/mod.rs:79-84` | Object→string round-trip via `serde_json::to_string` sanitizes control chars in keys. Lossy for opaque-string fields containing pre-serialized objects. |

Verified-clean: schema-hint idempotency basic case, post-coerce validation, panic isolation.

---

## §3 — Provider implementations (`#41`)

Files: `crates/oxicrab-providers/src/{anthropic,anthropic_oauth,anthropic_common,openai,gemini,fallback,strategy,circuit_breaker,prompt_guided,passthrough,errors}/`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `lib.rs:57-65` (+ `anthropic/mod.rs:79`, `openai/mod.rs:320`, `gemini/mod.rs:343`) | `apply_custom_headers()` does NOT validate header names. Caller can inject `Authorization: Bearer attacker` via `providers.<name>.headers` and override the API-key auth set earlier in the same request. Privilege escalation via config injection. |
| MEDIUM | `openai/mod.rs:127` | `tc["id"].as_str().unwrap_or_default()` — empty/missing OpenAI tool-call IDs collide on `""`, cross-crediting reflection outcomes (same class as the Anthropic empty-id bug we already fixed). |
| MEDIUM | `anthropic_oauth/mod.rs:186-207` | OAuth `ensure_valid_token` reads `expires_at`, requests refresh, then re-checks. Concurrent callers across processes with skewed wall clocks can see a refreshed token as still-expired, attempt their own refresh, and one ends up serving a stale token. |
| MEDIUM | `anthropic_common/mod.rs:325-330` & `84-90` | `redacted_thinking` blocks stored as opaque strings. If the data field round-trips through unsafe escaping it can desync from Anthropic's expected verbatim replay → signature mismatch. |
| LOW | `anthropic_common/mod.rs:178-195` | `convert_tools` with empty tools array sends an empty `tools: []` with no cache_control. Currently accepted by the API; latent issue if future API versions reject. |
| LOW | `anthropic_common/mod.rs:248-249` | System-only with zero messages + json_mode_hint sends a system block with cache_control to an empty messages array. Edge case in HTTP API gateway path. |
| LOW | `lib.rs:39, 61` | `SESSION_AFFINITY_ID` is per-process — the comment claims it identifies a conversation, but two processes generate distinct UUIDs while one process serving many concurrent sessions emits the same UUID. Documentation drift, not a code bug. |

---

## §4 — Streaming pipeline (`#42`)

Files: `crates/oxicrab-core/src/streaming/mod.rs`, `crates/oxicrab-core/src/providers/base/mod.rs` (`StreamChunk`, `chat_stream`), `src/agent/loop/iteration.rs::run_streaming_call`, `src/agent/loop/processing.rs::run_stream_pump`, `crates/oxicrab-channels/src/{telegram,discord,slack}/streaming.rs`, `gateway_setup.rs` consumer registration.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `streaming/mod.rs:159-181` | `begin_emitted` swap-then-send race. After `swap(true, AcqRel)` returns false, the send can fail; another clone observing `has_begun()=true` (Acquire load) returns `AlreadySent` *before* the failing thread restores the flag. Net: dispatcher state says begin sent, no Begin event was queued. |
| HIGH | `processing.rs:28-90` | `STREAM_FAILURE_THRESHOLD = 3` only counts `update` failures. `begin` failures abandon immediately; `end` failures abandon immediately. Mixed paths (begin OK, 2 update failures, then begin again on retry) reset the counter unintuitively. |
| HIGH | `processing.rs:492-501` & `streaming/mod.rs:61` | `dispatcher.has_begun()` and pump's `was_active` can disagree. If pump abandoned after the begin atomic flipped to true, `processing.rs` still emits End against a removed turn-state slot — the consumer drains it silently with `unknown turn_id`. |
| HIGH | `telegram/streaming.rs:174-195` | Two-step commit (`editMessageText` then `editMessageReplyMarkup`). If the markup edit fails after the text edit succeeds, the user sees the final text without buttons; only a `warn!` is logged. |
| HIGH | `slack/streaming.rs:150-185` | When `body.is_empty()` and buttons are present, the early `Ok(())` return at line 151-152 skips the edit entirely. User never sees the buttons. |
| MEDIUM | `streaming/mod.rs:100` | Unbounded mpsc. A slow channel API + chatty Delta stream balloons heap. |
| MEDIUM | `*streaming.rs throttle blocks` | `skipped_edit` flag forces the next delta through regardless of throttle window. Two edits can fire <50 ms apart at window edges. |
| MEDIUM | `processing.rs:505-507` | Pump's 5 s timeout can fire while `consumer.end()` is mid-flight. Half-edited message left visible. |
| MEDIUM | `discord/streaming.rs:140-150` | Empty-content edit lacks the placeholder fallback Telegram now has. Serenity may reject empty content; user sees the original placeholder `"…"`. |
| MEDIUM | `iteration.rs::run_streaming_call` cancel path | Cancellation discards accumulated content; processing.rs's End is never emitted unless `has_begun()` is already true. Cancelled turns can lose visible text. |

---

## §5 — Memory subsystem (`#43`)

Files: `crates/oxicrab-memory/src/memory_db/{claims,traces,dlq,stats,cost,cron,collections,subagent_log}/`, `memory_store/`, `embeddings/`, `hygiene/`, `remember/`, `quality/`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| MEDIUM | `claims.rs:254` | `find_contradiction_pairs`, `find_stale_low_confidence_claims`, `find_orphan_claims` all collect IDs under the connection lock then iterate `get_claim` per id without holding the lock. Mid-iteration deletes are silently swallowed; results reflect a stale snapshot. |
| MEDIUM | `stats.rs:418-430` | `purge_old_search_logs` manually pre-deletes hits, then deletes log rows under the same `ON DELETE CASCADE`. A concurrent insert into `memory_search_hits` between the manual delete and the cascade can violate the FK and roll the transaction back. |
| MEDIUM | `quality/mod.rs:168-201` | `filter_lines` removes trailing empty lines but not interior empties left when all content lines are rejected. Header without body can ship as the entire output. |
| MEDIUM | `dlq.rs:113` | `increment_dlq_retry` has no guard. Replays can accumulate without bound; the cron tool's max-5 check is the only hold-back. |
| LOW | `mod.rs:87` | `recency_decay` returns 1.0 for negative `age_days` (future timestamps). Silent masking of clock skew / DB corruption. |
| LOW | `embeddings.rs:108-119` | Dimension mismatch silently drops entries. If a model swap keeps the byte-multiple-of-4 invariant but changes dimensions, search degrades silently. |
| LOW | `remember/mod.rs:71-86` | Fast-path Jaccard dedup (0.7) and embedding cosine dedup (0.85) are independent paths. No caller checks both; thresholds aren't aligned. |

---

## §6 — Channels (`#44`)

Files: `crates/oxicrab-channels/src/{manager,telegram,discord,slack,whatsapp,twilio,utils,dispatch}/`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `manager/mod.rs:259-314` | `ChannelManager.send` returns on the FIRST channel name match. Duplicate channel names in config silently use the first registration; the second is unreachable. No startup warning. |
| HIGH | `manager/mod.rs:159-213` | `start_all` rollback only stops channels in `started`. A channel that PANICKED during start is dropped by its task and orphaned — its background work may continue without a handle. |
| HIGH | `slack/mod.rs:191, 558-583` | `participated_threads` HashMap evicts only on insert. Reads don't update timestamps or trigger eviction; under heavy diversity the map can sit near the hard cap with stale entries for hours. |
| HIGH | `slack/formatting.rs:1-50` | `format_for_slack` never escapes `<`, `>`, `&` for mrkdwn. User text containing `<https://x|y>` is interpreted as Slack link markup; angle-bracket-heavy text breaks rendering. |
| HIGH | `slack/mod.rs:85-116, 390-397` | `is_retryable()` returns true for HTTP `5xx` and `RateLimited` only. Network-level errors (DNS, TCP reset, TLS) classify as `Other(...)` and are NOT retried. |
| HIGH | `discord/payloads.rs:88-118` | `parse_unified_buttons` puts ALL buttons in a single `CreateActionRow`. Discord caps action rows at 5 buttons — beyond 5 the API rejects or truncates. |
| MEDIUM | `discord/mod.rs:126-132, 231-236` | `discord_interaction_ts` uses `SystemTime::now()`. NTP rollback / VM pause inverts ordering; TTL math breaks. |
| MEDIUM | `telegram/mod.rs:408-431` | `html_chunks` and `raw_chunks` length mismatch path falls back to `html_chunk` as the "raw" version — the fallback isn't actually a true raw text. |
| MEDIUM | `twilio/mod.rs:180-187` | HMAC includes the configured `webhook_url`. Trailing-slash mismatch between config and what Twilio actually POSTs to fails every request with no diagnostic hint. |

---

## §7 — Gateway / HTTP API (`#45`)

Files: `crates/oxicrab-gateway/src/{lib,a2a,status,response_format}.rs`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `lib.rs:450-451` | Response-format JSON-schema size check runs AFTER `serde_json::to_string` deserialization. A 100 MB schema bypasses the 1 MB body limit because `serde_json` allocates first, size-checks second. DoS vector. |
| HIGH | `lib.rs:171` | Per-IP `governor::RateLimiter::keyed` has no stale-key eviction. Distributed attacks with many unique IPs exhaust memory. |
| MEDIUM | `lib.rs:374, 394` | `DefaultBodyLimit` applied at different layers between merged routers. A2A routes inherit the chat router's larger 1 MB+1 KB limit; merged-router precedence ordering is brittle. |
| MEDIUM | `lib.rs:550-555` & `status.rs:218-228` | `/api/health` returns `"starting"` when `ready=false`, even in echo mode. Status page returns `"echo"` for the same state. K8s probes see contradictory readiness. |
| LOW | `a2a/mod.rs:282-284` | A2A task IDs accept arbitrary strings as HashMap keys. Spec is `a2a-{uuid}`, but no validation rejects malformed IDs — reduces the attacker's enumeration cost. |
| LOW | `lib.rs:823` | Webhook `serde_json::from_slice(&body).ok()` swallows parse errors. Templates with `{{field}}` references silently empty out. No diagnostic. |

Verified-clean: signature comparison via `subtle::ConstantTimeEq`, oneshot drop guard.

---

## §8 — Router + context (`#46`)

Files: `crates/oxicrab-router/src/{lib,context,semantic,now_ms,rules}/`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| MEDIUM | `context/mod.rs:49-59` & `rules.rs:134-143` | `OneOf` literal collisions silently keep the first directive's index. Two directives sharing an option string are semantically merged; the second is unreachable. |
| MEDIUM | `context/mod.rs:207-226` | `prune_expired` recomputes `expires_at_ms` after retain. The `if directives.is_empty() || *expires_at_ms <= now_ms` clause can `set_idle()` while non-empty directives remain whose expiries cluster just before `now_ms`. Invariant breaks. |
| MEDIUM | `lib.rs:34-41`, `processing.rs:247-262` | `RoutingPolicy.allowed_tools` and `blocked_tools` precedence undefined. Code only ever consults `allowed_tools`, so blocked entries are silently ignored when both are present. |
| MEDIUM | `processing.rs:1228-1276` | `semantic_filter_tool_subset` holds the cache `Mutex` across `embed_query()` and `embed_texts()` async awaits — long lock that blocks parallel sessions. |
| LOW | `context/mod.rs:146-171` | `set_active_tool` clears directives unconditionally on tool change. Mid-batch `install_directives` calls can be orphaned. |

---

## §9 — Safety (`#47`)

Files: `crates/oxicrab-safety/src/{leak_detector,prompt_guard,credential_scrubber}/`, integration sites in agent loop / bus / gateway.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `leak_detector/mod.rs:193-195` | `LeakDetector.known_secrets` is a bare `Vec`. `add_known_secrets()` mutates without lock; `scan()` iterates concurrently — possible UB if registration happens after init. |
| HIGH | `credential_scrubber/mod.rs:65-82` | JSON traversal recursion has no depth limit. Pathological deeply-nested `Value` graphs cause stack overflow. |
| HIGH | `credential_scrubber/mod.rs:97-125` | URL query-string parser splits on literal `&` and `=` without percent-decoding. `?api_key=%3Dsecret` slips through unredacted. |
| HIGH | `processing.rs:631-632` | `reasoning_content` (extended thinking) is never scanned by leak detector or prompt guard before persistence. Skill content also unscanned. |
| MEDIUM | `leak_detector/mod.rs:30-136` | Pattern coverage gaps: Twilio Account SIDs (`ACxxx...`), GCP service-account JSON keys, Azure storage SAS tokens, Azure connection strings. |
| MEDIUM | `prompt_guard/mod.rs:30-125` | Jailbreak coverage missing newer patterns (STAN, AIM, token-impersonation), plus base64-encoded payloads. Also no warn-vs-block distinction beyond `should_block()`. |

---

## §10 — Compaction (`#48`)

Files: `src/agent/compaction/`, `src/agent/loop/compaction_history.rs`, `src/agent/truncation/mod.rs`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `compaction/mod.rs:299-322` & `extract_facts():378-426` | `compact()` and `extract_facts()` lack the `finish_reason` guard `flush_to_memory` has. Truncated summaries (max_tokens) silently persist. |
| HIGH | `compaction/mod.rs:183-223` | When multiple orphaned IDs exist in one assistant turn, repair-insertion runs in REVERSE — but ALL inserts target the same `idx+1`, so semantic order of placeholders is reversed. `[tool_a, tool_b]` orphaned → `[placeholder_b, placeholder_a]`. |
| MEDIUM | `compaction/mod.rs:116-234` | Mixed messages (both OpenAI `tool_calls` AND Anthropic `tool_use` content blocks populated) merge into a single `assistant_tool_ids` set. Same-id collision across formats not deduplicated. |
| MEDIUM | `compaction_history.rs:172-189` | If user-supplied content contains `\x01[Checkpoint]`, the next `strip_annotations` cycle strips it as if it were a real annotation. |
| MEDIUM | `compaction_history.rs:193-201` | Recovery summary 2000-char cap is applied BEFORE leak redaction. Redaction can EXPAND the string (a short token redacted to a long category label) — final length unbounded. |
| LOW | `compaction/mod.rs:23-29` | `estimate_tokens = bytes / 4` undercounts CJK and code-heavy content by 3-4×. |
| LOW | `truncation/mod.rs:81-110` | Soft cap reservation degenerates for `max_chars < 200`. Hard-clip fallback covers it but the design is fragile. |
| LOW | `compaction/mod.rs:434-456` | `split_at_turn_boundary` returns 0 when no user role found. Tests don't cover empty-after-strip or assistant-first edge cases. |

---

## §11 — Skills (`#49`)

Files: `src/agent/skills/{index,propose,scanner,refine,loader,manager}.rs`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `propose.rs:163-173, 197` | `promote_staged_skill` checks `symlink_metadata().is_symlink()` but a HARDLINK to `/etc/passwd` (created in the staging dir before promotion) passes. `O_NOFOLLOW` doesn't cover hardlinks. |
| HIGH | `propose.rs:49, 119` | Concurrent `propose_skill` calls with the same name race on truncate+write — second writer silently overwrites the first. No file lock. |
| HIGH | `refine/mod.rs:315, 323-331` | Atomic patch (tmp+rename) succeeds, but the `{name}-CHANGELOG.md` sidecar is appended non-atomically. Crash between rename and sidecar write leaves audit log stale. |
| MEDIUM | `index.rs:119-140` | `index_one` returns `Ok` even when embedding service is down — caller never learns the freshly-promoted skill is undiscoverable. |
| MEDIUM | `mod.rs:447` | `keywords.dedup()` only collapses adjacent duplicates. Description-word matching name-part NOT adjacent yields duplicate keywords; relative ordering is unstable. |
| LOW | `scanner/mod.rs:184` | Sliding-window asymmetry on edge file lengths. |
| LOW | `mod.rs:16-17, 272` | `MAX_SKILL_CONTEXT_CHARS = 20_000` is aggregate. Per-skill budget unenforced — a 5 KB skill consumes 25% of the prompt alone. |

---

## §12 — Subagent + approval (`#50`)

Files: `src/agent/subagent/`, `src/agent/approval/`, `src/agent/loop/helpers.rs::await_approval`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `subagent/mod.rs:134, 270` | `is_finished()` HashMap entries persist after task abort (cleanup at line 270 doesn't run if cancellation removed the entry first). 100-task cap fills with zombies. |
| HIGH | `subagent/mod.rs:145-148` | When `semaphore.acquire()` fails (channel closed), the spawned task logs and drops silently. The user already saw "Subagent started" but never receives a result. |
| MEDIUM | `helpers.rs:400-414` | Approval-timeout enriched error redacts `params.to_string()`. The serde-string form may not match every leak-detector pattern (nested-object keys etc.) — narrow miss possible. |
| MEDIUM | `approval/mod.rs:88-101` | Source-channel validation is strict — operator clicking the same button from a different chat (group→DM with same user) is rejected. UX edge case. |
| LOW | `tools/read_only_wrapper/mod.rs:53-63` (also under §2) | Mutating-only tools wrapped read-only end up with empty enum + `required: ["action"]`. |

Verified-clean: self-approval deadlock prevention at `mod.rs:1251`, double-prompt prevention at `helpers.rs:186-206`.

---

## §13 — Cron (`#51`)

Files: `src/cron/`, `src/agent/tools/cron/`, `gateway_setup.rs` cron callback, `crates/oxicrab-memory/src/memory_db/{cron,traces}/`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `memory_db/cron/mod.rs:369` & `service/mod.rs:299-325` | Atomic claim's `WHERE last_status != 'running'` guard fails the race when two pollers compute the same `next_run_at_ms` and both pass the guard. Run count can double-increment. |
| HIGH | `gateway_setup.rs:574-580` | Trace persistence via `spawn_blocking()` has no timeout. SQLite lock contention starves the cron callback thread pool. |
| HIGH | `event_matcher/mod.rs:109-116` | Cooldown check uses millisecond precision. Two events arriving in the same millisecond both pass; both fire. |
| MEDIUM | `cron/mod.rs:825` (`update` action) | Cron-tool `update` action has no `IS_CRON_JOB` guard. A cron job can mutate other jobs' schedules from inside its own run, escaping the loop-prevention contract. |
| MEDIUM | `cron/mod.rs:953` & `gateway_setup.rs:531-546` | DLQ status set to `"replayed"` before run; if run succeeds, `last_status` becomes `"success"` but DLQ status stays `"replayed"`. Operator-facing status drift. |
| LOW | `finish_cron/mod.rs:100-144` | Multiple `finish_cron` calls in one run — second call's metadata wins silently, no error. |

---

## §14 — Session lifecycle (`#52`)

Files: `crates/oxicrab-memory/src/session/`, `crates/oxicrab-memory/src/memory_db/sessions/`, integration in agent loop.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `session/manager/mod.rs:407` | Daily rotation compares `created_at.date_naive() < Utc::now().date_naive()` — strips TZ info. DST / leap-second edge cases at midnight UTC can rotate prematurely or skip. |
| HIGH | `memory_db/mod.rs:238-242` | `INSERT OR REPLACE` under WAL allows two concurrent writers to silently last-write-wins. Multi-process deployments lose updates. |
| MEDIUM | `session/manager/mod.rs:354-402` | LRU cache is local to the process. A second oxicrab process updating the SQLite session row leaves Process A's cache stale until the next save. |
| MEDIUM | `processing.rs:2073-2083` | `is_reset_command` only strips `.!?` once, no embedded punctuation handling. `"reset??"` doesn't match. |
| LOW | `session/manager/mod.rs:447-459` | Delete then cache-pop has a narrow race on transient `spawn_blocking` failure. |

---

## §15 — Bus + dispatch (`#53`)

Files: `crates/oxicrab-core/src/bus/`, `crates/oxicrab-core/src/dispatch/`, `src/bus/`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `src/bus/queue/mod.rs:179-189` | Outbound leak-detector scans ONLY `msg.content`. `msg.media` paths and most of `msg.metadata` (other than button context) are not scanned. |
| HIGH | `events/mod.rs:168-173` | `OutboundMessage::from_inbound` strips `IS_CRON_JOB`, `RESPONSE_FORMAT`, `WEBHOOK_NAME`. `SESSION_ID`, `APPROVAL_REQUIRED`, `ACTIVE_TOOL` are inbound-only but NOT stripped — they leak to outbound consumers. |
| MEDIUM | `src/bus/queue/mod.rs:192, 196` | Hardcoded `"buttons"` and `"context"` strings instead of `meta::BUTTONS`. Future renames silently desync. |
| LOW | `dispatch/mod.rs:12-18` | Router emits `ActionDirective` source labels but core `ActionSource` enum has no `Directive` variant — orphaned label. |

Verified-clean: 1 MB inbound truncation char-boundary safety, `floor_char_boundary` outbound, session_key formatting.

---

## §16 — Reflection / trajectory / auto-suggest / skill-refine (`#54`)

Files: `src/agent/loop/{reflection,auto_suggest,auto_refine}.rs`, `src/agent/trajectory/`, `src/agent/activity_journal/`, `src/agent/skills/refine/`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `iteration.rs:230-231, 1148-1179` | `pending_reflection_outcomes` is mutated without lock across iterations. Spawned reflection tasks can race the next iteration's `clear()`. |
| HIGH | `reflection.rs:64-104` | `ReflectionBudget` is fresh per `process_message()`, so the per-request cap is in fact per-iteration when retries happen. Operator intent ("max 2 reflections per request") quietly violated. |
| HIGH | `activity_journal/mod.rs:70-104` | `append()` uses an in-process `write_lock` but no `flock()` for cross-process safety. `flush()` without `sync_all()` loses data on kernel crash. UTF-8 truncation before serialization can produce invalid JSON if cut mid-codepoint. |
| HIGH | `mod.rs:445-479` | Trajectory compression saves LLM-generated summaries to `trajectory_summaries` with no size cap. Long summaries multiplied across thousands of sessions can OOM the DB. |
| MEDIUM | `skill_suggester.rs:29-50, 118-162` | N-gram counter does NOT distinguish "same sequence in same retry loop" from "same sequence across distinct turns." A 5-step error-recovery loop counts as 5 cross-session occurrences. |
| MEDIUM | `refine/mod.rs:142-177` | Round-2 patch applied even when `new_body == body_before`. No-op patches pollute audit trail. |
| MEDIUM | `refine/mod.rs:179-214` | Round-1 malformed JSON aborts the refine attempt with no retry. |
| MEDIUM | `tool_reflections.rs:20-40` | `insert_tool_reflection` has no `ON CONFLICT`. Concurrent inserts for the same `(request_id, tool_name, action)` can race. |
| LOW | `trajectories.rs:68-87` | Trajectory events insert one-at-a-time — high-volume runs put pressure on SQLite. |

---

## §17 — Config + credentials (`#55`)

Files: `src/config/loader/`, `src/config/credentials/`, `crates/oxicrab-core/src/config/schema/`, `src/config/routing/`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `loader/mod.rs:71-86` | `.toml.lock` files are never deleted. Each `save_config` leaves a sentinel; long-lived deployments accumulate. |
| HIGH | `loader/mod.rs:230-243` | `WARNED` static HashSet is per-process and never garbage-collected. Memory leak on long-running processes loading configs repeatedly. |
| MEDIUM | `loader/mod.rs:71-86` | Atomic-write race window: lock is on the `.toml.lock` file (separate inode); rename is atomic but lock acquisition + tempfile write + persist is not a single critical section. |
| MEDIUM | `agent.rs:877-883` | `TaskRouting` `#[serde(untagged)]` enum tries `Model(String)` first. A struct value with a JSON-string-shaped error silently falls into `Model("...")` instead of failing loudly. |
| LOW | `tests.rs:40-46` | Round-trip test on `config.example.toml` only validates load — it doesn't save+reload to verify all serializers/deserializers round-trip. |
| LOW | `loader/mod.rs:204-225` | `serde_ignored` reports paths in nested arrays of objects, but no test covers `[gateway.webhooks.X.targets[0].unknownKey]`-style failures. |

Verified-clean: env > helper > keyring > TOML resolution, custom-header reserved-name lowercase compare, keyring graceful fallback.

---

## §18 — CLI / startup wiring (`#56`)

Files: `src/main.rs`, `src/lib.rs`, `src/cli/`, `src/cli/commands/gateway_setup.rs`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `gateway_setup.rs:177-178` | The HTTP API task handle is bound to `_http_task` and dropped immediately. The HTTP server runs detached; panics or port-conflict failures are NOT propagated to the main shutdown loop. |
| HIGH | `gateway_setup.rs:236-255` | Ctrl-C cleanup explicitly stops cron and agent but lets channels and HTTP server "stop themselves." With the dropped HTTP handle (above) and reference-counted outbound_tx, ordering is fragile. |
| HIGH | `cli/onboard.rs:124` | `save_config` overwrites silently. If the existing config has unknown keys, the next `load_config` rejects via `deserialize_config_strict`. Onboarding can break a working install. |
| MEDIUM | `gateway_setup.rs:825-839` | Channel-supervisor is `tokio::spawn`'d and never joined. A panic inside it logs and silently dies; nothing restarts the supervisor. |
| MEDIUM | `gateway_setup.rs:219-240` | Stream consumer registration + status-lock set + agent-loop spawn ordering: there's a window where channels could deliver messages before `run()` is consuming. |

---

## §19 — Built-in tools (`#57`)

Files: `src/agent/tools/{file,exec,shell,http,web_search,web_fetch}/`, `crates/oxicrab-tools-{system,web,browser}/`, `image_gen`, `web_summarize`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `shell/mod.rs:382-399` | `child_pid` captured before `wait_with_output()` consumes the handle. If the child exits and the PID is recycled before timeout fires, `killpg(pid, SIGKILL)` kills an unrelated process group. |
| HIGH | `config/schema/tools.rs:167-169` | `allowedCommands` exact-string match. `cat` allowed → `/usr/bin/cat` blocked, but a user-supplied `~/bin/cat` (basename `cat`) passes the basename comparison and runs the attacker binary. |
| HIGH | `browser/mod.rs:391-420` | Width clamp at 1920px combined with 10080px height = 19.3M pixels = ~77 MB raw RGBA per screenshot. OOM vector on low-resource hosts. |
| MEDIUM | `http/mod.rs:147-150, 176-179` | The 10 MB body limit is re-applied per redirect step. A 5-redirect chain accepts up to 50 MB of cumulative body. |
| MEDIUM | `filesystem/mod.rs:20-26, 55-66` | `resolve_path` falls back to `lexical_normalize` on non-existent paths. Symlinks in parent directories aren't resolved on this branch — TOCTOU race against post-check symlink creation. |
| LOW | `web_summarize/mod.rs:72-73` | Cache key includes `url` literally. `https://Example.com` and `https://example.com` cache as distinct entries. |
| LOW | `browser/mod.rs:239-246` | Post-action SSRF check fires after `page.goto()`. JS-initiated redirects to internal IPs see a brief window before the next action's check runs. |

Verified-clean: sandbox fail-closed, validate_and_resolve covers RFC 1918 + IPv6 ULA + multicast.

---

## §20 — External-service tools (`#58`)

Files: `crates/oxicrab-tools-{api,google,obsidian,rss}/`, `src/agent/tools/{todoist,media,reddit}/`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `google_common/mod.rs:51-62` & `auth/mod.rs:79-126` | OAuth refresh on 401 retries the same refresh on failure with no backoff or revocation detection. Revoked refresh tokens cause infinite-retry loops. |
| HIGH | `google_mail/mod.rs:268-332` | `reply` action only strips `\r` from body (line 294). `\n` survives in `message_id`, `reply_to`, `In-Reply-To` paths — header injection. |
| HIGH | `rss/feeds.rs:61-70` | `feed_rs::parser::parse()` runs on untrusted XML with no entity-expansion limit / XXE mitigation. Billion-laughs-style CPU DoS. |
| HIGH | `reddit/mod.rs:41` | `Policy::limited(5)` follows redirects without re-validating each hop. Attacker controlling a Reddit URL field can chain to internal targets via redirect. |
| HIGH | `obsidian/client.rs:124-133` | `validate_path` decodes URL-encoded path ONCE. `%252e%252e` decodes to `%2e%2e`, passes the `..` check, then re-encodes and the Obsidian REST API double-decodes server-side → traversal. |
| MEDIUM | `google_mail/mod.rs:326-329` | `message_id` extracted from headers used directly with no length cap. Multi-KB Message-IDs bloat the constructed email. |
| MEDIUM | `lib.rs:25-50` (google) | Scope-flag drift: `gmail=false` but credentials cached `gmail` scope retains it. Capability flag changes don't reconcile against stored scopes. |
| MEDIUM | `github/mod.rs:113-119` | `sanitize_api_error_text` doesn't strip HTML — GitHub HTML error pages echo back as tool-result text. |
| MEDIUM | `rss/scanner.rs:153-178` | Article URL UNIQUE constraint is case-sensitive. `Article` vs `article`, trailing-slash, www-prefix all create dupes. |
| MEDIUM | `media/mod.rs:48-83` | API key in `X-Api-Key` header is correct, but error logging may include URL with embedded query params if a redirect path put the key elsewhere. |
| LOW | `github/mod.rs:1010-1028` | `create_pr_review` with event=COMMENT is non-mutating but classified as mutating. `ReadOnlyToolWrapper` filters drift from real tool semantics. |

---

## §21 — MCP integration (`#59`)

Files: `src/agent/tools/mcp/`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `mcp/mod.rs:126-129` | 30 s handshake timeout uses `await?`. ANY single slow MCP server fails the whole agent startup with no retry. |
| HIGH | `mcp/proxy/mod.rs:69-70` | Null-param stripping is shallow (`map.into_iter().filter`). Nested nulls in nested objects/arrays survive — some MCP servers reject. |
| MEDIUM | `mcp/mod.rs:197-206` | Child-process shutdown calls `client.cancel()` only. No SIGTERM-then-SIGKILL fallback; relies on rmcp transport semantics. |
| MEDIUM | no respawn logic | MCP server crash = single-shot connection lost. Restart agent to reconnect. |
| LOW | `setup/mod.rs:118-126` | MCP-vs-MCP shadowing only `warn!`s. Two MCP servers with the same tool name silently keep one. |

Verified-clean: 10 s discovery timeout (graceful skip), 120 s execution timeout, 10 MB result cap, env CR/LF rejection.

---

## §22 — Transcription / voice (`#60`)

Files: `crates/oxicrab-transcription/`, `src/utils/transcription/`, `src/agent/loop/helpers.rs`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| MEDIUM | `transcription/lib.rs:249-260` | Extension MIME mapping is case-sensitive. `.OGG`, `.MP3` fall back to `audio/ogg` regardless of real format. |
| MEDIUM | `helpers.rs:586-604` | `replace_bracketed_tags` uses naive `find(']')` — first `]` wins. `[audio: /path/with]bracket.ogg]` extracts `/path/with`. |
| MEDIUM | `transcription/lib.rs:387-406` | Background load race. If transcription request arrives before `OnceCell::get()` returns `Some`, the message falls through to `strip_audio_tags()` and silently loses audio content. |
| LOW | `transcription/lib.rs:265-270 vs 448-453` | Cloud cap 25 MB; local PCM cap 50 MB. A 30 MB file fails cloud but the local PCM expansion may exceed 50 MB at runtime. |
| LOW | `transcription/lib.rs:303` | Groq 429 returned as generic "whisper API returned 429" without backoff/retry. |

---

## §23 — Pairing / workspace / cognitive / utils (`#61`)

Files: `src/pairing/`, `src/agent/workspace/`, `src/agent/cognitive/`, `src/utils/`.

| Sev | File:Line | Issue |
|-----|-----------|-------|
| HIGH | `media/mod.rs:10-12` | `media_dir()` joins `OXICRAB_HOME/media` without canonicalizing. Malicious env override (`OXICRAB_HOME=/tmp/../../../etc`) → `/etc/media`. Downstream `is_safe_media_path` canonicalizes, but this is the source of truth used elsewhere. |
| HIGH | `path_sanitize.rs:29, 39` | `is_char_boundary(home_str.len())` is checked AFTER `starts_with`, but the substring op can panic if the path equals home_str without trailing separator. |
| HIGH | `pairing/mod.rs:68, 207-211` | `cleanup_expired()` only fires from `request_pairing()`. `approve()` and `list_pending()` see stale codes (still rejected at approve, but pollute the table for up to 24 h). |
| MEDIUM | `workspace/mod.rs:339-346, 367-375` | `cleanup_expired` uses `created_at` only, not `accessed_at`. Frequently-read files get deleted at TTL despite recent access. |
| MEDIUM | `pairing/mod.rs:12` | Per-channel pending limit is 3, but no per-sender cap. Attacker can spam pairing across channels. |
| LOW | various | `Regex::new()` direct calls scattered across `complexity/`, `context/providers/`, `skills/scanner/`, `tools/cron/` instead of `RegexPatterns` registry. |
| LOW | `workspace/mod.rs:179` | Reserved dirs (`memory/`, `knowledge/`, `skills/`, `sessions/`) are policy-only; no filesystem guard prevents user code writing there. |

---

## Cross-subsystem patterns

A few classes of issue recur across subsystems and may be worth
addressing centrally:

- **Empty `unwrap_or_default` on tool/event IDs** — Anthropic
  tool_use, OpenAI tool_calls, A2A task IDs, approval IDs, dispatch
  IDs. We should standardise on "empty id ⇒ skip" everywhere.
- **Recursive JSON traversal without depth limits** — credential
  scrubber, tool params coercer, prompt-guard scanner. None bound
  recursion depth.
- **Per-process caches (Mutex<HashMap>) without cross-process
  coherency** — sessions, pairing, claims, button registry. Multi-
  process deployments see split-brain.
- **Throttle/timeout windows that race kernel rescheduling** —
  cron poller, stream throttle, OAuth refresh, LRU TTL eviction.
  Generally use millisecond-resolution wall-clock comparisons and
  trip in pathological scheduling.
- **Tool-action read-only classification drift** — registry's
  `ReadOnlyToolWrapper` and per-tool `ActionDescriptor.read_only`
  diverge over time as new actions are added. Audit trail not
  enforced by tests.
- **TTL eviction only on insert** — Slack thread tracker, dispatch
  store, regex pattern cache. Long-tail entries linger.

## Suggested next-pass priorities

If we fix in priority order by production impact:

1. **§4 Streaming** — five HIGH bugs in code that ships behind a
   per-channel flag. Stop-the-bleed before more channels enable it.
2. **§3 Provider header injection** — security-critical, simple fix.
3. **§19 Shell PID-recycle race** — security-critical, real attack
   surface on hosts with hostile PID exhaustion.
4. **§20 OAuth refresh loop** — production downtime risk.
5. **§9 Safety credential scrubber** — DoS vector + URL-encoded
   bypass.
6. **§16 Reflection HashSet race + ActivityJournal fsync** —
   correctness-of-audit issues.

The remaining LOW/MEDIUM items are accumulated drift and are best
addressed in a follow-up sweep.
