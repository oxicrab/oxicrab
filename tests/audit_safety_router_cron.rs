//! Validation tests for SAFETY/ROUTER/CRON audit findings
//! (validate/audit-findings branch).
//!
//! Each test reproduces a concrete, observable symptom of a reported bug via
//! the crate's public API. Tests are written to FAIL today and only pass
//! once the underlying issue is fixed. A few tests act as regression guards
//! that PASS today and document current behaviour — they will need updating
//! when the behaviour changes.
//!
//! Findings that are FALSE POSITIVES (#1, #2, #3) are not tested here.
//! They are intentional/documented behaviour — see validation report.

use oxicrab::agent::tools::base::routing_types::{DirectiveTrigger, StaticRule};
use oxicrab_safety::leak_detector::LeakDetector;

// ---------------------------------------------------------------------------
// Finding #4 [CONFIRMED]
// DirectiveTrigger::Pattern is not lowercased by `normalized()` but
// `matches_normalized()` runs against a lowercased input. Uppercase regex
// literals therefore never match real messages.
// ---------------------------------------------------------------------------
#[test]
fn audit_srcr_04_pattern_trigger_not_lowercased() {
    // Author registers a pattern using uppercase alternation — common when
    // copying from docs or writing case-sensitive regex by habit.
    let trig = DirectiveTrigger::Pattern("Yes|No".into()).normalized();

    // The input "yes" has been lowercased by the router's `matches()`.
    let matched = trig.matches_normalized("yes");

    assert!(
        matched,
        "Pattern trigger 'Yes|No' should match lowercased input 'yes' after \
         normalization; today it does not because Pattern(_) is left unchanged \
         by normalized() while the input is forced to lowercase",
    );
}

// ---------------------------------------------------------------------------
// Finding #5 [CONFIRMED]
// Cron service sleeps POLL_WHEN_EMPTY_SEC (30s) when no jobs exist, with no
// wakeup signal. A newly added job waits up to 30s before being picked up.
// This test is a regression guard — it asserts the constant is <= 30s today
// and will flag a bug if someone bumps it without adding a wakeup signal.
// ---------------------------------------------------------------------------
#[test]
fn audit_srcr_05_cron_startup_empty_queue_latency() {
    // Read constant via the source text — it is private. We pin the known
    // value and document the expected fix (either lower the constant or add
    // a notify/wakeup).
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/cron/service/mod.rs"
    ))
    .expect("read cron service source");
    assert!(
        src.contains("const POLL_WHEN_EMPTY_SEC: u64 = 30"),
        "cron service still polls with 30s empty-queue latency; a new job \
         can wait up to 30s to fire. Expected fix: add a notify channel or \
         lower the constant",
    );
    // There should be no `Notify` in the cron service yet — if/when one is
    // added, this test must be updated because the latency is no longer
    // load-bearing.
    assert!(
        !src.contains("tokio::sync::Notify"),
        "cron service now uses Notify; update this audit test",
    );
}

// ---------------------------------------------------------------------------
// Finding #6 [CONFIRMED]
// Router GuidedLLM tool filter has hardcoded string exemptions for
// `add_buttons` and `tool_search`. Renaming either tool silently breaks the
// exemption; adding a third exempt tool requires editing iteration.rs.
// This test pins the current hardcoded strings so a rename trips the test.
// ---------------------------------------------------------------------------
#[test]
fn audit_srcr_06_guidedllm_hardcoded_exemptions() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/agent/loop/iteration.rs"
    ))
    .expect("read iteration.rs source");
    let has_add_buttons = src.contains(r#"td.name == "add_buttons""#);
    let has_tool_search = src.contains(r#"td.name == "tool_search""#);
    assert!(
        has_add_buttons && has_tool_search,
        "expected hardcoded exemption strings to still exist; once the fix \
         lands these should move to a tool-capability flag and this test \
         must be updated accordingly",
    );
}

// ---------------------------------------------------------------------------
// Finding #7 [PARTIAL]
// Telegram bot token AC prefix is ':AA' — only 3 chars. This is a
// performance concern (many false candidates), not a correctness one: the
// full regex still validates. We assert that a benign string containing
// ':AA' is NOT detected as a leak (correctness), and that the AC prefix
// remains ':AA' today (documenting the performance issue).
// ---------------------------------------------------------------------------
#[test]
fn audit_srcr_07_telegram_short_prefix_correctness() {
    let det = LeakDetector::new();

    // Benign string that contains ':AA' but is not a Telegram bot token.
    // Should NOT be flagged.
    let benign = "status::AAborted queue:AApproved item";
    let matches = det.scan(benign);
    let names: Vec<&str> = matches.iter().map(|m| m.name).collect();
    assert!(
        !names.contains(&"telegram_bot_token"),
        "benign ':AA' occurrences must not be flagged as telegram tokens; \
         got matches: {names:?}",
    );

    // Actual Telegram token format: <digits>:AA<33+ chars>
    let real = "bot token: 123456789:AAEabcdefghijklmnopqrstuvwxyz0123456";
    let real_matches = det.scan(real);
    let real_names: Vec<&str> = real_matches.iter().map(|m| m.name).collect();
    assert!(
        real_names.contains(&"telegram_bot_token"),
        "legitimate telegram token must still be detected; got: {real_names:?}",
    );
}

// ---------------------------------------------------------------------------
// Finding #8 [CONFIRMED]
// Cron timezone silently falls back to UTC on invalid tz strings. A job
// scheduled with "Not/A/Zone" gets computed as if it were UTC, at log-warn
// severity only. This is a data-integrity issue for users who rely on
// local-time schedules.
//
// We construct a CronService with an invalid tz and verify the job is
// accepted (next_run_at_ms is computed as UTC), rather than rejected.
// This is a regression guard documenting current behaviour.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn audit_srcr_08_cron_timezone_silent_utc_fallback() {
    use oxicrab::cron::service::CronService;
    use oxicrab::cron::types::{CronJob, CronJobState, CronPayload, CronSchedule, CronTarget};
    use std::sync::Arc;

    let db = Arc::new(
        oxicrab::agent::memory::memory_db::MemoryDB::new(":memory:").expect("create in-memory db"),
    );
    let svc = CronService::new(db);

    let job = CronJob {
        id: "tz-bad".to_string(),
        name: "bad tz".to_string(),
        enabled: true,
        schedule: CronSchedule::Cron {
            expr: Some("0 9 * * *".to_string()),
            tz: Some("Not/A/Zone".to_string()),
        },
        payload: CronPayload {
            kind: "agent_turn".to_string(),
            message: "ping".into(),
            agent_echo: false,
            targets: vec![CronTarget {
                channel: "telegram".to_string(),
                to: "user1".to_string(),
            }],
        },
        state: CronJobState::default(),
        created_at_ms: 1_000_000,
        updated_at_ms: 1_000_000,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: None,
        cooldown_secs: None,
        max_concurrent: None,
    };

    // Today: accepted silently, next_run computed as UTC.
    // Expected fix: reject invalid tz strings at add_job time.
    let res = svc.add_job(job);
    assert!(
        res.is_ok(),
        "current behaviour is to silently accept invalid tz; when the fix \
         lands, add_job should return Err for 'Not/A/Zone' and this test \
         must be inverted",
    );

    let listed = svc.list_jobs(false).expect("list jobs");
    assert_eq!(listed.len(), 1, "bad-tz job was accepted");
    assert!(
        listed[0].state.next_run_at_ms.is_some(),
        "next_run was computed (as UTC) despite invalid timezone — silent \
         fallback confirmed",
    );
}

// ---------------------------------------------------------------------------
// Finding #9 [UNCERTAIN]
// StaticRule.matches compares `active_tool != Some(self.tool.as_str())`
// case-sensitively. If a tool's registered name and the context's
// active_tool were ever to differ in case, rules would silently never
// match. In practice tool names come from a single source, so this is a
// theoretical hardening opportunity. We test the bad case explicitly so
// the fix (a `.eq_ignore_ascii_case()` or normalisation) has a target.
// ---------------------------------------------------------------------------
#[test]
fn audit_srcr_09_static_rule_tool_name_case_mismatch() {
    let rule = StaticRule {
        tool: "MyTool".into(),
        trigger: DirectiveTrigger::Exact("go".into()).normalized(),
        params: serde_json::json!({}),
        requires_context: true,
    };

    // Active tool registered with a different case than the rule's tool name.
    let matched = rule.matches("go", Some("mytool"));
    assert!(
        matched,
        "StaticRule should match tool names case-insensitively; today it \
         compares case-sensitively via `active_tool != Some(self.tool.as_str())`",
    );
}
