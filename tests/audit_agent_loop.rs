//! Validation tests for AGENT LOOP audit findings (validate/audit-findings).
//!
//! Each test reproduces a concrete, observable symptom of a reported bug via
//! the crate's public API. Tests are written to FAIL today and only pass
//! once the underlying issue is fixed.
//!
//! Findings that rely on private internals (e.g. `coalesce_messages`,
//! `is_reset_command`, `AgentLoop::execute_tools`) are validated by reading
//! the code — no integration test is included for those here.

use oxicrab::agent::approval::{ApprovalDecision, ApprovalStore};
use oxicrab::agent::compaction::estimate_tokens;

/// Finding #3 — Pre-flight token estimate is ±50% off on non-ASCII.
///
/// The estimator divides `chars().count()` by a constant 4. For CJK and
/// emoji-heavy inputs the real tokenizer produces 1–2 tokens per character,
/// so `chars/4` under-estimates by 4×–8×. For pure whitespace or repeated
/// ASCII punctuation it over-estimates. We assert that a Japanese sample
/// produces an estimate within 50% of the ground-truth cl100k-ish count
/// (heuristic: ~1 token per CJK char). This test fails today.
#[test]
fn audit_agent_loop_03_preflight_nonascii_estimate() {
    // Japanese pangram-ish text: each char should be roughly 1 token.
    let text = "いろはにほへとちりぬるをわかよたれそつねならむ"; // 23 chars
    let estimated = estimate_tokens(text);
    let approx_real = text.chars().count(); // ~1 token per CJK char
    let low = approx_real / 2;
    let high = approx_real * 3 / 2;
    assert!(
        estimated >= low && estimated <= high,
        "non-ASCII token estimate {estimated} out of ±50% of approx real {approx_real} \
         (range {low}..={high}) — char/4 heuristic is inaccurate for CJK text",
    );
}

/// Finding #9 — Approval store is purely in-memory; a new store has no
/// knowledge of approvals registered in a prior process. We cannot restart
/// across test runs, but we can show that constructing a fresh store never
/// carries pending entries and `resolve` on any id returns the "resolved or
/// expired" error — proving there is no persistence layer to recover from.
///
/// This test is a regression guard: it PASSES today (documenting the
/// current ephemeral behaviour) and will FAIL when durable approval
/// storage is added, forcing the fix to update the semantics here.
#[test]
fn audit_agent_loop_09_approval_ephemeral_loss_on_restart() {
    let store = ApprovalStore::new();
    assert!(
        store.pending_ids().is_empty(),
        "fresh ApprovalStore must start empty — ephemeral by design",
    );
    // Any previously-issued approval id is unknown to a new store.
    let prior_id = ApprovalStore::generate_id();
    let err = store
        .resolve(&prior_id, "slack:C123", ApprovalDecision::Approved)
        .expect_err("resolve must fail: nothing persisted across 'restarts'");
    assert!(
        err.contains("already been resolved or expired"),
        "expected expired-or-resolved error, got: {err}",
    );
}
