//! Cross-session skill auto-save — find tool sequences that recur
//! across many turns and persist the most-frequent uncovered one as a
//! new skill file. Fired in the background after each turn end.
//!
//! "Coverage" is checked by reading the `skills_index` table: if a
//! skill name or description already mentions a step in the candidate
//! sequence, the suggester skips it. This keeps the bar low (we'd
//! rather skip a real new skill than write a duplicate of an existing one).

use super::super::memory::memory_db::{MemoryDB, RepeatedSequence, SkillIndexEntry};
use anyhow::Result;
use std::sync::Arc;

/// One candidate ready for promotion. The caller decides whether to
/// fire an LLM round for body generation, or write a fixed-template
/// skill straight to disk.
#[derive(Debug, Clone)]
pub struct SkillCandidate {
    pub fingerprint: String,
    pub steps: Vec<String>,
    pub occurrences: u32,
    pub example_session_id: String,
    pub already_covered: bool,
}

/// Inspect `trajectory_events` for sequences appearing at least
/// `min_occurrences` times. For each, mark whether an existing
/// indexed skill already covers it. Returns highest-frequency first.
pub fn find_candidates(
    db: &Arc<MemoryDB>,
    min_occurrences: u32,
    min_len: usize,
    max_steps: usize,
) -> Result<Vec<SkillCandidate>> {
    let sequences = db.find_repeated_tool_sequences(min_occurrences, min_len, max_steps)?;
    let existing = db.list_skill_index_entries()?;
    Ok(sequences
        .into_iter()
        .map(|s| {
            let covered = is_covered_by_existing(&s, &existing);
            SkillCandidate {
                fingerprint: s.fingerprint,
                steps: s.steps,
                occurrences: s.occurrences,
                example_session_id: s.example_session_id,
                already_covered: covered,
            }
        })
        .collect())
}

/// Cheap heuristic: a candidate is "covered" when an indexed skill
/// mentions every step's tool name (case-insensitive substring) in
/// either its name or description. Conservative — false-positive
/// coverage just suppresses a candidate, never silently rewrites a
/// skill.
fn is_covered_by_existing(seq: &RepeatedSequence, existing: &[SkillIndexEntry]) -> bool {
    if existing.is_empty() {
        return false;
    }
    let needles: Vec<String> = seq
        .steps
        .iter()
        .map(|s| {
            // Compare against the tool root, not "tool/action" — actions
            // are too granular to anchor coverage on.
            s.split('/').next().unwrap_or(s).to_lowercase()
        })
        .collect();
    existing.iter().any(|entry| {
        let haystack = format!("{} {}", entry.name, entry.description).to_lowercase();
        needles.iter().all(|n| haystack.contains(n))
    })
}

/// Pick the first uncovered candidate above the threshold, if any.
/// `min_occurrences` is the auto-save bar; if no candidate clears it,
/// returns `None` and the caller goes back to sleep.
pub fn pick_top_uncovered(
    candidates: Vec<SkillCandidate>,
    min_occurrences: u32,
) -> Option<SkillCandidate> {
    candidates
        .into_iter()
        .find(|c| !c.already_covered && c.occurrences >= min_occurrences)
}

/// Generate a deterministic skill name from a fingerprint. The result
/// is safe to use in `propose.rs` (`[A-Za-z0-9][A-Za-z0-9_-]{0,63}`).
pub fn name_from_fingerprint(fingerprint: &str) -> String {
    let mut name: String = fingerprint
        .chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() => c.to_ascii_lowercase(),
            '/' => '-',
            _ => '_',
        })
        .collect();
    if !name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        name.insert(0, 's');
    }
    name.truncate(48);
    if name.is_empty() {
        "skill".to_string()
    } else {
        format!("auto_{name}")
    }
}

/// Render a default skill body when no LLM is available. Captures the
/// trigger sequence so a future operator can review it.
pub fn default_skill_body(candidate: &SkillCandidate) -> String {
    let steps: Vec<String> = candidate
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}. {s}", i + 1))
        .collect();
    format!(
        "# Auto-suggested workflow\n\n\
         Detected from {n} repeated cross-session occurrences \
         (last seen in session `{sid}`).\n\n\
         ## Tool sequence\n\n{steps}\n\n\
         ## When to use\n\n\
         When the user describes a task that maps to the steps above. \
         Confirm the goal before running the full chain.\n",
        n = candidate.occurrences,
        sid = candidate.example_session_id,
        steps = steps.join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::memory::memory_db::SkillIndexEntry;

    fn entry(name: &str, desc: &str) -> SkillIndexEntry {
        SkillIndexEntry {
            path: format!("/tmp/{name}.md"),
            name: name.to_string(),
            description: desc.to_string(),
            embedding: vec![],
            file_sha256: String::new(),
            embedding_model_id: String::new(),
            use_count: 0,
            last_used_ms: None,
            created_at_ms: 0,
            last_indexed_ms: 0,
        }
    }

    fn seq(fp: &str, steps: &[&str], n: u32) -> RepeatedSequence {
        RepeatedSequence {
            fingerprint: fp.to_string(),
            steps: steps.iter().map(std::string::ToString::to_string).collect(),
            occurrences: n,
            example_session_id: "demo".to_string(),
        }
    }

    #[test]
    fn name_from_fingerprint_normalises() {
        assert_eq!(
            name_from_fingerprint("github/list_issues\tweb_search"),
            "auto_github-list_issues_web_search"
        );
    }

    #[test]
    fn coverage_matches_when_all_tools_appear() {
        let s = seq("a\tb", &["a", "b"], 5);
        let existing = vec![entry("write_status", "Calls a then b")];
        assert!(is_covered_by_existing(&s, &existing));
    }

    #[test]
    fn coverage_misses_when_tool_absent() {
        let s = seq("a\tb", &["a", "b"], 5);
        let existing = vec![entry("only_a", "uses just a tool")];
        assert!(!is_covered_by_existing(&s, &existing));
    }

    #[test]
    fn pick_top_filters_covered_and_threshold() {
        let cands = vec![
            SkillCandidate {
                fingerprint: "x".to_string(),
                steps: vec!["x".into()],
                occurrences: 3,
                example_session_id: "s".into(),
                already_covered: true,
            },
            SkillCandidate {
                fingerprint: "y".to_string(),
                steps: vec!["y".into()],
                occurrences: 7,
                example_session_id: "s".into(),
                already_covered: false,
            },
            SkillCandidate {
                fingerprint: "z".to_string(),
                steps: vec!["z".into()],
                occurrences: 1,
                example_session_id: "s".into(),
                already_covered: false,
            },
        ];
        let pick = pick_top_uncovered(cands, 5).unwrap();
        assert_eq!(pick.fingerprint, "y");
    }

    #[test]
    fn default_body_includes_steps() {
        let c = SkillCandidate {
            fingerprint: "a\tb".to_string(),
            steps: vec!["a".into(), "b".into()],
            occurrences: 5,
            example_session_id: "demo".to_string(),
            already_covered: false,
        };
        let body = default_skill_body(&c);
        assert!(body.contains("1. a"));
        assert!(body.contains("2. b"));
        assert!(body.contains("5 repeated"));
    }
}
