# Design: Self-Improvement for Oxicrab

Plan for four parallel tracks of self-development capability, ranked cheapest
and safest first. Based on the 2026-04-16 research synthesis across SICA
(arXiv 2504.15228), Darwin Gödel Machine (arXiv 2505.22954), Voyager
(arXiv 2305.16291), Reflexion (arXiv 2303.11366), and METR's reward-hacking
corpus.

## Non-goals

- Full SICA/DGM-style recursive self-editing loop (documented evaluator
  sabotage; blast-radius too large for a production multi-channel agent).
- Agent-initiated merges to main.
- Runtime synthesis of new Rust source files.
- Self-editing of the leak detector, approval config, sandbox rules, or
  any other safety-perimeter code.

---

## Track 1: Reflexion-style failure reflection

Attach a compact verbal-critique step to the existing agent loop so tool
failures feed back into the next iteration as structured reflections, not
just error strings.

### Current state

- `ToolRegistry::inject_schema_hint()` already appends tool description +
  schema on `ToolResult::is_error = true`.
- Tool results flow into the next iteration's messages verbatim.
- No explicit reflection step; the LLM either self-corrects from the raw
  error or doesn't.

### Proposal

Add an optional reflection turn when a tool call errors. Keep it bounded
(at most one reflection per tool call; max 2 chained reflections per
request) so cost stays predictable.

```rust
// src/agent/loop/reflection.rs (new)
pub(super) struct ToolFailureReflection {
    pub tool: String,
    pub action: Option<String>,
    pub attempt_number: u8,
    pub error_excerpt: String,      // capped 500 chars
    pub hypothesis: String,         // LLM-generated, ≤200 chars
    pub retry_strategy: String,     // LLM-generated, ≤200 chars
}
```

Wire into `AgentLoop::execute_tools()`:

1. Detect `is_error` on a `ToolResult`.
2. If `AgentLoopConfig.reflection.enabled` and attempts < cap, invoke the
   LLM with a tiny reflection prompt (`~200 tokens in, ~100 out`, fixed
   temperature 0.2).
3. Inject the reflection back into the next iteration's tool result
   wrapper with a `<reflection>…</reflection>` marker.
4. Log reflections to `memory_db` under a new `tool_reflections` table
   for offline analysis (what errors recur? which tools are confusing?).

### Config

```toml
[agents.defaults.reflection]
enabled = false       # opt-in while gathering data
maxPerRequest = 2
maxPerTool = 1
temperature = 0.2
storePath = "daily:reflections"
```

### Metrics

- `oxicrab_reflection_triggered_total{tool,action}`
- `oxicrab_reflection_token_cost_seconds`
- Per-reflection: did the *next* tool call succeed? (simple outcome
  counter, paired by request id)

### Risk surface

- **Cost inflation.** Bounded by `maxPerRequest` and a token budget.
- **False-success reflections.** LLM claims the retry will work, it
  doesn't, repeat. Capped by `maxPerTool`.
- **Sensitive content in reflections.** Reflections run through the
  existing `LeakDetector::redact()` before persisting.

### Tests

- `audit_loop_reflection_triggers_on_error`: tool returns error, next
  call includes `<reflection>` block.
- `audit_loop_reflection_respects_caps`: ≥3 consecutive errors trigger
  only 2 reflections; the 3rd tool call runs without a reflection.
- `audit_loop_reflection_redaction`: a tool error containing `sk-...` is
  redacted before entering the reflection prompt.

### Rollout

1. Land opt-in with `enabled = false` default.
2. Dogfood with a specific Slack channel for 2 weeks.
3. Review stored reflections; prune tools where reflections never help.
4. Flip default to enabled if the success-on-retry rate is > 50%.

---

## Track 2: Voyager-style skill library

Oxicrab already persists per-collection skill files to
`~/.oxicrab/skills/collection_{name}/`. That infrastructure is
underexploited. Extend it into a general skill library with
embedding-indexed retrieval, approval-gated additions, usage counters,
and weekly hygiene.

### Current state

- `SkillManager::load_skills_for_context()` loads skills matching the
  channel/context.
- `scan_skill()` (in `src/agent/skills/scanner/`) vets skill content for
  injection / secret-exfiltration patterns before injection.
- No embedding index, no usage tracking, no pruning.

### Proposal

Three additions on top of the existing skills store:

#### 2a. Embedding-indexed retrieval

Store an embedding per skill description in a new `skills_index` table
keyed by skill filename.  Query-time: cosine similarity against the
embedding of the current user turn, return top-k matching skills, include
in system prompt.

```sql
CREATE TABLE IF NOT EXISTS skills_index (
    path            TEXT PRIMARY KEY,
    description     TEXT NOT NULL,
    embedding       BLOB NOT NULL,
    file_sha256     TEXT NOT NULL,       -- invalidate on content change
    created_at_ms   INTEGER NOT NULL,
    last_indexed_ms INTEGER NOT NULL
);
```

Reuse the existing `EmbeddingService`. Rebuild lazily: on skill load,
compare `file_sha256` to disk; re-embed if changed.

#### 2b. Approval-gated `skill_propose` tool

New tool allowing the agent to **propose** a new skill file. Writes to a
staging directory, not the active skills directory. Requires operator
approval via the existing `ApprovalConfig` workflow.

```rust
// src/agent/tools/skill_propose/mod.rs (new)
impl Tool for SkillProposeTool {
    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            subagent_access: SubagentAccess::Denied,
            actions: vec![
                ActionDescriptor { name: "propose", read_only: false },
                ActionDescriptor { name: "list_staged", read_only: true },
            ],
            concurrency: ToolConcurrency::Exclusive,
            ..Default::default()
        }
    }
    // `propose` writes to ~/.oxicrab/skills/staged/<slug>.md
    // Always goes through approval_config.covers() which is forced-true
    // via APPROVAL_REQUIRED metadata (same pattern as webhook dispatch).
}
```

Staged skills do **not** load into the system prompt. An operator approves
a staged skill via a channel button, which calls a `skill_promote` action
that moves the file into the active directory (and re-runs `scan_skill`
on promotion).

#### 2c. Usage counters + weekly hygiene

Extend `skills_index` with `use_count` and `last_used_ms`. Bump on every
system-prompt injection. Weekly cron job:

- Drops skills with `use_count == 0` and `created_at_ms < 30 days ago`.
- Flags skills whose pairing with tool-call errors is statistically high
  (dispatched via existing trace store).
- Re-scans all skills against the current `scan_skill` patterns in case
  the allowlist tightened.

### Config

```toml
[agents.defaults.skills]
enabled = true
indexingEnabled = true       # off disables embedding lookup
maxSystemPromptSkills = 5
maxSkillBytes = 8000
hygieneIntervalHours = 168   # weekly
pruneUnusedDays = 30

[agents.defaults.skills.propose]
enabled = false              # opt-in
requireApproval = true       # never bypass
stagedDir = "~/.oxicrab/skills/staged"
```

### Risk surface

- **Prompt-injection via skill content.** Existing `scan_skill` catches
  known patterns. Re-scan on promotion closes the window.
- **Skill explosion.** Per-channel cap + 30-day unused purge + operator
  visibility via a `skill_list` tool action.
- **Embedding pollution.** Use `embed_texts` for documents (uncached),
  not `embed_query`, to match the Track-3 memory lesson.

### Tests

- `audit_skills_index_rebuilds_on_content_change`
- `audit_skills_propose_requires_approval`
- `audit_skills_promote_rescans_scanner`
- `audit_skills_hygiene_prunes_unused`
- `audit_skills_retrieval_top_k_respects_cap`

### Rollout

1. Land 2a (indexing) first. No user-visible change except better
   retrieval.
2. Land 2c (counters + hygiene) after 2 weeks of 2a data.
3. Land 2b (propose) as opt-in. Start with one operator channel.

---

## Track 3: Propose-PR-only autonomous maintainer

A cron job that acts as a permanent code-review / maintenance agent,
producing PRs but **never merging**. Zero self-deploy, zero write to
`~/.oxicrab/`, all output is human-reviewable.

### Current state

- `CronService` handles scheduled agent turns today.
- `github` tool supports list/create PRs and comments.
- Shell tool runs inside Landlock + bubblewrap sandbox.
- `jj git init --colocate` is already a pattern for worktree isolation.

### Proposal

A dedicated cron job with a narrow `SelfEditing` capability profile. Runs
every 24 hours. Workflow:

1. **Fetch**: `git fetch origin`. Inspect open dependabot PRs, failing
   CI runs on main, and clippy warnings from nightly build.
2. **Investigate**: for each task, open a fresh worktree
   (`$HOME/.oxicrab-worktrees/maintainer-<date>`). Run `cargo check`,
   `cargo clippy --all-targets -- -D warnings`, `cargo test --lib`, and
   the property-security tests.
3. **Draft**: if a task is within scope (clippy fix, dep bump, test
   flake), produce a minimal diff and a PR description.
4. **Propose**: open a draft PR on GitHub with the `maintainer-bot` label.
   Never mark ready-for-review, never merge.
5. **Report**: post a summary of the run to a dedicated Slack channel.

### New capability: `SelfEditing` subagent access

```rust
// crates/oxicrab-core/src/tools/base/capabilities.rs
pub enum SubagentAccess {
    Denied,
    ReadOnly,
    Full,
    /// Can edit files inside a dedicated worktree, can run cargo/git,
    /// cannot touch ~/.oxicrab/, cannot call channel-send tools, cannot
    /// merge PRs. Enforced by a capability filter, not by trust.
    SelfEditing,
}
```

Concrete constraints enforced by the filter:

- Allowed tools: `read_file`, `write_file` (scoped to worktree), `shell`
  (allowlist: `cargo`, `git`, `jj`, `rg`, `fd`, `cat`, `ls`, `grep`,
  `diff`), `github` (actions: `list_issues`, `get_issue`, `list_prs`,
  `get_pr`, `get_pr_files`, `create_pr_review` — no merge, no write to
  default branch).
- Blocked tools: all channel-send tools, `memory_search delete`,
  `cron add`, `skill_propose`, `add_buttons`, `tool_search`.
- Workspace root restricted to `$HOME/.oxicrab-worktrees/maintainer-*`
  via Landlock rules.
- Network: blocked except `api.github.com`, `index.crates.io`,
  `crates.io`, the configured LLM provider endpoints.

### Config

```toml
[agents.maintainer]
enabled = false                                       # explicit opt-in
cronSchedule = "0 3 * * *"                            # 3am UTC daily
worktreeRoot = "~/.oxicrab-worktrees"
maxWorktreeAgeDays = 7
scope = ["clippy", "dependabot", "ci-regressions"]
allowedShellCommands = ["cargo", "git", "jj", "rg", "fd", "cat", "ls", "grep", "diff"]
blockedPatterns = ["--no-verify", "rm -rf", "force-with-lease"]
reportChannel = "slack:C12345"
prLabel = "maintainer-bot"
maxPrPerRun = 3
```

### Metrics

- `oxicrab_maintainer_pr_opened_total{scope}`
- `oxicrab_maintainer_pr_merged_total{scope}` (from webhook; tracks
  human merge of maintainer PRs)
- `oxicrab_maintainer_run_duration_seconds`
- `oxicrab_maintainer_llm_cost_usd_total` (via existing token log)

### Risk surface

The uncomfortable truth: the maintainer's scope ends at "opens PR."
Everything downstream is human judgement + existing CI.

Specific mitigations:

- **Force-push to someone else's branch.** Blocked: allowed-command list
  excludes `push --force` patterns; `.git/hooks/pre-push` rejects force
  push in the worktree.
- **Sneaking a second commit into an approved PR.** Each run creates a
  fresh worktree; PRs are draft-only; human flips ready-for-review.
- **Running `cargo test` on external crates.** Network allowlist blocks
  everything except crates.io and api.github.com. Shell allowlist
  prevents arbitrary binaries.
- **Self-editing the maintainer config.** Workspace restriction blocks
  writes to `~/.oxicrab/`; the maintainer cannot modify its own
  `agents.maintainer` block. Any change to that block requires a human
  PR into this repo.
- **Token exhaustion.** `maxPrPerRun = 3` and the existing
  `llm_cost_log` daily budget cap.

### Tests

- `audit_maintainer_cannot_write_outside_worktree` (Landlock rule test).
- `audit_maintainer_tool_filter_excludes_channels`.
- `audit_maintainer_shell_rejects_force_push`.
- `audit_maintainer_opens_pr_in_draft`.
- End-to-end integration test: clippy introduces a fixable warning on a
  fixture branch → maintainer produces a PR with the fix → PR is draft →
  human-merges → CI passes.

### Rollout

1. Land the `SelfEditing` capability mode in isolation (no cron yet).
   Tests prove the filter denies all blocked paths.
2. Add the cron callback behind `enabled = false`. Manual trigger only.
3. One trusted operator runs it daily for a month with manual PR review.
4. Collect data: how many PRs landed, how much human review time per
   PR, how many were incorrect.
5. If net positive, flip to `cronSchedule = "0 3 * * *"` with an
   operator on-call for the first week.

---

## Track 4: Harvest the 18 SICA/DGM observed modifications

Instead of running the self-improvement loop, read SICA and DGM's logs
of what the loop discovered and cherry-pick the ideas. Each item below
gives paper provenance, the problem it solved, how it maps onto
oxicrab, and a pros/cons verdict.

**Summary table** (details below):

| # | Source | Modification | Verdict |
|---|--------|--------------|---------|
| 1 | SICA Fig 3 | Smart Edit Tool (diff/range edits) | **Already have it** (`write_file` + shell `git apply`); small refinement possible |
| 2 | SICA Fig 3 | Code Context Summarization | **Already have it** (compaction + context builder) |
| 3 | SICA Fig 3 | File Edit Verification | **Adopt** (cheap post-edit check before tool result returns) |
| 4 | SICA Fig 3 | AST Symbol Locator | **Adopt** for Rust (tree-sitter) as new tool |
| 5 | SICA Fig 3 | Hybrid Symbol Locator | **Adopt** bundled with #4 |
| 6 | SICA Fig 4 | Independent Math Verifier | **Skip** (not a general-purpose need) |
| 7 | SICA Fig 4 | SymPy Symbolic Calculator | **Skip** (narrow; already have `shell` + python) |
| 8 | SICA Fig 4 | Math Cross-Validator | **Skip** (paper says it may have regressed) |
| 9 | SICA Fig 4 | Geometry Specialist | **Skip** (domain-specific) |
| 10 | SICA Fig 4 | Systematic Math Reasoning | **Skip** (paper says it may have regressed) |
| 11 | DGM Fig 3 | Non-empty Patch Validation + Retry | **Adopt** (maps onto tool-result guard) |
| 12 | DGM Fig 3 | Granular File Viewing by Lines | **Already have it** (`read_file` with offset/limit) |
| 13 | DGM Fig 3 | String-replacement Editing | **Already have it** (`Edit` tool semantics) |
| 14 | DGM Fig 3 | Auto-summarise on Context Limit | **Already have it** (compaction) + minor refinement |
| 15 | DGM Fig 3 | Multiple Patch Generations + Ranking | **Adopt** (fits the existing wave-based executor) |
| 16 | DGM Fig 3 | History-aware Patch Generation | **Adopt** (pairs with Track 1 reflection) |
| 17 | DGM App H | Tool Transaction Logging / Markers | **Adopt** (audit defence, not just improvement) |
| 18 | DGM App H | Hallucination Stripping | **Adopt** (extends the prompt guard) |

### 1. Smart Edit Tool (SICA, Figure 3, iter ~3)

**What**: Replace whole-file overwrite with diff/range editing.

**Problem solved**: Base SICA agent could only overwrite entire files;
slow, destructive, token-expensive.

**Oxicrab status**: Already have it. The `Edit` tool uses string-match
replacement; `Write` requires `Read` first. The CLAUDE.md "prefer edit
over write" guidance matches this.

**Refinement**: Add an `Edit` variant that accepts unified diff format
(`patch_apply`), reusing the shell sandbox for `git apply --cached
--check` before writing. Helps the LLM commit multi-hunk edits in one
call.

**Pros**: Small new surface, bounded risk. Matches how dependabot /
git-hosted review workflows think about edits.

**Cons**: Yet another edit-shaped tool; LLM may get confused about
which to use.

**Verdict**: Optional. Land only if operators request it after Track 3
is live.

---

### 2. Code Context Summarisation (SICA, Figure 3, early iters)

**What**: Summarise accumulated tool-call traces so the working window
stays focused.

**Problem solved**: Long tool-call traces were crowding out signal.

**Oxicrab status**: Already have it. `MessageCompactor`, pre-flush
extraction (`compaction_config.pre_flush_enabled`), and the context
builder all serve this. Reasoning content is preserved through the
post-loop summary path (audit fix #11).

**Verdict**: No action.

---

### 3. File Edit Verification (SICA, Figure 3, iter ~5)

**What**: Post-edit verification step checks the new file content
against the intended change before the tool returns success.

**Problem solved**: Silent bad edits (e.g. string-replace succeeded but
produced syntactically broken code) cost benchmark points.

**Oxicrab status**: Not currently verified. `Edit` returns success as
soon as the string replacement succeeds. Syntax / cargo-check signal is
only available if the LLM manually runs `cargo check` afterwards.

**Proposal**: Add an optional verification hook to `Edit` and `Write`:
if the target extension is `.rs`, run `rustc --emit=metadata -Zparse-only`
(or `syn` via a small helper) inside the sandbox, attach
`verification_ok`/`verification_warnings` to the tool result metadata.

**Pros**: Catches syntax regressions before they waste further LLM
turns. Very cheap (syn parse is fast).

**Cons**: Extra dep (`syn` in a helper) or extra shell call. On large
files the parse can take 10-50ms. Extension hard-coding (what about
`.toml`, `.md`?) — likely start with Rust only.

**Verdict**: **Adopt**. Start Rust-only. Land as new `verify_after_edit`
middleware in the tool registry, opt-in via `tools.edit.verifyRust`.

---

### 4. AST Symbol Locator (SICA, Figure 3, iter 9)

**What**: Parse the target file's AST and return line/col ranges for
named symbols.

**Problem solved**: grep-based search returned too many false positives
on large files; the LLM wasted turns triaging matches.

**Oxicrab status**: Not present. The codebase has `rg` / `ast-grep` on
PATH but no registered tool. Symbol navigation happens through shell
`grep` or via Read + eyeballing.

**Proposal**: New tool `code_symbol` with actions:
- `find` — return file:line:col of symbol definition by name
- `references` — list call sites / references
- `context` — return the ±N lines around each match

Implementation: wrap `tree-sitter-rust` (add dep) or shell out to
`ast-grep`. Start with Rust only; extensible later.

**Pros**: SICA's biggest single lift on SWE-bench came from here
(symbol-navigation benchmark 0.33 → 0.40; SWE-bench 0.38 → 0.47 same
iteration). Maps perfectly onto Rust via tree-sitter.

**Cons**: New dep; maintenance overhead. Tree-sitter is big (~2MB
compiled). For a 14-crate workspace the per-call latency is low (single
file parse) but operators who use oxicrab on small projects pay a
static binary-size cost.

**Verdict**: **Adopt**, Rust-only, as a first-class tool. Highest
expected ROI of any item in this list.

---

### 5. Hybrid Symbol Locator (SICA, Figure 3, later iter)

**What**: Fall back to text/grep when AST parse fails or the symbol is
from an uninstalled crate.

**Problem solved**: AST-only fails on external libraries, generated code,
or parse errors.

**Oxicrab status**: Folds into #4.

**Verdict**: **Adopt** bundled with #4. `code_symbol` should degrade to
`rg --type rust` if tree-sitter returns no hits.

---

### 6. Independent Math Verifier (SICA, Figure 4)

**What**: Sub-agent that re-derives math answers to check the primary
reasoner.

**Oxicrab relevance**: Low. Oxicrab users don't do AIME problems.
Cross-check of arithmetic already available via shell + python/calc.

**Verdict**: **Skip**.

---

### 7. SymPy Symbolic Calculator (SICA, Figure 4)

**What**: Tool wrapping SymPy for exact symbolic math.

**Oxicrab relevance**: Narrow. The shell tool already allows
`python3 -c 'import sympy; ...'` if python is on PATH.

**Verdict**: **Skip**. Can be added as an operator-opt-in skill later.

---

### 8-10. Math Cross-Validator, Geometry Specialist, Systematic Math Reasoning (SICA, Figure 4)

**Oxicrab relevance**: SICA explicitly says these **may have regressed**
performance because they interrupted the reasoning-model's chain of
thought. Not generalisable.

**Verdict**: **Skip all three**.

---

### 11. Non-empty Patch Validation and Retry (DGM, Figure 3)

**What**: Reject empty diffs, diffs that only touch test files, or
syntactically invalid patches. Re-run the agent with a specific error
message.

**Problem solved**: Base DGM frequently produced empty patches or
modified only tests — automatic zero on SWE-bench.

**Oxicrab status**: Not present. `Edit` tool returns success on
zero-byte replacements; no check that any file change actually
happened.

**Proposal**: Add a new `ToolMiddleware` hook (`after_execute`) that
compares pre/post file hashes for `Edit` / `Write`. If no change, wrap
the result with a targeted error and hint. Pairs cleanly with Track 1
reflection.

**Pros**: Cheap. Catches a common LLM failure mode where the model
claims to have edited but the string didn't match.

**Cons**: Slight complication of the tool-result contract (success with
no-change is now explicitly flagged).

**Verdict**: **Adopt**. Add as `EditNoOpGuard` middleware.

---

### 12. Granular File Viewing by Lines (DGM, Figure 3)

**What**: `view_range` parameter to read a line range of a large file.

**Oxicrab status**: Already have it. `Read` accepts `offset` and `limit`
parameters (see `read_file` tool).

**Verdict**: No action.

---

### 13. String-replacement Editing (DGM, Figure 3, node 24 = 40.5%)

**What**: `old_str`/`new_str` substring replacement with required
unique match.

**Oxicrab status**: Already have it. `Edit` tool enforces unique match
(`replace_all` is opt-in explicit).

**Verdict**: No action.

---

### 14. Auto-summarise on Context Limit (DGM, Figure 3)

**What**: When an LLM call hits the context ceiling, summarise the
message history and retry.

**Oxicrab status**: Already have it. The pre-flight token estimator
(audit fix #13, now byte-based) trims oldest messages before the call;
the compaction system summarises on the next turn.

**Refinement**: Mid-call recovery. Today, a 413 `context_length_exceeded`
response from the provider returns an error to the caller. DGM's
improvement is to catch that error, summarise, and retry within the
same request ID.

**Pros**: Would eliminate a class of "LLM call failed but data lost"
errors.

**Cons**: Small; the pre-flight estimator already prevents most of
these. More complexity in the retry path.

**Verdict**: **Adopt as minor refinement**. Add a `ProviderError::
ContextLengthExceeded` variant and a one-shot retry in
`ModelGateway::invoke_with_retry`.

---

### 15. Multiple Patch Generations and Ranking (DGM, Figure 3, Polyglot best)

**What**: Generate N candidate patches, run tests on each, keep the one
whose tests pass.

**Problem solved**: Single-shot patch generation is high-variance on
hard problems; one bad patch = 0.

**Oxicrab relevance**: Medium. For Track-3 maintainer scope (clippy
fixes, dependabot), the problems are narrow and a single attempt
usually suffices. For deeper code-editing tasks (hypothetical Track 5),
multi-attempt would be useful.

**Proposal (long-horizon)**: Add a `cargo run` / `cargo test` wrapper
that generates up to 3 candidate patches (different prompts, different
temperatures), runs tests against each in parallel worktrees, keeps
the highest-scoring one. Pairs with Track 3; gated behind a new
`maintainer.multiAttempt` flag.

**Pros**: DGM's single biggest Polyglot delta (14.2 → 30.7%) came from
this.

**Cons**: 3× LLM cost per task; 3× shell invocation; concurrent
worktree management.

**Verdict**: **Adopt later**, after Track 3 has data on single-attempt
success rate. Skip initially.

---

### 16. History-aware Patch Generation and Ranking (DGM, Figure 3)

**What**: When re-generating, include which prior attempts failed and
their error messages in the next instruction.

**Oxicrab status**: Not present for patch-level retry; tool-level retry
today sends only the raw error.

**Proposal**: Pairs with Track 1 (reflection). When a tool fails and a
reflection is produced, include a terse summary of all prior failed
attempts' hypotheses in the next LLM prompt.

**Pros**: Cheap. Reuses reflection infrastructure.

**Cons**: Prompt growth. Cap at N=3 prior attempts.

**Verdict**: **Adopt as part of Track 1**. Not a separate effort.

---

### 17. Tool Transaction Logging with Markers (DGM, Appendix H)

**What**: Wrap every real tool call with a special marker in the log
so hallucinated tool use (plain-text imitation) can be detected. The
agent claiming "Tool Used: bash" without calling the tool becomes
visible.

**Problem solved**: Long-context models hallucinate tool calls as
free-form text.

**Oxicrab status**: Partially present. `contains_action_claims()` and
`ACTION_CLAIM_PATTERNS` exist as regex-based detection for false
action claims; the agent loop no longer acts on them (per audit notes)
because the false-positive rate was too high.

**Proposal**: A targeted variant: when an assistant message contains a
pattern like `<tool_use>…</tool_use>` or `Tool Result:` *without* a
matching structured tool call entry in the message, log a warning and
redact the claim from the message before it enters the next LLM call.
This is the DGM failure case — the agent deleted the marker-injection
code to defeat the detector. Oxicrab's version must make the detector
**unreachable** from the agent's edit capability (enforce via
capability filter, not trust).

**Pros**: Catches a real hallucination mode; unlike the removed
hallucination detector, this one is passive (doesn't intercept normal
flow, only flags-and-redacts).

**Cons**: False positives if a user literally writes `<tool_use>` in a
conversation. Mitigation: only apply in assistant messages, not user
messages.

**Verdict**: **Adopt**. Important as a defence property regardless of
whether Track 3 ever ships.

---

### 18. Hallucination Stripping from Output (DGM, Appendix H)

**What**: Detect hallucinated tool-use blocks in the model output and
strip them before forwarding the response to the user.

**Problem solved**: Even with detection, hallucinated text polluted
downstream context and confused users.

**Oxicrab status**: Not present. `strip_think_tags()` exists for
thinking blocks but not for fake tool-use claims.

**Proposal**: New post-processing step in
`iteration.rs::handle_text_response`: strip `<tool_use>…</tool_use>`
blocks that don't correspond to real tool calls, replace with a
`[hallucinated tool claim removed]` marker. Emit a counter
(`oxicrab_hallucination_strip_total`).

**Pros**: Surface-level polish; prevents users from seeing fake "I ran
this" claims.

**Cons**: Same false-positive risk as #17. Probably safe because users
rarely type `<tool_use>`.

**Verdict**: **Adopt**, alongside #17.

---

## Rollout ordering

Absolute order based on cost × risk × expected value:

1. **Track 1 (Reflexion)** — weeks 1-2. Smallest footprint, measurable.
2. **Harvest items 17 + 18** — week 3. Defensive hardening against the
   DGM-observed hallucination mode.
3. **Harvest items 3, 11, 14** — weeks 4-5. Edit verification, no-op
   guard, context-limit retry. Low-risk middleware additions.
4. **Harvest items 4 + 5** — weeks 5-6. AST symbol locator. Highest ROI
   new tool.
5. **Track 2 2a (skill indexing)** — weeks 6-7.
6. **Track 2 2c (hygiene)** — week 8.
7. **Track 2 2b (propose)** — month 3. After a month of indexing data.
8. **Track 3 (maintainer)** — months 3-4. Requires the `SelfEditing`
   capability mode and full sandbox test coverage.
9. **Harvest item 15 (multi-attempt)** — month 4+. Only if Track 3
   single-attempt data shows we need it.

Items marked "already have it" (1, 2, 12, 13) are no-ops. Items 6-10
are explicit skips.

## Success criteria

- Track 1: ≥ 50% of reflection-triggered retries succeed on the next
  tool call over 30 days.
- Track 2: operators promote ≥ 1 staged skill per week. Skills have
  non-zero use-counts after 30 days.
- Track 3: maintainer PRs have < 20% revert rate, < 5 min human review
  time per PR.
- Track 4 items: each adopted item gets its own audit-style regression
  test and measurable outcome (e.g. "verification catches X broken
  edits over 30 days").

## What would make us stop

- Reflection retries succeed < 25% (kill Track 1).
- Promoted skills have < 10% reuse rate (freeze Track 2b).
- Maintainer PRs have > 40% revert rate (disable Track 3).
- Any item introduces a new security finding in the property-security
  test suite (revert the item).

## Open questions

1. Where does the `SelfEditing` capability live? A standalone crate, or
   an enum variant on existing `SubagentAccess`? Leaning enum variant
   to keep the capability machinery in one place.
2. Should the maintainer target `main` or a dedicated
   `maintainer-proposals` branch? `main` is simpler; dedicated branch
   keeps maintainer PRs out of the default PR review queue.
3. Do we want to ship the `SelfEditing` capability before Track 3 is
   ready? It's useful for other things (e.g. a `code_review` subagent).
   Probably yes, ship it with #17/#18.

## References

- Robeyns et al., "A Self-Improving Coding Agent" (SICA), arXiv
  [2504.15228](https://arxiv.org/abs/2504.15228)
- Zhang et al., "Darwin Gödel Machine" (DGM), arXiv
  [2505.22954](https://arxiv.org/abs/2505.22954)
- Wang et al., "Voyager: An Open-Ended Embodied Agent", arXiv
  [2305.16291](https://arxiv.org/abs/2305.16291)
- Shinn et al., "Reflexion: Language Agents with Verbal RL", arXiv
  [2303.11366](https://arxiv.org/abs/2303.11366)
- METR, "Recent Frontier Models Are Reward Hacking" (June 2025),
  https://metr.org/blog/2025-06-05-recent-reward-hacking/
- Hubinger et al., "Risks from Learned Optimization", arXiv
  [1906.01820](https://arxiv.org/abs/1906.01820)
