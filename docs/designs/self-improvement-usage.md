# Self-improvement Tracks 1 + 2: Usage Guide

This is the operator-facing guide for the self-improvement features
that ship in oxicrab today. The companion design doc
[`docs/designs/self-improvement.md`](self-improvement.md) explains
the rationale and the wider roadmap; this document is just "what's
in the box and how do I use it".

Two tracks are live:

1. **Reflexion-style failure reflection** — Track 1.
2. **Voyager-style skill library** (embedding-indexed retrieval +
   propose/promote workflow + weekly hygiene) — Track 2.

Tracks 3 (PR-only autonomous maintainer) and 4 (harvest the
SICA/DGM modifications) are not yet implemented.

## Track 1: Reflexion failure reflection

### What it does

When any tool returns `is_error = true`, the agent loop optionally
makes a small LLM call that produces:

- `hypothesis` — one-sentence guess at the cause.
- `retry_strategy` — one concrete instruction for the next attempt.

Both are appended to the original tool result content as a
`<reflection>` block, so the next iteration sees them explicitly. A
copy of every reflection (request id, tool, action, error excerpt,
hypothesis, retry strategy, eventual outcome) is written to the
`tool_reflections` SQLite table for offline analysis.

This is the `Reflexion` pattern (Shinn et al., arXiv 2303.11366),
specialised to tool-call failures and bounded so cost stays
predictable.

### When to turn it on

Off by default. Turn it on when:

- You're debugging a recurring tool failure and want to see the
  agent's own self-diagnosis pattern.
- You want to measure whether retries succeed at a useful rate
  before considering harder self-improvement work.

Skip it when:

- Cost predictability is critical and you can't tolerate one extra
  small LLM call per error.
- Your tools fail in ways the LLM has no leverage on (network
  outage, missing credentials).

### Configuration

In `~/.oxicrab/config.toml`:

```toml
[agents.defaults.reflection]
enabled = true        # off by default
maxPerRequest = 2     # hard cap per agent run
maxPerTool = 1        # cap per (tool, action) pair per run
temperature = 0.2     # low for determinism
maxTokens = 200       # response budget
persistToDb = true    # save to tool_reflections
```

| Field | Default | Notes |
|-------|---------|-------|
| `enabled` | `false` | Master switch. |
| `maxPerRequest` | `2` | Caps total reflections in one agent run. |
| `maxPerTool` | `1` | Same `(tool, action)` cannot be reflected on twice. |
| `temperature` | `0.2` | Use a low value for stable parsing. |
| `maxTokens` | `200` | Reflection prompt asks for ≤2 short lines. |
| `persistToDb` | `true` | Disable for tests; leave on otherwise. |

### What an injected reflection looks like

A failed tool result with reflection enabled looks like this in the
next LLM iteration:

```
ToolResult { is_error: true, content:
    "Error: file not found: ./notes.md
    <reflection>
    attempt: 1
    hypothesis: file path was relative to the wrong working directory
    retry_strategy: pass the absolute path or call shell pwd first
    </reflection>"
}
```

The LLM sees both the original error and the structured guidance and
typically produces a different second attempt.

### Querying reflection history

```bash
sqlite3 ~/.oxicrab/workspace/memory/memory.sqlite3 \
  "SELECT tool_name, action, attempt_number, hypothesis, retry_strategy, next_outcome
   FROM tool_reflections
   ORDER BY id DESC LIMIT 20;"
```

The `next_outcome` column is filled by the agent loop after the next
attempt resolves. `success` / `error` / `null` (still pending) are
the typical values.

### Metrics

Prometheus counters when the metrics exporter is enabled:

- `oxicrab_reflection_triggered_total{tool, action}` — reflections
  produced.
- `oxicrab_reflection_llm_error_total{tool}` — reflection LLM call
  failed.

Use these to track adoption rate and the cost of running with
reflection on.

### Rollout suggestion

1. Land config with `enabled = false` (the default).
2. Flip `enabled = true` for a single channel first.
3. After 7 days, query the table for retry-success rate. Aim for
   >50%. If lower, the prompt isn't earning its keep — look at the
   error patterns and consider whether a structural fix is better.
4. If the rate holds up, leave it on.

### Safety

The original error string is redacted through the leak detector
before being sent to the reflection model. The model's hypothesis
and retry-strategy are redacted again before being appended to the
tool result and persisted. Both the prompt and the result share the
existing safety perimeter.

## Track 2: Skill library

### What it does

Three pieces, all building on the existing `~/.oxicrab/workspace/skills/`
directory.

1. **2a — Embedding-indexed retrieval.** `SkillIndex::rebuild()`
   walks the skills tree, computes a SHA-256 of each file, and
   re-embeds only files whose SHA differs from the last index. The
   embedding lives in the `skills_index` SQLite table along with
   `use_count` and `last_used_ms`. `top_k_for_query(query, k)`
   returns the k highest-cosine matches against the embedded query.
2. **2b — Propose/promote workflow.** `propose_skill` writes a
   candidate skill file to a *staged* directory under
   `workspace/skills/staged/`. Staged skills are not loaded into
   the system prompt. `promote_staged_skill` re-runs the safety
   scanner, verifies the staged path is a regular file (no
   symlinks), and moves it into a per-skill directory where it
   becomes active.
3. **2c — Weekly hygiene.** `run_hygiene` calls
   `prune_unused_skill_index(now, 30 days, min_uses=1)` at
   startup. Indexed skills that are at least 30 days old, have
   never been used (`use_count = 0`), and have never been touched
   (`last_used_ms IS NULL`) are dropped from the index — disk files
   are left alone.

### Skill file layout

Same as before. Each skill lives at:

```
~/.oxicrab/workspace/skills/<name>/<name>.md
```

with optional YAML frontmatter:

```markdown
---
name: deploy
description: How to deploy oxicrab to staging
hints:
  - deploy
  - staging
---

# Deploy

1. Run `cargo build --release`.
2. ...
```

The `description` field is what the index embeds. If it's missing,
the index falls back to the first non-blank, non-`#` line in the
body.

### Skill-name rules

`[A-Za-z0-9][A-Za-z0-9_-]{0,63}`:

- 1 to 64 characters.
- Alphanumeric plus `_` and `-`.
- Must NOT start with `_` or `-`.
- Must NOT contain `/`, `\`, or `..` (no path components).

### Building or rebuilding the index

The index is **not** built automatically yet — it's a tool an
operator opts into. Drive it from a custom call site (the public
`SkillIndex` struct lives at `oxicrab::agent::skills::index`):

```rust
use oxicrab::agent::skills::index::SkillIndex;

let index = SkillIndex::new(memory_db.clone(), workspace_skills, builtin_skills);
let n = index.rebuild(&embedding_service)?;
println!("re-indexed {n} skills");
```

`rebuild()` is idempotent and incremental: re-running it after no
changes is a near-no-op.

### Looking up skills for a query

```rust
let hits = index.top_k_for_query(&embedding_service, "how do I deploy?", 3)?;
for hit in hits {
    println!("{:.3}  {}  ({})", hit.score, hit.name, hit.path);
}
```

Each hit records a `use_count` bump and updates `last_used_ms` so
the hygiene job knows the skill is active.

The default `k` is capped at 5 (`DEFAULT_TOP_K_CAP`) to keep the
system-prompt budget bounded.

### Proposing a new skill

```rust
use oxicrab::agent::skills::propose;

let staged = propose::propose_skill(
    &workspace_skills,
    "deploy",
    "---\nname: deploy\ndescription: How to deploy\n---\n\n# Deploy\n\n1. ...",
)?;
```

The body is capped at 32 KB. The safety scanner runs *before* the
file is written; if it matches a blocked pattern (prompt-injection
markers, credential exfiltration signatures, reverse-shell
patterns), the propose call returns `Err`.

The staged file does not become active. List staged skills:

```rust
for name in propose::list_staged(&workspace_skills) {
    println!("staged: {name}");
}
```

### Promoting a staged skill

```rust
let active = propose::promote_staged_skill(&workspace_skills, "deploy")?;
```

This:

1. `symlink_metadata` on the staged path; rejects symlinks
   immediately (TOCTOU defence).
2. Bounded read (≤64 KB).
3. Re-runs the safety scanner on the freshly-read content.
4. `rename`s the file into `workspace/skills/deploy/deploy.md`.

After promotion, the next call to `SkillIndex::rebuild()` will pick
the new file up.

### Hygiene

Runs at startup as part of `run_hygiene`. Pruning policy:

| Skill state | Action |
|-------------|--------|
| Used at least once (`use_count >= 1`) | Always kept |
| Created < 30 days ago | Always kept |
| Created ≥ 30 days ago AND `use_count = 0` AND never accessed | Dropped from index |

Dropping from the index does not delete the disk file — the file
re-appears in the index on the next `rebuild()` if it's still
there.

### Operational caveats

- **No registered Tool wrapper yet.** The `propose_skill` and
  `promote_staged_skill` helpers ship as library functions, not as
  an agent-callable tool. The agent cannot create or promote skills
  on its own without operator action. The Tool wrapper is the next
  step on the design-doc roadmap.
- **`SkillsLoader` does not yet consult the index.** The index
  exists and stores embeddings; the existing `SkillsLoader` still
  uses keyword/hint matching to choose which skills to inject into
  the system prompt. Wiring `top_k_for_query` into the loader is
  also queued.
- **The `embeddings` feature must be enabled** for indexing to do
  anything. With the default oxicrab build it is.

### Schema

```sql
CREATE TABLE skills_index (
    path             TEXT PRIMARY KEY,    -- absolute path on disk
    name             TEXT NOT NULL,
    description      TEXT NOT NULL,        -- what was embedded
    embedding        BLOB NOT NULL,        -- f32 little-endian
    file_sha256      TEXT NOT NULL,        -- triggers re-embed on change
    use_count        INTEGER NOT NULL DEFAULT 0,
    last_used_ms     INTEGER,
    created_at_ms    INTEGER NOT NULL,
    last_indexed_ms  INTEGER NOT NULL
);
CREATE INDEX idx_skills_index_name ON skills_index(name);
CREATE INDEX idx_skills_index_use ON skills_index(use_count, last_used_ms);
```

Migration #8 in `crates/oxicrab-memory/src/memory_db/migrations/mod.rs`
creates this alongside `tool_reflections`.

### Metrics

- `oxicrab_skill_index_blob_corrupt_total` — incremented when a stored
  embedding BLOB is malformed (non-multiple-of-4 length). The entry is
  skipped for the current query and surfaced at `warn` level.
- `oxicrab_skill_index_dim_mismatch_total` — incremented per skill
  whose embedding dimensionality differs from the query embedding.
  Typical cause: the embedding model was changed without rebuilding
  the index. Solution: call `SkillIndex::rebuild` again.

## Combined: a typical session

1. Operator turns on `agents.defaults.reflection` for one channel.
2. Operator runs `SkillIndex::rebuild()` once after dropping a
   handful of new skill files into `workspace/skills/`.
3. The agent runs normally. When a tool fails, a `<reflection>`
   block is appended to the result; the next LLM turn produces a
   different attempt.
4. After a week, operator queries `tool_reflections` to see what
   the agent has been struggling with and whether the retry
   strategy worked. They might:
   - Rewrite a tool's error message to be self-explanatory.
   - Add a new skill describing the failing pattern.
   - Disable reflection for a specific tool by adding an
     allowlist (future work — currently it's all-or-nothing).
5. Next startup, `run_hygiene` drops skills that haven't been used
   in 30 days from the index.

## What's deliberately NOT in scope

- Self-merging code changes. (Track 3.)
- Self-editing of safety-perimeter code. (Explicitly excluded by
  design.)
- Letting the agent edit its own system prompt or tool registry.
- Runtime synthesis of new Rust source files.

See [`docs/designs/self-improvement.md`](self-improvement.md) §6 for
the full set of explicitly-rejected ideas and why.

## Troubleshooting

**Reflections are firing but retries don't improve outcomes.**
Look at the `next_outcome` column in `tool_reflections`. If most
reads `error`, the LLM is hallucinating a fix that doesn't apply.
Either turn reflection off for that tool (work item) or tighten
the tool's error message so the model has more signal to work with.

**`top_k_for_query` returns no skills even though they exist on
disk.** The index hasn't been rebuilt since the files were added.
Call `SkillIndex::rebuild` once.

**`top_k_for_query` returns a `Result<Vec<_>>` with `Err` on
embedding model load.** The `embeddings` feature is disabled or the
embedding service hasn't initialised. Verify the build has the
feature enabled and the model has finished loading
(`MemoryStore::has_embeddings()` must return `true`).

**Promoted skill doesn't show up in retrieval.** Either the index
hasn't been rebuilt, or the embedding dimension differs from the
query embedding (check `oxicrab_skill_index_dim_mismatch_total`).
Re-run `rebuild` after any embedding model change.

**`promote_staged_skill` returns "is a symlink".** The staged
directory was tampered with between propose and promote. Re-propose
the skill from a clean source — never accept symlinks here.

## Source pointers

- `src/agent/loop/reflection.rs` — reflection module + budget +
  parser.
- `src/agent/loop/iteration.rs::augment_results_with_reflection`
  — agent-loop hook.
- `src/agent/skills/index.rs` — `SkillIndex`, `ScoredSkill`,
  rebuild/top_k/prune.
- `src/agent/skills/propose.rs` — propose / promote / list_staged.
- `crates/oxicrab-memory/src/memory_db/tool_reflections.rs` —
  insert / outcome / count helpers.
- `crates/oxicrab-memory/src/memory_db/skills_index.rs` —
  upsert / list / record_use / prune helpers.
- `crates/oxicrab-memory/src/hygiene/mod.rs` — startup hygiene.
- `tests/reflection_persistence.rs` — DB contract tests.
