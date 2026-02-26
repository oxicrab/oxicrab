//! Memory quality gates: filter low-signal content and reframe negative memories
//! before they are persisted to daily notes or memory storage.

/// Result of running content through quality gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualityVerdict {
    /// Content passes all gates unchanged.
    Pass,
    /// Content was reframed to a constructive form.
    Reframed(String),
    /// Content was rejected as low-signal.
    Reject(RejectReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    TooShort,
    Greeting,
    Filler,
}

const MIN_USEFUL_LEN: usize = 15;

const GREETINGS: &[&str] = &[
    "hi",
    "hello",
    "hey",
    "yo",
    "sup",
    "thanks",
    "thank you",
    "thx",
    "ty",
    "bye",
    "goodbye",
    "good morning",
    "good evening",
    "good night",
    "good afternoon",
    "gm",
    "gn",
];

const FILLER: &[&str] = &[
    "ok",
    "okay",
    "sure",
    "yes",
    "no",
    "yep",
    "nope",
    "yeah",
    "nah",
    "cool",
    "nice",
    "great",
    "awesome",
    "got it",
    "understood",
    "alright",
    "right",
    "fine",
    "lol",
    "haha",
    "lmao",
    "hmm",
    "hm",
    "ah",
    "oh",
    "uh",
    "um",
    "wow",
    "k",
    "kk",
];

const NEGATIVE_PATTERNS: &[&str] = &[
    "was broken",
    "were broken",
    "didn't work",
    "doesn't work",
    "don't work",
    "failed",
    "crashed",
    "bug in",
    "was failing",
    "kept failing",
    "is broken",
    "are broken",
    "was wrong",
    "caused errors",
    "threw an error",
    "threw errors",
    "was buggy",
    "had a bug",
    "kept crashing",
    "wouldn't start",
    "couldn't connect",
    "timed out",
    "was down",
];

/// Run content through all quality gates. Returns a verdict indicating whether
/// the content should pass, be reframed, or be rejected.
pub fn check_quality(content: &str) -> QualityVerdict {
    let trimmed = content.trim();

    if trimmed.len() < MIN_USEFUL_LEN {
        return QualityVerdict::Reject(RejectReason::TooShort);
    }

    let lower = trimmed.to_lowercase();

    // Strip trailing punctuation for greeting/filler matching
    let normalized = lower.trim_end_matches(|c: char| c.is_ascii_punctuation());

    if GREETINGS.contains(&normalized) {
        return QualityVerdict::Reject(RejectReason::Greeting);
    }

    if FILLER.contains(&normalized) {
        return QualityVerdict::Reject(RejectReason::Filler);
    }

    // Check for negative patterns and reframe if found
    if let Some(reframed) = try_reframe_negative(&lower, trimmed) {
        return QualityVerdict::Reframed(reframed);
    }

    QualityVerdict::Pass
}

/// Attempt to reframe negative/broken memory content into constructive form.
/// Returns `Some(reframed)` if the content matched a negative pattern.
fn try_reframe_negative(lower: &str, original: &str) -> Option<String> {
    let matched = NEGATIVE_PATTERNS.iter().any(|pat| lower.contains(pat));
    if !matched {
        return None;
    }

    // If the content already has constructive framing, let it pass
    let constructive_markers = [
        "fixed by",
        "solved by",
        "resolved by",
        "workaround:",
        "fix:",
        "solution:",
        "to fix",
        "instead use",
        "use instead",
        "TODO:",
        "todo:",
    ];
    if constructive_markers.iter().any(|m| lower.contains(m)) {
        return None;
    }

    // Reframe: prepend "NOTE:" and append corrective hint
    Some(format!(
        "NOTE (reframed): {} — verify current state before relying on this",
        original
    ))
}

/// Filter multi-line content (e.g. LLM-extracted facts) through quality gates.
/// Each line is checked independently. Rejected lines are dropped, reframed lines
/// are replaced. Returns the filtered content (may be empty if all lines rejected).
pub fn filter_lines(content: &str) -> String {
    let mut output = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        // Preserve headers and empty lines
        if trimmed.is_empty() || trimmed.starts_with('#') {
            output.push(line.to_string());
            continue;
        }
        // Strip leading "- " for quality check, preserve formatting
        let (prefix, check_text) = if let Some(rest) = trimmed.strip_prefix("- ") {
            ("- ", rest)
        } else {
            ("", trimmed)
        };
        match check_quality(check_text) {
            QualityVerdict::Pass => output.push(line.to_string()),
            QualityVerdict::Reframed(reframed) => {
                let indent = line.len() - trimmed.len();
                output.push(format!("{}{}{}", &line[..indent], prefix, reframed));
            }
            QualityVerdict::Reject(_) => {
                // Drop low-signal lines
            }
        }
    }

    // Clean up: remove trailing empty lines that resulted from dropped content
    while output.last().is_some_and(|l| l.trim().is_empty()) {
        output.pop();
    }

    output.join("\n")
}

#[cfg(test)]
mod tests;
