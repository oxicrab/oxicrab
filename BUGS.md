# Bug Report — Oxicrab Codebase Review

This document records bugs and issues identified during a codebase review. Items are
grouped by severity. No fixes are applied here; this is a reference document only.

---

## High Severity

### 1. `merge_pr()` Bypasses Standard GitHub API Request Helpers

**File:** `crates/oxicrab-tools-api/src/github/mod.rs`  
**Lines:** 323–335

`merge_pr()` builds its HTTP request manually instead of routing through
`github_headers()` and `api_send()`, which every other API method uses. This causes
four divergences from the rest of the GitHub tool:

| Property | All other methods | `merge_pr()` |
|---|---|---|
| Auth header | `Bearer {token}` (`github_headers()`) | `token {token}` (line 328 as of review) — legacy format |
| `X-GitHub-Api-Version` header | ✓ (`github_headers()`) | ✗ |
| Per-request timeout | 15 s (`github_headers()`) | ✗ |
| Rate-limit check | `check_rate_limit()` (`api_send()`) | ✗ |

GitHub's fine-grained personal access tokens (the current recommended type) require the
`Bearer` scheme. With the legacy `token` scheme those tokens may be rejected, meaning
`merge_pr` can fail while every other action succeeds.

```rust
// line 328 — should use github_headers() like all other methods
.header("Authorization", format!("token {}", self.token))
```

---

### 2. Cron Job Fires Even When `fire_cron_job()` DB Update Fails

**File:** `src/cron/service/mod.rs`  
**Lines:** 308–328

`fire_cron_job()` is the call that atomically increments `run_count`, updates
`next_run`, and transitions the job to `running` state. When this call fails (DB
unavailable, lock contention, etc.) the code logs a warning but unconditionally
appends the job to `jobs_to_fire` and runs it anyway.

```rust
let res = tokio::task::spawn_blocking(move || {
    db.fire_cron_job(&id, effective_next, now, now) // DB update
})
.await
.unwrap_or_else(|e| { warn!("cron: spawn_blocking failed: {e}"); Ok(false) });

if let Err(e) = res {
    warn!("failed to fire cron job '{}': {}", job.id, e); // warning only
}

// Job is always added here, regardless of the DB result above
if let Some(ref callback) = callback_opt {
    jobs_to_fire.push((job.clone(), callback.clone()));
}
```

Consequences:

- The job's `run_count` is not incremented, so `max_runs` enforcement is broken.
- The job's status never transitions to `running`, so the scheduler may fire the same
  job again on the next tick.
- The `next_run` column is not advanced, causing repeated duplicate executions until
  the DB recovers.

---

## Medium Severity

### 3. `check_file_permissions()` Only Checks the First Config File

**File:** `src/config/loader/mod.rs`  
**Lines:** 51–55, 218–248

`check_file_permissions()` uses `static WARNED: Once` internally. Because `Once`
executes its closure exactly once per process, calling the function in a loop over
multiple config-layer paths means only the **first** file's permissions are ever
inspected. Any subsequent layer files (`.local.toml` overlays, environment-specific
overrides, etc.) have their permissions silently skipped.

```rust
// Caller (lines 51-55) — iterates over multiple paths
for path in &layer_paths {
    if path.exists() {
        check_file_permissions(path);  // Only the first call does anything
    }
}

// Inside the function (line 222-223) — runs the check only once total
static WARNED: Once = Once::new();
WARNED.call_once(|| {
    // The path captured here is only the first layer's path
});
```

A user whose primary `config.toml` has mode `0600` but whose `.local.toml` has mode
`0644` will never receive a warning about the world-readable overlay.

---

### 4. Circuit-Breaker `HalfOpen` Probe Limit Uses Stale Counter

**File:** `crates/oxicrab-providers/src/circuit_breaker/mod.rs`  
**Lines:** 116–126

In the `HalfOpen` state, the gate condition is:

```rust
if breaker.active_probes + successes >= self.config.half_open_probes {
    // reject new probe
}
```

`successes` counts completed successful probes. `active_probes` counts in-flight ones.
When `record_success()` is called, `active_probes` is decremented _before_ `successes`
is incremented, so their sum is always `<= total_probes`. The intent appears to be
"limit concurrent in-flight probes to `half_open_probes`", but the `+ successes` term
reduces this limit by the number of already-completed probes. As successes accumulate,
the circuit breaker increasingly rejects new probes even when there are far fewer
concurrent in-flight probes than the configured limit. In the worst case, after
`half_open_probes - 1` successes, no further probes are ever allowed in the same
`HalfOpen` cycle (unless a failure resets `successes`), making the transition to
`Closed` impossible for the last required probe.

---

### 5. `canonicalize().unwrap_or(p)` Can Bypass Workspace Checks

**File:** `crates/oxicrab-tools-system/src/shell/mod.rs`  
**Line:** 39

```rust
let working_dir = working_dir.map(|p| p.canonicalize().unwrap_or(p));
```

If `canonicalize()` fails (e.g. the directory doesn't exist yet at tool construction
time, or a symlink target is unreachable), the raw, un-canonicalized path is kept.
Later, `check_paths_in_workspace()` (lines 247–254) resolves relative paths against
this working dir using `canonicalize()` again and checks whether the result starts with
`workspace`. If the working dir stored here is not the canonical form of `workspace`,
the `starts_with` check may produce a false negative or false positive, potentially
allowing or blocking paths incorrectly.

---

### 6. Slack File Upload Target URL Not Domain-Validated

**File:** `crates/oxicrab-channels/src/slack/mod.rs`  
**Line:** 210

```rust
let step2_resp = self.client.post(upload_url).multipart(form).send().await?;
```

`upload_url` comes verbatim from Slack's `files.getUploadURLExternal` API response.
File bytes (which may include sensitive content) are uploaded to this URL without
first verifying it is a `slack.com` subdomain. If a Slack workspace is compromised or
if a MITM attack against the Slack API is possible, an attacker could redirect file
uploads to an arbitrary endpoint.

The `is_slack_domain()` helper used elsewhere in the same file (lines 1037–1047)
correctly validates Slack domains using URL parsing; applying it to `upload_url` would
close the gap.

---

### 7. Discord Attachment URLs Downloaded Without Domain Validation

**File:** `crates/oxicrab-channels/src/discord/mod.rs`  
**Line:** 356

```rust
match self.http_client.get(&attachment.url).send().await {
```

Attachment objects come from the Discord API. Their `url` field is not validated
against a known Discord CDN domain before the download request is sent. If the Discord
API ever returns unexpected URLs (e.g. through a compromised bot token, API bug, or
future API change), the bot will make outbound HTTP requests to arbitrary hosts, which
is a server-side request forgery (SSRF) risk. Validating that the URL's host is under
`cdn.discordapp.com` or `media.discordapp.net` before fetching would mitigate this.

---

## Low Severity

### 8. Dead Code in `SYSTEM_PREFIXES` Loop (Two Copies)

**Files:**  
- `src/utils/path_sanitize/mod.rs`, lines 30–34  
- `crates/oxicrab-tools-system/src/utils/path_sanitize.rs`, lines 25–29  

The loop checking system prefixes contains a condition that is logically impossible:

```rust
// Line 25: early return if path is NOT under home
if !path_str.starts_with(home_str.as_ref()) {
    return path_str.to_string();
}

// Lines 30-34: this branch can never be taken
for prefix in SYSTEM_PREFIXES {
    if path_str.starts_with(prefix) && !path_str.starts_with(home_str.as_ref()) {
        //                              ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
        // Always false: we already returned at line 25 if this were true
        return path_str.to_string();
    }
}
```

The comment says "Check system prefixes first (in case home is under /var or similar)"
but the early-return above defeats this intent. A path under both `/var` and the home
directory (e.g. `home = /var/home/user`) will be treated as a home-directory path and
potentially have its components redacted, even though it is also a system path.

---

### 9. Silent Fallback to Relative Config Path

**File:** `src/config/loader/mod.rs`  
**Line:** 67

```rust
let default_path = get_config_path().unwrap_or_else(|_| PathBuf::from("config.toml"));
```

When `get_config_path()` fails (e.g. home directory not found), the error is silently
swallowed and config is read from/written to a relative `config.toml` in the current
working directory. There is no log message. An operator running the service from an
unexpected working directory may be surprised to find config changes silently applied
to a file in the wrong location.

---

### 10. OpenAI and Gemini Providers Omit Per-Request Timeout on `chat()`

**Files:**  
- `crates/oxicrab-providers/src/openai/mod.rs`, line 301  
- `crates/oxicrab-providers/src/gemini/mod.rs`, line 328  

The Anthropic provider sets an explicit 120-second timeout on the main `chat()` request
(line 136) via `.timeout(Duration::from_secs(PROVIDER_REQUEST_TIMEOUT_SECS))`. The
OpenAI and Gemini providers do not; they rely only on the client-level default set in
`provider_http_client()`. While the client-level default is the same value (120 s),
the inconsistency means that if the client is ever shared or reconfigured, the per-
request safety net is absent for OpenAI and Gemini. Both providers do set the 15-second
timeout on their `warmup()` method, which makes the omission on the main path more
conspicuous.

---

### 11. Known Performance Issue: O(n) Embedding Search with No ANN Index

**File:** `crates/oxicrab-memory/src/memory_db/embeddings.rs`  
**Line:** 75 (acknowledged in comments)

```rust
// TODO: This performs a brute-force linear scan over all embeddings.
// A proper vector index (e.g. `instant-distance` or `usearch` crate)
// would reduce search from O(n) to approximately O(log n).
```

The embedding similarity search iterates over every stored embedding on each query.
As the memory store grows, search latency grows linearly. This is a known, tracked
issue but has no associated fix or mitigation (e.g. a size warning at startup).
Note: the O(log n) claim in the source comment is approximate — typical ANN indexes
like HNSW achieve sub-linear average-case performance but not strict O(log n)
guarantees.

---

## Summary Table

| # | Severity | File | Short Description |
|---|----------|------|-------------------|
| 1 | High | `crates/oxicrab-tools-api/src/github/mod.rs:328` | `merge_pr` uses legacy auth header, missing headers/timeout/rate-limit |
| 2 | High | `src/cron/service/mod.rs:320–328` | Cron job fires even when DB update fails |
| 3 | Medium | `src/config/loader/mod.rs:218–248` | `check_file_permissions` only inspects first config file |
| 4 | Medium | `crates/oxicrab-providers/src/circuit_breaker/mod.rs:118` | HalfOpen probe gate uses stale `successes` counter |
| 5 | Medium | `crates/oxicrab-tools-system/src/shell/mod.rs:39` | `canonicalize().unwrap_or()` can bypass workspace checks |
| 6 | Medium | `crates/oxicrab-channels/src/slack/mod.rs:210` | Slack upload URL not domain-validated before use |
| 7 | Medium | `crates/oxicrab-channels/src/discord/mod.rs:356` | Discord attachment URLs downloaded without domain check |
| 8 | Low | `src/utils/path_sanitize/mod.rs:30–34` (×2) | Dead code in system-prefix loop |
| 9 | Low | `src/config/loader/mod.rs:67` | Silent fallback to relative config path |
| 10 | Low | OpenAI/Gemini providers | `chat()` lacks per-request timeout unlike Anthropic |
| 11 | Low | `crates/oxicrab-memory/src/memory_db/embeddings.rs:75` | O(n) embedding search, no ANN index |
