//! Audit regression tests for the memory/search subsystem.
//!
//! Each test that can be reduced to a reproducing case lives here as
//! `audit_memory_NN_shortname`. Tests are FAILING (red) when the bug is
//! present so a fix is visible in CI.
//!
//! Findings that are not testable at the unit level (races that require a
//! deliberate schedule, Windows-only behavior, embedding service
//! instrumentation) are skipped; see the audit report for details.

use oxicrab::agent::memory::memory_db::MemoryDB;
use oxicrab_core::config::schema::FusionStrategy;

/// Finding 5 (MED): BM25 normalization collapses all near-tie scores to `1.0`
/// when `range.abs() < 1e-10`, destroying the original BM25 ordering. For
/// large result sets whose raw scores differ by small but meaningful amounts
/// (e.g. many entries with identical keyword counts plus one slight
/// outlier), ordering should be preserved. This test builds a corpus where
/// BM25 scores cluster tightly, then asserts that hybrid_search does NOT
/// return results in an order that looks flattened.
///
/// Test strategy: insert N entries containing the query terms such that the
/// top-ranked BM25 row is clearly the best match. When the range is tiny the
/// current code flattens every normalized score to `1.0`, which lets the
/// vector component alone decide ordering — even when `keyword_weight = 1.0`.
#[test]
fn audit_memory_05_bm25_range_epsilon_flattens_ordering() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("audit05.db");
    let db = MemoryDB::new(&db_path).unwrap();

    // All entries share the query terms so BM25 scores are very close.
    // One entry has more occurrences — it should rank first.
    db.insert_memory("a.md", "rust rust rust rust rust rust")
        .unwrap();
    for i in 0..20 {
        db.insert_memory(&format!("b{i}.md"), "rust background filler text")
            .unwrap();
    }

    // keyword_weight = 1.0 -> vector component is disabled; ordering should
    // come purely from BM25.
    let query_emb = vec![0.0_f32; 4];
    let hits = db
        .hybrid_search(
            "rust",
            &query_emb,
            10,
            None,
            1.0,
            FusionStrategy::WeightedScore,
            60,
            0,
        )
        .unwrap();

    assert!(!hits.is_empty(), "expected hits for 'rust' query");
    assert_eq!(
        hits[0].source_key, "a.md",
        "top hit should be the entry with most matches; \
         BM25 range-epsilon flattening broke ordering (got {})",
        hits[0].source_key,
    );
}

/// Finding 6 (MED): Group-mode daily exclusion is only applied in
/// `get_memory_context_scoped`. The dedup helpers (e.g.
/// `is_semantically_duplicate`) call `hybrid_search` with
/// `exclude_sources = None`, so in a group chat dedup still matches
/// against the user's personal `daily:` entries.
///
/// We assert the shape of the API: `is_semantically_duplicate(content,
/// threshold)` takes no `is_group` argument, so it cannot honor the
/// group-mode exclusion invariant. A fixed implementation would expose a
/// group-aware variant or accept an exclude set.
#[test]
fn audit_memory_06_dedup_ignores_group_mode_daily_exclusion() {
    // This is a structural assertion: searching the memory_store source for
    // an `is_group` parameter or a `list_daily_source_keys` call in the
    // dedup path should find nothing. We keep the check cheap by scanning
    // the compiled module's public API via the function signature.
    //
    // If a future fix adds `is_semantically_duplicate_scoped(content,
    // threshold, is_group)` or equivalent, this test should be updated to
    // exercise the new path and then flipped to PASS.
    let src = include_str!("../crates/oxicrab-memory/src/memory_store/mod.rs");

    // After the fix: the dedup function takes an explicit `is_group` flag
    // and, when set, filters out daily:* entries at search time.
    assert!(
        src.contains("is_semantically_duplicate") && src.contains("is_group: bool"),
        "is_semantically_duplicate must take an is_group flag after the fix",
    );

    // The call site passes `None` for exclude_sources, i.e. it never
    // excludes daily:* entries. This is the bug.
    let dedup_block_start = src
        .find("pub fn is_semantically_duplicate")
        .expect("dedup fn must exist");
    let dedup_block = &src[dedup_block_start..];
    let end = dedup_block
        .find("\n    }\n")
        .map_or(dedup_block.len(), |i| i + 6);
    let dedup_body = &dedup_block[..end];

    // The failing invariant: dedup must either accept an is_group flag or
    // pass a non-None exclude set. Today it does neither.
    let has_group_flag = dedup_body.contains("is_group");
    let has_exclude = dedup_body.contains("list_daily_source_keys")
        || dedup_body.contains("exclude_sources: Some");
    assert!(
        has_group_flag || has_exclude,
        "is_semantically_duplicate does not honor group-mode daily exclusion: \
         no is_group parameter and no daily exclude set is passed to hybrid_search",
    );
}

/// Finding 7 (MED): CLAUDE.md and `memory_store::is_semantically_duplicate`
/// both document a 0.7 Jaccard threshold, but `is_duplicate_of_entries`
/// uses 0.55 — an aggressive word-level threshold that rejects memories
/// that share common filler words with older entries.
///
/// Lock the documented threshold in place: two sentences with only two
/// words in common should NOT be considered duplicates.
#[test]
fn audit_memory_07_jaccard_threshold_too_aggressive() {
    use oxicrab_memory::remember::{is_duplicate_of_entries, jaccard_similarity};

    // Two distinct facts that share common English filler words.
    let new_content = "the deadline is Friday";

    // Jaccard: {the, is} / {the, server, is, 10.0.0.1, deadline, Friday}
    // = 2/6 = 0.333. Should not be a duplicate under a sane threshold.
    let sim = jaccard_similarity(new_content, "the server is 10.0.0.1");
    assert!(
        sim < 0.5,
        "unexpected jaccard similarity between distinct facts: {sim}",
    );

    // The current 0.55 threshold correctly rejects this case, but another
    // pair with more overlap exposes the aggressive gate.
    let existing2 = vec!["- remember the meeting is on Friday at noon".to_string()];
    let new_content2 = "the meeting is on Monday at noon";
    let sim2 = jaccard_similarity(new_content2, "remember the meeting is on Friday at noon");
    // Jaccard ≈ 6/9 ≈ 0.667 — under a 0.7 threshold this is NOT a
    // duplicate (different day), but under 0.55 it IS. The aggressive
    // threshold loses real information.
    assert!(
        (0.55..0.7).contains(&sim2),
        "test corpus invalid: jaccard={sim2}, expected 0.55 <= sim < 0.7",
    );
    assert!(
        !is_duplicate_of_entries(new_content2, &existing2),
        "0.55 threshold wrongly rejects '{new_content2}' as a duplicate of \
         '{}' (jaccard={sim2:.3}); the documented 0.7 threshold would accept it",
        existing2[0],
    );
}
