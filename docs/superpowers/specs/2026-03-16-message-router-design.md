# Message Router Design

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan.

**Goal:** A high-performance message routing layer that fuses deterministic dispatch with LLM interpretation. The router sits at the top of the processing pipeline, making sub-100μs decisions about whether a message needs LLM involvement at all. Mechanical actions (button clicks, contextual shortcuts, commands) execute directly. Ambiguous messages in a known context get a guided LLM call with filtered tools and hints. Everything else flows to the LLM unchanged.

**Motivation:** LLMs consistently fail at mechanical tasks — fabricating tool results instead of calling tools, ignoring button contexts, losing track of conversation state. The current system tries to fix this reactively (4-layer hallucination detection, intent classification, correction retries). The router fixes it proactively by never sending mechanical actions to the LLM in the first place. This is faster (skip 1-3s LLM round-trip), cheaper (fewer tokens), more reliable (deterministic), and simpler (removes reactive control systems).

**Supersedes:** `docs/superpowers/specs/2026-03-16-action-dispatch-design.md` — the action dispatch concept is subsumed by the router's `DirectDispatch` path. The earlier spec's `ActionDispatch`, `ActionDispatchPayload`, and `DispatchContextStore` types are retained and used by the router.

## Core Architecture

### `MessageRouter`

```rust
// src/router/mod.rs

pub struct MessageRouter {
    /// Tool-declared rules, compiled at startup. Keyed by tool name for fast lookup.
    static_rules: Vec<StaticRule>,
    /// User-defined prefix commands from config. HashMap<trigger_word, ConfigRule>.
    config_rules: HashMap<String, ConfigRule>,
    /// Command prefix character(s), default "!". Avoids collision with Slack/Discord slash commands.
    prefix: String,
    /// Aho-Corasick automaton built from all static rule triggers.
    /// Rebuilt only when static rules change (i.e., never after startup).
    static_ac: aho_corasick::AhoCorasick,
}
```

The router is **stateless** — it does not hold session state, tool references, or LLM providers. It receives everything it needs as arguments to `route()` and returns a decision. This makes it independently testable and zero-cost to share across threads via `Arc<MessageRouter>`.

### `RoutingDecision`

```rust
pub enum RoutingDecision {
    /// Execute tool call directly, no LLM involved.
    DirectDispatch {
        tool: String,
        params: serde_json::Value,
        source: DispatchSource,
    },
    /// LLM interprets, but with constraints: filtered tool set + context hint.
    GuidedLLM {
        tool_subset: Vec<String>,
        context_hint: String,
    },
    /// Full LLM, no constraints. Today's behavior.
    FullLLM,
}

pub enum DispatchSource {
    Button,
    ActionDirective,
    StaticRule,
    ConfigRule,
    RememberFastPath,
    Webhook,
}
```

### Routing Priority

`route()` checks sources in strict priority order, returns on first match:

1. **`InboundMessage.action`** (buttons, webhooks) → `DirectDispatch`. Structured payload, no ambiguity.
2. **Action directives** from `RouterContext` — match message text against active directives. Matching is **case-insensitive** and **whole-message** (after trimming whitespace). For `Exact`: message equals trigger. For `OneOf`: message equals any alternative. For `Pattern`: regex applied to full message (for captures). → `DirectDispatch`.
3. **Prefixed config rules** — if message starts with `prefix` (default `!`), strip prefix, look up command word in `config_rules` HashMap. Run prompt guard on substituted `$N`/`$*` values before dispatch (config rules inject user text into params). → `DirectDispatch`.
4. **Static tool rules** — match message text against static rules, filtered to rules where `requires_context == false` OR `requires_context == true && active_tool matches`. Matching is **case-insensitive** and **whole-message** (same as directives). → `DirectDispatch`.
5. **Remember fast path** — existing `extract_remember_content()` logic, moved into router. Router only **classifies** the message as a remember intent. Actual execution (quality gates, dedup, DB writes) happens in the DirectDispatch handler, not in the router. → `DirectDispatch` to remember handler.
6. **Active tool context exists** but no direct match → `GuidedLLM`. `tool_subset` contains: the active tool name + core tools (`memory`, `add_buttons`) + any deferred tools activated by `tool_search` during the session. `context_hint` describes current state.
7. **No context, no matches** → `FullLLM`.

### Performance Contract

- **Target:** < 100μs per routing decision.
- **Matching:** All trigger matching is **case-insensitive whole-message** comparison. Message is lowercased and trimmed once, then compared against pre-lowercased triggers. No substring/AC automaton needed for `Exact`/`OneOf` — these are HashSet lookups on the normalized message. AC automaton reserved for static rules (need to check if any rule matches, not where in the message).
- **Config rules:** HashMap lookup on command word. O(1).
- **Directive matching:** AC scan for `Exact`/`OneOf` literals (single pass). Regex compiled lazily and cached (rare path — only for parameter captures).
- **No allocations on the fast path** for `FullLLM` decisions (most common case — just fall through).
- **No IO:** Router never touches disk, network, or database. Pure CPU.

## RouterContext — Per-Session Conversation State

```rust
// src/router/context.rs

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouterContext {
    /// Which tool is "in focus" — set when a tool's result establishes ongoing interaction.
    pub active_tool: Option<String>,
    /// Dynamic rules from the last tool result. Short-lived.
    pub action_directives: Vec<ActionDirective>,
    /// Millisecond timestamp of last update.
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionDirective {
    pub trigger: DirectiveTrigger,
    pub tool: String,
    pub params: serde_json::Value,
    /// Consumed after matching if true.
    pub single_use: bool,
    /// Milliseconds from creation until expiry.
    pub ttl_ms: i64,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DirectiveTrigger {
    /// Single literal — "next", "done". AC automaton match.
    Exact(String),
    /// Alternative literals — "yes|accept|ok". HashSet lookup.
    OneOf(Vec<String>),
    /// Regex with captures. Compiled lazily, cached. Rare.
    Pattern(String),
}
```

### Persistence

Serialized as JSON into `Session.metadata["router_context"]`. Piggybacks on the existing session load/save — no extra DB queries, no new tables. Loaded once when the session is loaded (before routing), saved back only when directives change.

### Lifecycle

- **Tool sets context:** `ToolResult.metadata["active_tool"]` and `ToolResult.metadata["action_directives"]` extracted after tool execution. New directives **replace** all existing directives (not append). This is a full replacement — the tool declares the complete set of expected follow-ups each time.
- **Max directives:** 20 per `RouterContext`. If a tool returns more, truncate to 20. Prevents session metadata bloat.
- **Context switch:** When `active_tool` changes from its previous value, all existing directives are cleared before installing new ones. Clean slate.
- **Single-use consumption:** Directive with `single_use: true` is removed from context immediately after matching.
- **TTL expiry:** `created_at_ms + ttl_ms < now_ms` — lazy-pruned during routing. No background timers.
- **Default TTL:** 5 minutes (300000ms). Tools can override per-directive.
- **Missing context on load:** If `Session.metadata["router_context"]` is absent (old sessions), `RouterContext::default()` is used — no active tool, no directives. Equivalent to a fresh conversation.

### Directive Matching Implementation

Since matching is **whole-message** (not substring), directives use a `HashMap<String, usize>` mapping lowercased trigger strings to directive indices. `Exact` and `OneOf` triggers are inserted into this map. Lookup is O(1) — lowercase the message, trim, check the map. No AC automaton needed for directives.

`Pattern` triggers are checked only if the hashmap misses. Patterns are compiled lazily via `regex::Regex::new()` and cached. Pattern strings are limited to 256 chars to prevent ReDoS. Compilation failures are logged and the directive is skipped.

For typical directive counts (2-6 literals), the entire matching step is sub-microsecond.

## Static Rules — Tool Trait Extension

```rust
// Added to Tool trait in src/agent/tools/base/mod.rs

pub trait Tool: Send + Sync {
    // ... existing methods ...

    /// Static routing rules. Called once at registration, compiled into router.
    fn routing_rules(&self) -> Vec<StaticRule> {
        Vec::new()  // default: no static rules
    }
}
```

```rust
// src/router/rules.rs

pub struct StaticRule {
    pub tool: String,
    pub trigger: DirectiveTrigger,
    pub params: serde_json::Value,
    /// Only matches when this tool is the active_tool in RouterContext.
    pub requires_context: bool,
}
```

### Collection

`ToolRegistry` collects all `routing_rules()` at registration time and exposes them for `MessageRouter::new()`. The global AC automaton is built once from all static rule triggers. Per-message cost: one AC scan, zero allocations for non-matches.

### Examples

**RSS tool:**
```rust
fn routing_rules(&self) -> Vec<StaticRule> {
    vec![
        StaticRule {
            tool: "rss".into(),
            trigger: DirectiveTrigger::OneOf(vec!["next".into(), "more".into()]),
            params: json!({"action": "next"}),
            requires_context: true,
        },
        StaticRule {
            tool: "rss".into(),
            trigger: DirectiveTrigger::Exact("done reviewing".into()),
            params: json!({"action": "done"}),
            requires_context: true,
        },
    ]
}
```

**Cron tool:**
```rust
fn routing_rules(&self) -> Vec<StaticRule> {
    vec![
        StaticRule {
            tool: "cron".into(),
            trigger: DirectiveTrigger::Exact("list jobs".into()),
            params: json!({"action": "list"}),
            requires_context: false,
        },
    ]
}
```

## Config Rules — User-Defined Prefix Commands

```rust
// src/config/schema/router.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    /// Command prefix. Default "!". Avoids collision with Slack/Discord slash commands.
    #[serde(default = "default_prefix")]
    pub prefix: String,
    /// User-defined command rules.
    #[serde(default)]
    pub rules: Vec<ConfigRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigRule {
    pub trigger: String,
    pub tool: String,
    pub params: serde_json::Value,
}

fn default_prefix() -> String { "!".into() }
```

### Config example

```json
{
  "router": {
    "prefix": "!",
    "rules": [
      {"trigger": "weather", "tool": "weather", "params": {"location": "$1"}},
      {"trigger": "todo", "tool": "todoist", "params": {"action": "list_tasks"}}
    ]
  }
}
```

### Resolution

`!weather portland` → strip prefix → lookup `weather` in config rules → match → substitute `$1` = `portland` → `DirectDispatch("weather", {"location": "portland"})`.

Substitution: `$1`, `$2`, ... are positional args (whitespace-split after command word). `$*` is the entire remainder. No regex. Sub-microsecond.

Stored as `HashMap<String, ConfigRule>` keyed by trigger word. O(1) lookup.

## Dynamic Directives — How Tools Establish Context

Tools set active context and register follow-up actions via `ToolResult.metadata`:

```json
{
  "active_tool": "rss",
  "action_directives": [
    {
      "trigger": {"OneOf": ["yes", "accept", "ok"]},
      "tool": "rss",
      "params": {"action": "accept", "article_ids": ["abc123"]},
      "single_use": true,
      "ttl_ms": 300000
    },
    {
      "trigger": {"OneOf": ["no", "reject", "skip"]},
      "tool": "rss",
      "params": {"action": "reject", "article_ids": ["abc123"]},
      "single_use": true,
      "ttl_ms": 300000
    }
  ]
}
```

### Flow

1. User says "show me articles" → LLM calls `rss.next`
2. RSS tool returns article + metadata with `active_tool: "rss"` and accept/reject directives
3. Agent loop extracts metadata → updates `RouterContext` → saves session
4. User says "yes" → Router matches `OneOf(["yes", "accept", ...])` → `DirectDispatch`
5. RSS accept executes → returns next article + new directives (replacing old ones)
6. Cycle continues without LLM involvement

### Metadata Keys

Two new well-known keys in `ToolResult.metadata`:
- `"active_tool"`: `String` — sets `RouterContext.active_tool`
- `"action_directives"`: `Vec<ActionDirective>` (JSON) — replaces `RouterContext.action_directives`

These join existing sideband keys (`"suggested_buttons"`, `"display_text"`).

### Relationship with Buttons

Buttons and directives coexist on the same tool result:
- **Buttons** are a UI concern — rendered by channels (Block Kit, Discord components). Visual affordance.
- **Directives** are a routing concern — tell the router what text responses mean. Invisible to the user.

A tool can emit both: buttons for click-based interaction, directives for text-based shortcuts. Clicking "Accept" and typing "yes" produce the same `DirectDispatch`.

Buttons also carry `ActionDispatchPayload` in their context field for the structured dispatch path (priority 1 in routing). This is belt-and-suspenders: if the router somehow doesn't match a directive, the button's structured payload still works.

## GuidedLLM Path

When the router detects an active tool context but can't deterministically resolve the message, it returns `GuidedLLM`:

```rust
GuidedLLM {
    tool_subset: vec!["rss".to_string()],
    context_hint: "The user is reviewing RSS articles. Current article: 'How Rust Changed My Life' from blog.example.com. Available actions: accept, reject, next, done, stats.".to_string(),
}
```

### What the agent loop does with it

1. **Tool filtering:** Only include tool definitions for tools in `tool_subset` when building the LLM request. Fewer tokens, faster response, less confusion. Deferred tools activated during the session (via `tool_search`) are included if they match the active tool's category.
2. **Context hint injection:** Append `context_hint` to the system prompt as a `## Active Interaction` section (distinct from the existing `## Current Context` section which carries date/timezone). Gives the LLM the "what's happening right now" frame.

### Context hint construction

The hint is built from `RouterContext`:
- Active tool name
- Last tool result's `display_text` (if available, truncated to 500 chars)
- Available actions from active directives (listed as keywords)

The router constructs this from data it already has — no extra IO.

## Agent Loop Integration

### New flow in `process_message_unlocked()`

```
process_message_unlocked(msg):
    if msg.channel == "system": return process_system_message()

    typing indicator (fire-and-forget)
    event-triggered cron jobs (background)
    acquire session lock
    load session
    extract RouterContext from session.metadata["router_context"]

    decision = router.route(&msg, &router_context)

    match decision:
        DirectDispatch { tool, params, source }:
            log: info!("direct dispatch: tool={tool} source={source}")
            secret-scan params via LeakDetector
            validate tool exists in registry
            reject if tool.requires_approval()
            execute via ToolRegistry::execute()
            extract directives from result.metadata → update RouterContext
            save RouterContext to session.metadata, save session
            record synthetic session history (user + assistant messages)
            return OutboundMessage with result content + suggested_buttons

        GuidedLLM { tool_subset, context_hint }:
            secret scanning on message content
            prompt guard
            build messages (inject context_hint into system prompt)
            run_agent_loop (tool definitions filtered to tool_subset)
            extract directives from tool results → update RouterContext
            save session
            return OutboundMessage

        FullLLM:
            secret scanning on message content
            prompt guard
            build messages
            run_agent_loop (all tools, today's behavior)
            extract directives from tool results → update RouterContext
            save session
            return OutboundMessage
```

### Session loading moved earlier

Currently, session loading happens after prompt guard. With the router, it moves before routing — the router needs `RouterContext` from the session. This is safe: the session load is a local SQLite read (sub-millisecond), and it happens after the session lock is acquired.

### `process_direct_with_overrides()` integration

`AgentRunOverrides` gains `action: Option<ActionDispatch>`. When `Some`, the same `DirectDispatch` path executes. Existing callers pass `action: None`.

### Directive extraction

After tool execution in all three paths, the agent loop checks `ToolResult.metadata` for `"active_tool"` and `"action_directives"`.

**For DirectDispatch:** Single tool call — extract from that result.

**For GuidedLLM and FullLLM:** The iteration loop may call multiple tools across multiple iterations. Only the **last** tool result's `active_tool` and `action_directives` are used. Earlier tool results' directives are overwritten. This matches the mental model: the last tool action establishes the current context.

When directives are found:

1. If `active_tool` changed from the previous value, clear all existing directives (context switch).
2. Replace directives in `RouterContext` with new ones (full replacement, not append). Cap at 20.
3. Save updated `RouterContext` to `Session.metadata["router_context"]`.

### No `follow_up_action` chaining

The earlier action dispatch spec defined a `follow_up_action` metadata key for automatic re-dispatch chains. This is **replaced** by action directives. Instead of a tool saying "after this, automatically call X", it says "if the user responds with Y, call X." The user remains in control. The `follow_up_action` key and `MAX_DISPATCH_CHAIN_DEPTH` constant from the dispatch module are not used by the router.

This extraction happens in a shared helper called from all three paths.

### Session history for DirectDispatch

After a successful direct dispatch, two synthetic messages are appended to the session:

**User message:**
```
[action: {tool}.{action} via {source}]
```
Example: `[action: rss.accept via directive]`

**Assistant message:**
The raw tool result content.

Uses `Session::add_message(role, content, extra)`.

## What Gets Removed

### Modules deleted entirely
- `src/agent/loop/intent/` — regex + semantic intent classification. Router replaces this entirely.
- `src/agent/loop/tool_filter.rs` — category-based tool filtering. GuidedLLM's `tool_subset` replaces this.

### Modules gutted
- `src/agent/loop/hallucination.rs` — remove layers 0, 2, 3. Keep Layer 1 with its single-retry correction flow: if `contains_action_claims()` detects fabricated actions and no tools were called, inject a correction message and retry once. This is the safety net for the FullLLM path. Remove `CorrectionState` state machine (overkill for one layer). Remove: `is_false_no_tools_claim()`, `is_legitimate_refusal()`, `mentions_multiple_tools()`, `mentions_any_tool()`, `MAX_LAYER0_CORRECTIONS`, `is_clarification_question()` (was in intent module, consumed only by Layer 2). The `user_has_action_intent` parameter is removed from `run_agent_loop_with_overrides()` and all callers — Layer 1 does not depend on it.
- `src/agent/loop/hallucination/helpers.rs` — keep `contains_action_claims()` pattern list. Remove everything else.

### Code removed from `iteration.rs`
- Anti-hallucination system prompt injection ("You have tools available...").
- Tool category filtering and caching (`infer_tool_categories()` calls, cached categories).
- `CorrectionState` construction, threading, and usage. Layer 1 uses a simple `bool` flag instead.
- `user_has_action_intent` parameter from `run_agent_loop_with_overrides()` and all internal threading. Layer 1 does not use it.
- All parameters and code paths for layers 0/2/3 hallucination.

### Code removed from `processing.rs`
- `extract_remember_content()` check — moved into router as a built-in rule.
- `try_remember_fast_path()` method — logic preserved, invocation moved to DirectDispatch handler.
- `classify_and_record_intent()` call and method body.
- Semantic intent classification embedding calls.

### Database writes stopped
- `intent_events` table — stop writing new rows. Table kept for historical data. Remove `record_intent_event()` calls from hallucination and intent code paths.
- `oxicrab stats intent` CLI subcommand — remove entirely. Drop `get_intent_stats()` and `get_recent_hallucinations()` DB methods.

### Config fields removed
- Any hallucination-specific config toggles (if they exist).
- Intent classification config (if it exists).

### Observability

All routing decisions are logged:
```
info!("router: decision=DirectDispatch tool={tool} source={source} channel={channel}")
info!("router: decision=GuidedLLM tool_subset=[{tools}] channel={channel}")
debug!("router: decision=FullLLM channel={channel}")
```
`FullLLM` is `debug!` (most common case, would be noisy at `info!`). Direct dispatch and guided decisions are `info!` since they represent the router actively intervening.

## What Stays Unchanged

- **Security:** prompt guard, leak detection (inbound + tool output), exfiltration guard, secret scanning, shell sandboxing
- **Model routing:** complexity scoring, per-provider temperature, model routing config
- **Session:** session management, compaction, pre-flush, orphan cleanup
- **Cognitive:** checkpoint tracking, breadcrumbs, pressure messages
- **Tool infrastructure:** `Tool` trait (extended with `routing_rules()`), `ToolResult.metadata` sideband, `ToolRegistry::execute()` with full middleware pipeline, tool parameter auto-casting, schema hint injection on errors
- **Channel infrastructure:** button rendering (Block Kit, Discord components), reaction emoji lifecycle, `suggested_buttons` metadata, `merge_suggested_buttons()`
- **`ButtonSpec` and `add_buttons` tool** — LLM can still add buttons in FullLLM/GuidedLLM paths
- **Temperature switching** — 0.7 initial, 0.0 after tool calls
- **Hallucination Layer 1** — `contains_action_claims()` regex as lightweight safety net
- **Remember logic** — same behavior, invocation moved from processing.rs into router DirectDispatch handler
- **Webhook infrastructure** — signature validation, replay protection, template substitution

## Dispatch Infrastructure (Retained from Earlier Design)

The following types from the earlier action dispatch design are retained and used by the router:

- `ActionDispatch`, `ActionSource`, `ActionDispatchPayload` — structured payloads for buttons and webhooks
- `DispatchContextStore` — in-memory LRU for Discord button context (Discord can't carry JSON in button payloads)

These live in `src/dispatch/mod.rs` as a top-level module.

### Button context format

All tools adopt the `ActionDispatchPayload` JSON format for `ButtonSpec.context`:

```json
{"tool": "rss", "params": {"action": "accept", "article_ids": ["abc123"]}}
```

Slack deserializes this in `handle_interactive_payload()` → creates `ActionDispatch` on `InboundMessage`.
Discord stores payloads in `DispatchContextStore` on render, looks up on click.

## Files Changed

### New
- `src/router/mod.rs` — `MessageRouter`, `RoutingDecision`, `DispatchSource`, `route()`, AC automaton management
- `src/router/context.rs` — `RouterContext`, `ActionDirective`, `DirectiveTrigger`, serialization, TTL/lifecycle
- `src/router/rules.rs` — `StaticRule`, `ConfigRule`, `$N` substitution, config parsing
- `src/dispatch/mod.rs` — `ActionDispatch`, `ActionSource`, `ActionDispatchPayload`, `DispatchContextStore`, constants
- `src/config/schema/router.rs` — `RouterConfig`, config rule schema

### Modified
- `src/lib.rs` — add `pub mod router;`, `pub mod dispatch;`
- `src/agent/tools/base/mod.rs` — add `fn routing_rules()` default method to `Tool` trait
- `src/agent/tools/registry/mod.rs` — collect `routing_rules()` at registration, expose for router
- `src/agent/loop/mod.rs` — `AgentLoop` gains `router: Arc<MessageRouter>`, constructed at startup
- `src/agent/loop/processing.rs` — replace pipeline top with `router.route()`, remove intent classification, remove remember fast path check, add directive extraction, move session load earlier
- `src/agent/loop/iteration.rs` — remove anti-hallucination prompt injection, remove tool category filtering/caching, accept `tool_subset`/`context_hint` from GuidedLLM, remove CorrectionState threading, extract directives from tool result metadata
- `src/agent/loop/hallucination.rs` — gut to Layer 1 only: `contains_action_claims()` check, no state machine, no retry flow
- `src/agent/loop/config.rs` — `AgentRunOverrides` gains `action: Option<ActionDispatch>`
- `src/bus/events/mod.rs` — `InboundMessage` gains `#[serde(skip)] action: Option<ActionDispatch>`, builder gains `.action()`
- `src/channels/slack/mod.rs` — `handle_interactive_payload()` creates `ActionDispatch` from button context
- `src/channels/discord/mod.rs` — `DispatchContextStore` integration for button dispatch
- `src/gateway/mod.rs` — webhook dispatch path
- `src/config/schema/mod.rs` — add `RouterConfig` to top-level config, add `WebhookDispatchConfig` to `WebhookConfig`
- All tools with buttons — structured button context (`ActionDispatchPayload`), `routing_rules()` impl, `action_directives`/`active_tool` in result metadata:
  - `src/agent/tools/rss/articles.rs`
  - `src/agent/tools/google_calendar/mod.rs`
  - `src/agent/tools/google_mail/mod.rs`
  - `src/agent/tools/google_tasks/mod.rs`
  - `src/agent/tools/todoist/mod.rs`
  - `src/agent/tools/github/mod.rs`
  - `src/agent/tools/cron/mod.rs`

### Deleted
- `src/agent/loop/intent/` — entire module (regex + semantic intent classification)
- `src/agent/loop/tool_filter.rs` — category-based tool filtering
- Dead code in `hallucination.rs` and `hallucination/helpers.rs` — layers 0/2/3, CorrectionState, helper functions

## Error Handling

| Scenario | Behavior |
|----------|----------|
| Tool not found (DirectDispatch) | Return outbound: "Action failed: tool '{name}' is not available." |
| Tool requires approval (DirectDispatch) | Return outbound: "Action failed: tool '{name}' requires approval." |
| Tool execution error | Return error content as outbound message |
| Malformed button payload | Fall through to FullLLM path (graceful degradation) |
| Directive TTL expired | Skip directive, continue routing priority chain |
| Context switch during DirectDispatch | Old directives already cleared, new ones installed from result |
| Secrets in dispatch params | `LeakDetector::redact()` before tool execution |
| Session load failure | Treat as empty RouterContext, route as FullLLM |
| Discord button expired (>15 min) | DispatchContextStore miss → FullLLM fallback |
| Config rule `$N` with missing arg | Substitute empty string (same as shell behavior) |
| Config rule tool not registered | Fall through to FullLLM (not an error — LLM may interpret) |
| Prompt injection in config rule `$N`/`$*` | Prompt guard runs on substituted values before dispatch |
| Directive regex > 256 chars | Directive skipped, warning logged |
| Directive regex compilation failure | Directive skipped, warning logged |

## Testing Strategy

### Router unit tests (pure logic, no IO)
- `route()` with `InboundMessage.action` → `DirectDispatch`
- `route()` with active directive matching "yes" → `DirectDispatch`
- `route()` with prefix command `!weather portland` → `DirectDispatch` with `$1` substitution
- `route()` with `$*` substitution → entire remainder captured
- `route()` with static rule + active context → `DirectDispatch`
- `route()` with static rule + wrong active context → not matched
- `route()` with static rule + `requires_context: false` → matches without context
- `route()` with active tool but no directive/rule match → `GuidedLLM`
- `route()` with no context, no matches → `FullLLM`
- Directive TTL expiry → skipped
- Single-use directive consumed → removed from context
- Context switch clears old directives
- `RouterContext` serde round-trip
- Config rule `$1`, `$2`, `$*` substitution
- Config rule missing arg → empty string
- Prefix stripping and command word parsing
- Case sensitivity (directives should be case-insensitive)
- Empty message → `FullLLM`

### Router performance tests
- 100 directives + 50 static rules + 20 config rules → route < 100μs
- AC automaton rebuild with 200 patterns < 1ms

### Integration tests
- End-to-end DirectDispatch: InboundMessage with action → tool executes → outbound has result
- End-to-end directive cycle: tool result sets directives → next message matches → direct dispatch → new directives
- Context switch: change active_tool → old directives gone
- GuidedLLM: verify tool_subset and context_hint reach LLM request
- Session persistence: directives survive save/load cycle
- Hallucination Layer 1 still fires in FullLLM path
- Remember fast path via router → same behavior as before
- Webhook with dispatch config → DirectDispatch
- Button click (Slack) → DirectDispatch
- Button click (Discord) → DispatchContextStore → DirectDispatch

### Not tested (removed)
- Intent classification accuracy
- Hallucination layers 0, 2, 3
- Tool category filtering
