//! Audit tests for CHANNELS findings.
//!
//! Each test documents the cited behavior with the exact logic from the source.
//! Because the cited internal functions are private to their modules, these
//! tests replicate the relevant comparison/arithmetic to pin down and
//! demonstrate the bug. When fixes land, update the test to match the new
//! intended behavior.

// ============================================================================
// Finding #1: Slack 429 hardcoded 1s fallback
// crates/oxicrab-channels/src/slack/mod.rs:81, 91
// ============================================================================

/// Replicates `classify_slack_error`'s fallback logic.
/// Source: `retry_after.unwrap_or(1)` on both HTTP-429 branch and
/// `ratelimited` error-field branch.
fn slack_retry_fallback(retry_after: Option<u32>) -> u32 {
    retry_after.unwrap_or(1)
}

#[test]
fn audit_channels_01_slack_429_hardcoded_1s_fallback() {
    // When Slack omits Retry-After, the channel retries after 1 second.
    // This creates thundering-herd risk under sustained rate limiting — a
    // safer default would be a randomized or larger fallback (e.g. 5-30s).
    assert_eq!(slack_retry_fallback(None), 1);
    // When the header is present it is honored, so the bug is strictly the
    // fallback value.
    assert_eq!(slack_retry_fallback(Some(30)), 30);
}

// ============================================================================
// Finding #2: Discord token TTL off-by-one
// crates/oxicrab-channels/src/discord/mod.rs:753
// ============================================================================

/// Replicates the token-expired check literal: `now - ts > 14 * 60`.
fn discord_token_expired(now: i64, ts: i64) -> bool {
    now - ts > 14 * 60
}

#[test]
fn audit_channels_02_discord_token_ttl_off_by_one() {
    let ts = 1_000_000_i64;

    // At exactly 14:00 (840s) since issue, token is NOT flagged as expired,
    // even though the 14-minute safety margin should arguably include the
    // boundary. The check should be `>=` (or the threshold should be 13*60
    // for a true 1-minute margin before the 15-minute Discord limit).
    assert!(!discord_token_expired(ts + 14 * 60, ts));

    // 14:01 — flagged as expired, as expected.
    assert!(discord_token_expired(ts + 14 * 60 + 1, ts));
}

// ============================================================================
// Finding #3: Slack thread map unbounded
// crates/oxicrab-channels/src/slack/mod.rs:499-502
// ============================================================================

/// Replicates the thread-map maintenance logic: insert + prune-by-TTL only
/// (no size cap). `SlackChannel::participated_threads` uses the same pattern.
fn prune_threads(
    threads: &mut std::collections::HashMap<String, std::time::Instant>,
    ttl: std::time::Duration,
) {
    let now = std::time::Instant::now();
    if let Some(cutoff) = now.checked_sub(ttl) {
        threads.retain(|_, last| *last > cutoff);
    }
}

#[test]
fn audit_channels_03_slack_thread_map_unbounded() {
    let mut threads: std::collections::HashMap<String, std::time::Instant> =
        std::collections::HashMap::new();
    let now = std::time::Instant::now();
    // Simulate 20_000 active threads all within the 24h TTL window.
    for i in 0..20_000 {
        threads.insert(format!("thread-{i}"), now);
    }
    prune_threads(&mut threads, std::time::Duration::from_secs(86_400));
    // TTL-only pruning leaves every live entry in place — no size bound.
    // Under bursty group traffic this grows without limit.
    assert_eq!(threads.len(), 20_000);
}

// ============================================================================
// Finding #4: Telegram button context >64B silent truncation
// crates/oxicrab-channels/src/telegram/mod.rs:79
// ============================================================================

/// Replicates the lossy-fallback path: when the combined `id|ctx` exceeds
/// Telegram's 64-byte callback_data limit AND no dispatch_store is wired,
/// the string is truncated on a char boundary.
fn telegram_callback_truncate(id: &str, ctx: &str) -> String {
    const CALLBACK_DATA_MAX_BYTES: usize = 64;
    let inline = format!("{id}|{ctx}");
    if inline.len() <= CALLBACK_DATA_MAX_BYTES {
        inline
    } else {
        // floor_char_boundary is unstable in older toolchains — use a
        // deterministic byte-walk here that mirrors the production path.
        let mut cut = CALLBACK_DATA_MAX_BYTES;
        while !inline.is_char_boundary(cut) {
            cut -= 1;
        }
        inline[..cut].to_string()
    }
}

#[test]
fn audit_channels_04_telegram_callback_silent_truncation() {
    let id = "btn";
    let ctx = "x".repeat(200);
    let out = telegram_callback_truncate(id, &ctx);
    // Result is exactly the limit — the rest of the context is silently
    // discarded when no DispatchContextStore is wired. The button click
    // that arrives back will have a mangled callback_data that the LLM must
    // decipher.
    assert_eq!(out.len(), 64);
    assert!(out.starts_with("btn|"));
    // The truncated payload is shorter than the original — data was dropped.
    assert!(out.len() < format!("{id}|{ctx}").len());
}

// ============================================================================
// Finding #7: Slack reaction swap fire-and-forget loses state
// crates/oxicrab-channels/src/slack/mod.rs:968-1004
// ============================================================================

/// Demonstrates that fire-and-forget task results are dropped. If
/// `reactions.remove` silently fails but `reactions.add` succeeds, the user
/// sees BOTH the thinking and the done emoji — a persistent state mismatch
/// that the channel never retries or logs.
#[tokio::test]
async fn audit_channels_07_slack_reaction_fire_and_forget_loses_state() {
    let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));
    let o = observed.clone();
    tokio::spawn(async move {
        // Simulate reactions.remove failing.
        let remove_result: Result<(), &'static str> = Err("remove failed");
        // Production code uses `let _ = client.post(...).send().await;` —
        // so an error is silently dropped.
        let _ = remove_result;
        // Simulate reactions.add succeeding.
        let add_result: Result<(), &'static str> = Ok(());
        if add_result.is_ok() {
            o.lock().unwrap().push("done_added");
        }
    })
    .await
    .unwrap();

    let log = observed.lock().unwrap();
    // The done reaction was added but we have no record of whether the
    // thinking reaction was successfully removed — from the caller's
    // perspective the result is unobservable.
    assert_eq!(log.as_slice(), &["done_added"]);
}

// ============================================================================
// Finding #8: WhatsApp queue drops old without logging — FALSE POSITIVE
// crates/oxicrab-channels/src/whatsapp/mod.rs:510-512
// ============================================================================
// Source shows `warn!("whatsapp: message queue full (1000), dropping oldest message");`
// at line 511 before `queue.pop_front()`. The finding claims "without
// logging" which is incorrect.
//
// No test needed — this is demonstrably disproven by reading the source.

// ============================================================================
// Finding #9: Telegram UTF-16 mention silent fail
// crates/oxicrab-channels/src/telegram/mod.rs:879
// ============================================================================

/// Replicates `utf16_substr` — returns None on out-of-range offsets.
fn utf16_substr(s: &str, offset: usize, length: usize) -> Option<String> {
    let utf16: Vec<u16> = s.encode_utf16().collect();
    let slice = utf16.get(offset..offset + length)?;
    String::from_utf16(slice).ok()
}

#[test]
fn audit_channels_09_telegram_utf16_mention_silent_fail() {
    // Valid offsets
    assert_eq!(utf16_substr("hello @bot", 6, 4).as_deref(), Some("@bot"));
    // Out-of-range offset (malformed entity) — returns None. The caller in
    // is_bot_mentioned drops this silently with no warn/debug log, so a
    // malformed mention entity looks the same as "bot not mentioned".
    assert_eq!(utf16_substr("hi", 100, 4), None);
    // Oversized length — same silent-None outcome.
    assert_eq!(utf16_substr("hi", 0, 100), None);
}

// ============================================================================
// Finding #10: Slack seen-messages eviction silent — PARTIAL
// crates/oxicrab-channels/src/slack/mod.rs:1425-1440
// ============================================================================
// Source DOES log at `debug!` level:
//   debug!("Pruned Slack dedup set to {} entries", seen.len());
// So it is not strictly silent, but it is debug-level and won't surface at
// typical production log levels (info/warn). Finding is PARTIAL.
//
// No test needed — verdict established by reading the source.
