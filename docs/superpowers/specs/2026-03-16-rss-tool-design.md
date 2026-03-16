# RSS Tool Design Spec

## Overview

An action-based RSS/Atom feed tool for oxicrab that scans feeds, filters articles through a two-layer learning system (LinTS pre-ranking + LLM triage), and delivers summaries with accept/reject feedback buttons. The tool includes a deterministic onboarding state machine, automatic cron scheduling, and robust lifecycle enforcement.

## Goals

- Scan RSS/Atom feeds on a configurable schedule (default: every 6 hours)
- Learn user preferences at the article level using Linear Thompson Sampling (LinTS)
- Surface relevant articles with LLM-generated summaries
- Collect accept/reject feedback to improve recommendations over time
- Provide a guided onboarding experience that bootstraps the system completely
- Be robust: every action validates preconditions and fails with clear corrective instructions

## Non-Goals

- Non-RSS sources (scrapers, APIs) — RSS/Atom only for V1
- Interactive buttons in the tool itself — uses existing `suggested_buttons` metadata sideband
- Real-time streaming — batch scan and deliver model
- Full-text indexing or search across articles
- Multi-user support — singleton profile, single-user. Multi-user would require keying by user/session ID

---

## Architecture

### Two-Layer Learning System

**Layer 1: LinTS (Linear Thompson Sampling) — Pre-ranker**

LinTS maintains a Bayesian linear model over article features. For each article, it samples from the posterior weight distribution and computes a relevance score. This is sub-millisecond per article, no LLM call required.

Feature vector per article:
- Source (one-hot encoded per feed)
- Tags (multi-hot, assigned by LLM during triage or from RSS categories)
- Title keyword signals (presence of domain terms from the user's interest profile)
- Source age (how long this feed has been tracked)

Model state:
- Mean vector `mu` (dimension = number of features) with prior `N(0, I)`
- Covariance matrix `Sigma`
- Stored as serialized `nalgebra` `DVector`/`DMatrix` blobs in SQLite (raw `f64` bytes via `nalgebra::storage`)
- Maximum feature dimension: 200. When exceeded, prune least-informative features (lowest absolute `mu` weight with high `Sigma` diagonal, indicating uncertain and unimpactful features) and log a warning

Non-stationarity handling: each scan cycle inflates the covariance by `Sigma = Sigma + epsilon * I` (default epsilon = 0.01), increasing uncertainty over time and encouraging re-exploration of features that haven't been validated recently.

Feature expansion: when a new feed is added or a new tag appears, `mu` and `Sigma` are extended with prior values for the new dimensions.

This layer takes ~100 raw articles down to ~20 candidates.

**Layer 2: LLM Triage**

The agent loop receives the top ~20 pre-ranked articles plus the user's interest profile and recent accept/reject examples. The LLM applies nuanced judgment (context, novelty, timeliness) and generates summaries for the articles that pass.

This layer takes ~20 candidates down to ~5-10 surfaced articles.

**Feedback loop:** Accept/reject updates flow back to both layers. LinTS gets a Bayesian posterior update. The LLM sees recent feedback examples in-context (few-shot, no training).

### Tool Structure

Single action-based tool (`rss`) with 11 actions:

| Action | Category | Mutating | Description |
|--------|----------|----------|-------------|
| `onboard` | Onboarding | Yes | State machine: detects current step, returns next instruction or auto-executes mechanical steps |
| `set_profile` | Onboarding | Yes | Store/update interest description (min 20 chars) |
| `add_feed` | Feed mgmt | Yes | Add feed URL — fetches and validates RSS/Atom on add |
| `remove_feed` | Feed mgmt | Yes | Remove feed and cascade to articles |
| `list_feeds` | Feed mgmt | No | List feeds with stats |
| `scan` | Scanning | Yes | Fetch feeds, dedup, pre-filter, LinTS rank, return candidates for LLM |
| `get_articles` | Articles | No | List articles by status/feed/date with pagination |
| `accept` | Feedback | Yes | Mark article(s) accepted, update LinTS model |
| `reject` | Feedback | Yes | Mark article(s) rejected, update LinTS model |
| `get_article_detail` | Articles | No | Fetch full page content on demand, mark as read |
| `feed_stats` | Analytics | No | Per-feed acceptance rates, model feature weights, trends |

---

## Onboarding State Machine

```
needs_profile → needs_feeds → needs_calibration → complete
```

Cron job creation happens automatically at the end of calibration (no separate state).

### Principle

If a step does not require natural language interaction, the tool does it directly. The LLM is a relay for human communication, not an executor of mechanical steps.

| Step | Who does it | Why |
|------|------------|-----|
| Profile prompt | LLM relays tool's message | Needs natural language |
| Profile storage | Tool validates + stores | Mechanical |
| Feed suggestions | Tool generates from hardcoded map | Mechanical |
| Feed validation | Tool fetches + parses on `add_feed` | Mechanical |
| Calibration scan | Tool auto-triggers on entering state | Mechanical |
| Calibration article presentation | LLM presents articles | Needs natural language |
| Accept/reject | Tool updates DB + model | Mechanical |
| Cron job creation | Tool creates directly via `CronService` | Mechanical |

### State Transitions

**`needs_profile`**: `onboard` returns a prompt asking the user to describe their interests. `set_profile` validates >= 20 chars, stores, transitions to `needs_feeds`.

**`needs_feeds`**: `onboard` returns the user's profile plus a curated list of suggested feeds based on profile keywords. The mapping covers at least 5 categories (e.g., "rust", "ai/ml", "web-dev", "security", "devops") with 3-5 feeds per category. Implementation detail — the exact mapping is maintained in `onboard.rs`. Transitions to `needs_calibration` when >= 1 feed exists.

**`needs_calibration`**: `onboard` auto-triggers a scan, returns ~10 recent articles for review. No LinTS scoring (model is cold) — just newest articles across sources. Transitions to `complete` when >= 5 accept/reject decisions exist. On transition: tool creates cron job directly via `CronService::add_job()` using the channel and chat_id from the current `ExecutionContext`, stores job ID in `rss_profile.cron_job_id`. Safety: checks `ctx.metadata` for `IS_CRON_JOB` — if called during a cron execution, skips cron job creation and returns a message telling the user to complete onboarding via direct chat.

**`complete`**: `onboard` returns summary stats. Idempotent.

### Action Gate Matrix

| Action | `needs_profile` | `needs_feeds` | `needs_calibration` | `complete` |
|--------|:-:|:-:|:-:|:-:|
| `onboard` | yes | yes | yes | yes (status) |
| `set_profile` | yes | yes | yes | yes |
| `add_feed` | no | yes | yes | yes |
| `remove_feed` | no | yes | yes (keeps >=1) | yes |
| `list_feeds` | no | yes | yes | yes |
| `scan` | no | no | no | yes |
| `get_articles` | no | no | yes (calibration) | yes |
| `accept` | no | no | yes | yes |
| `reject` | no | no | yes | yes |
| `get_article_detail` | no | no | yes | yes |
| `feed_stats` | no | no | no | yes |

### Error Format

Every gated action returns structured errors:

```json
{
  "error": true,
  "message": "This action requires onboarding to be complete.",
  "onboarding_state": "needs_calibration",
  "progress": "2/5 reviews completed",
  "next_action": "Call 'onboard' to continue calibration."
}
```

---

## Scan Flow

**Phase 1: Fetch**
1. Load all enabled feeds from DB
2. Validate all feed URLs through `validate_and_resolve()` (SSRF protection — blocks internal IPs, cloud metadata endpoints, embedded credentials). DNS is pinned to prevent TOCTOU rebinding.
3. Fetch all validated feeds concurrently (bounded to 8 concurrent requests)
4. Per-feed: respect `ETag`/`Last-Modified` (304 = skip), timeout per `scan_timeout` config
5. Parse with `feed-rs` — extract title, URL, author, published date, description/snippet
6. Per-feed error handling: log error, increment `consecutive_failures`, continue to next feed
7. Auto-disable feeds after 5 consecutive failures with reason in `last_error`

**Phase 2: Pre-filter (no LLM)**
1. Deduplicate by article URL against DB (`UNIQUE` constraint)
2. Reject articles with empty/whitespace-only titles
3. Reject articles older than 7 days (stale on arrival)
4. Cap at `max_articles_per_feed` per source per scan
5. Insert surviving articles into DB with status `new`

**Phase 3: LinTS Ranking**
1. Inflate covariance: `Sigma = Sigma + epsilon * I`
2. For each new article, build feature vector (source one-hot + tag multi-hot + keyword signals)
3. Sample `w ~ N(mu, Sigma)` (one sample per scan cycle, shared across articles)
4. Score each article: `score = w^T * x`
5. Rank by score, take top `candidates_per_scan` (default 20)

**Phase 4: Return to Agent**
1. Load user's interest profile from DB
2. Return formatted text to the agent containing:
   - The interest profile for context
   - Ranked articles, each with: short ID, title, source name, author (if available), snippet (first 200 chars of description), LinTS score
   - Per-source summary (articles fetched, articles passing pre-filter)
   - Instruction to triage and summarize
3. LLM triages, generates summaries, presents with accept/reject buttons

**Phase 5: Hygiene**
1. Purge articles older than `purge_days` in `new`/`triaged` status
2. Update `last_fetched_at_ms` on all successfully fetched feeds

---

## Robustness & Lifecycle Enforcement

### Scan Resilience
- Per-feed try/catch — one feed failing doesn't block others
- Feed errors recorded in `last_error`, surfaced in `list_feeds` and `feed_stats`
- After 5 consecutive failures, feed auto-disables with reason
- Deduplication enforced at DB level (`UNIQUE` on article URL)

### Accept/Reject
- Article already in terminal state → per-ID error, not blanket failure
- Unknown article ID → error with `get_articles` suggestion
- Empty ID list → error
- Model update failure → accept/reject succeeds in DB, model update queued for next scan (feedback never lost)
- Batch operations: accepts list of IDs, reports per-ID results

### URL Security
- All feed URLs validated through `validate_and_resolve()` on `add_feed` AND on every `scan` cycle
- Prevents SSRF against internal networks (169.254.x.x, 10.x.x.x, etc.) and cloud metadata endpoints
- DNS pinning prevents TOCTOU rebinding between validation and fetch
- `get_article_detail` full-page fetch also validates article URLs through `validate_and_resolve()`

### Feed Management
- `add_feed` validates URL security, fetches, and parses before inserting — invalid feed caught immediately
- Unreachable URL → clear error with timeout details
- Valid HTTP but not RSS/Atom → clear error
- `remove_feed` with accepted articles → succeeds with warning about article count
- `remove_feed` during calibration → allowed but enforces >= 1 feed remains

### LinTS Model
- Corrupted model blob → re-initialize from prior, log warning, don't block scan
- Feature vector expansion → extend `mu` with zeros, extend `Sigma` with prior diagonal
- Covariance inflation prevents the model from becoming overconfident
- Maximum feature dimension: 200. Exceeding this prunes least-informative features and logs a warning

### Cron Job Integrity
- On `scan`, verify `rss_profile.cron_job_id` still exists via `CronService`
- If deleted externally, surface warning in `feed_stats`: "Scheduled scanning is not active"
- Don't auto-recreate — let the user decide

### Article Content
- Title + description/snippet stored by default (from RSS)
- Full page content fetched on demand via `get_article_detail`
- Full fetch failure (network error, paywall) → return snippet with note, don't error
- Marks article as read on detail view

### Profile Updates
- Changing profile post-calibration updates the interest text but does not reset the LinTS model
- New keyword signals are naturally incorporated in subsequent scans as the feature vector is rebuilt per article

---

## Database Schema

Migration version is tentative — verify the current `user_version` in `migrations.rs` before implementation and use the next available version.

```sql
CREATE TABLE rss_feeds (
    id TEXT PRIMARY KEY,
    url TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    site_url TEXT,
    last_fetched_at_ms INTEGER,
    last_error TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE rss_articles (
    id TEXT PRIMARY KEY,
    feed_id TEXT NOT NULL REFERENCES rss_feeds(id) ON DELETE CASCADE,
    url TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    author TEXT,
    published_at_ms INTEGER,
    fetched_at_ms INTEGER NOT NULL,
    description TEXT,
    full_content TEXT,
    summary TEXT,
    status TEXT NOT NULL DEFAULT 'new',
    read INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE rss_article_tags (
    article_id TEXT NOT NULL REFERENCES rss_articles(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    PRIMARY KEY (article_id, tag)
);

CREATE TABLE rss_profile (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    interests TEXT NOT NULL,
    onboarding_state TEXT NOT NULL DEFAULT 'needs_profile',
    cron_job_id TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE rss_model (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    feature_index TEXT NOT NULL,  -- JSON object: {"feed:abc123": 0, "tag:rust": 1, ...}
    mu BLOB NOT NULL,             -- nalgebra DVector<f64>, raw f64 bytes
    sigma BLOB NOT NULL,          -- nalgebra DMatrix<f64>, raw f64 bytes (row-major)
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX idx_rss_articles_feed ON rss_articles(feed_id, status);
CREATE INDEX idx_rss_articles_status ON rss_articles(status, created_at_ms);
CREATE INDEX idx_rss_articles_published ON rss_articles(published_at_ms);
CREATE INDEX idx_rss_article_tags_tag ON rss_article_tags(tag);
```

---

## Configuration

New `RssConfig` in `src/config/schema/tools.rs`:

```rust
pub struct RssConfig {
    pub enabled: bool,                // default true
    pub scan_timeout: u64,            // per-feed fetch timeout in seconds, default 15
    pub max_articles_per_feed: usize, // per scan cycle, default 50
    pub purge_days: u64,              // auto-purge unreviewed articles after N days, default 90
    pub candidates_per_scan: usize,   // articles sent to LLM after LinTS ranking, default 20
    pub covariance_inflation: f64,    // LinTS drift epsilon, default 0.01
}
```

No API keys required. Tool registers whenever `MemoryDB` is available and `enabled` is `true` (default). The `enabled` flag gates registration — when `false` or config absent, the tool is not registered.

The RSS tool overrides `execution_timeout()` to 5 minutes (300s) to accommodate scanning many feeds concurrently. Default 2-minute timeout is insufficient for worst-case scenarios (40+ feeds at 15s timeout each in concurrent batches).

---

## Tool Capabilities

```rust
ToolCapabilities {
    built_in: true,
    network_outbound: true,
    subagent_access: SubagentAccess::ReadOnly,
    actions: actions![
        onboard, set_profile, add_feed, remove_feed,
        list_feeds: ro, scan, get_articles: ro,
        accept, reject, get_article_detail: ro, feed_stats: ro
    ],
    category: ToolCategory::Web,
}
```

---

## File Structure

### New Files

```
src/agent/tools/rss/
├── mod.rs          -- RssTool struct, Tool trait impl, action dispatch
├── feeds.rs        -- add_feed, remove_feed, list_feeds, RSS parsing, validation
├── articles.rs     -- get_articles, get_article_detail, accept, reject
├── scanner.rs      -- scan orchestration, fetch, dedup, pre-filter, LinTS ranking
├── onboard.rs      -- state machine, set_profile, suggested feeds, cron creation
├── model.rs        -- LinTS: feature encoding, posterior update, sampling, covariance inflation
├── tests.rs        -- unit tests

src/agent/memory/memory_db/rss.rs  -- DB access methods
```

### Modified Files

| File | Change |
|------|--------|
| `Cargo.toml` | Add `feed-rs`, `nalgebra`, `rand_distr` |
| `src/agent/tools/setup/mod.rs` | Add `register_rss()`, `rss_config` to `ToolBuildContext` |
| `src/agent/loop/config.rs` | Add `rss_config` to `ToolConfigs` |
| `src/config/schema/tools.rs` | Add `RssConfig` |
| `src/config/schema/mod.rs` | Add `rss` field, wire through `from_config()` |
| `src/agent/memory/memory_db/mod.rs` | Add `mod rss;` |
| `src/agent/memory/memory_db/migrations.rs` | Add migration v5 |
| `src/agent/tools/mod.rs` | Add `mod rss;` |
| `tests/common/mod.rs` | Add `rss_config` to `create_test_agent_with()` |
| `config.example.json` | Add `rss` section |
| `docs/_pages/tools.html` | Add RSS tool documentation |
| `README.md` | Add `rss` to tool list |
| `CLAUDE.md` | Add RSS patterns to architecture section |

### Dependencies

| Crate | Purpose |
|-------|---------|
| `feed-rs` | RSS/Atom parsing |
| `nalgebra` | LinTS matrix operations (DVector, DMatrix, Cholesky) |
| `rand_distr` | Multivariate normal sampling for LinTS |

`reqwest` and `rand` are already in the project.

RSS is behind a cargo feature flag `tool-rss` (default enabled), consistent with the project's pattern of optional features. Users who don't need RSS can exclude it to avoid the `nalgebra` compile-time cost:

```toml
[features]
tool-rss = ["dep:feed-rs", "dep:nalgebra", "dep:rand_distr"]
```

---

## Cron Integration

The RSS tool takes `Arc<CronService>` as a constructor dependency (same pattern as `CronTool`). During onboarding completion, it creates a cron job directly:

- Schedule: `0 */6 * * *` (every 6 hours)
- Payload: `"Scan RSS feeds using the rss tool scan action. Filter articles by my interest profile, summarize the top candidates, and present them with accept/reject options."`
- Target channel/chat_id: taken from `ExecutionContext` during the `onboard` call
- Job ID stored in `rss_profile.cron_job_id`

Safety: `onboard` checks `ctx.metadata` for `IS_CRON_JOB` before creating the cron job. If called during a cron execution, it skips cron job creation and returns a message telling the user to complete onboarding via direct chat. This prevents the same infinite feedback loop that the cron tool guards against, since the RSS tool bypasses `CronTool` and calls `CronService::add_job()` directly.

The cron job triggers a standard agent turn. The agent calls `rss.scan`, receives ranked candidates, triages with the LLM, and delivers summaries to the channel.

---

## Suggested Buttons

`scan` returns `suggested_buttons` with accept/reject options for the first article. `accept` and `reject` actions return navigation buttons:

```json
[
  { "id": "rss-next", "label": "Next Article", "style": "primary" },
  { "id": "rss-done", "label": "Done Reviewing", "style": "default" }
]
```

Button taps arrive as `[button:rss-next]` via existing channel infrastructure. The LLM interprets and calls appropriate actions.

Article IDs use short format (first 8 chars of UUID) for readability. The tool accepts both short and full UUIDs. If a short ID matches multiple articles, the tool returns an error with a message suggesting the full UUID.
