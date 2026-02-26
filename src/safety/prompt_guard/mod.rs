use regex::Regex;
use tracing::warn;

/// Category of detected prompt injection pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InjectionCategory {
    RoleSwitch,
    InstructionOverride,
    SecretExtraction,
    Jailbreak,
}

/// A prompt injection pattern match.
#[derive(Debug)]
pub struct InjectionMatch {
    pub category: InjectionCategory,
    pub pattern_name: &'static str,
    pub matched_text: String,
}

struct GuardPattern {
    category: InjectionCategory,
    name: &'static str,
    regex: Regex,
}

/// Regex-based prompt injection detection guard.
///
/// Scans text for patterns across 4 categories:
/// 1. Role switching — attempts to change the LLM's persona
/// 2. Instruction override — attempts to replace system prompts
/// 3. Secret extraction — attempts to extract system prompts or secrets
/// 4. Jailbreak patterns — common jailbreak prefixes
///
/// Disabled by default; enabled via `agents.defaults.promptGuard.enabled`.
pub struct PromptGuard {
    patterns: Vec<GuardPattern>,
}

impl Default for PromptGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptGuard {
    pub fn new() -> Self {
        let pattern_defs: Vec<(InjectionCategory, &str, &str)> = vec![
            // Role switching
            (
                InjectionCategory::RoleSwitch,
                "ignore_previous",
                r"(?i)\b(?:ignore|disregard|forget)\b.{0,50}\b(?:previous|above|prior|all)\b.{0,50}\b(?:instructions?|prompts?|rules?|guidelines?)\b",
            ),
            (
                InjectionCategory::RoleSwitch,
                "you_are_now",
                r"(?i)\byou are now\b.{0,50}\b(?:acting as|pretending|roleplaying|playing|a new)\b",
            ),
            (
                InjectionCategory::RoleSwitch,
                "new_persona",
                r"(?i)\b(?:from now on|henceforth)\b.{0,50}\b(?:you are|act as|behave as|respond as)\b",
            ),
            // Instruction override
            (
                InjectionCategory::InstructionOverride,
                "new_instructions",
                r"(?i)(?:^|\n)\s*(?:system|new|updated|revised)\s*(?:prompt|instructions?|rules?)\s*:",
            ),
            (
                InjectionCategory::InstructionOverride,
                "override_system",
                r"(?i)\b(?:override|replace|overwrite)\b.{0,50}\b(?:system|original|initial)\b.{0,50}\b(?:prompt|instructions?|rules?)\b",
            ),
            // Secret extraction
            (
                InjectionCategory::SecretExtraction,
                "reveal_prompt",
                r"(?i)\b(?:repeat|show|display|output|print|reveal|tell me)\b.{0,50}\b(?:your|the|its|system)\s+(?:system prompt|instructions?|initial prompt|rules|guidelines)\b",
            ),
            (
                InjectionCategory::SecretExtraction,
                "what_are_your",
                r"(?i)\bwhat (?:are|is|were) your\b.{0,50}\b(?:instructions?|rules?|system prompt|guidelines)\b",
            ),
            // Jailbreak patterns
            (
                InjectionCategory::Jailbreak,
                "dan_mode",
                r"(?i)\b(?:DAN|developer|god)\s*mode\b",
            ),
            (
                InjectionCategory::Jailbreak,
                "jailbreak",
                r"(?i)\bjailbreak\b",
            ),
            (
                InjectionCategory::Jailbreak,
                "do_anything_now",
                r"(?i)\bdo anything now\b",
            ),
        ];

        let patterns = pattern_defs
            .into_iter()
            .filter_map(|(category, name, pattern)| match Regex::new(pattern) {
                Ok(regex) => Some(GuardPattern {
                    category,
                    name,
                    regex,
                }),
                Err(e) => {
                    warn!("failed to compile prompt guard pattern '{}': {}", name, e);
                    None
                }
            })
            .collect();

        Self { patterns }
    }

    /// Strip zero-width, invisible, and combining Unicode characters that attackers
    /// use to evade regex-based detection (e.g. "ig\u{200B}nore" → "ignore").
    /// Also strips RTL/LTR overrides, combining diacriticals, and variation selectors.
    fn normalize(text: &str) -> String {
        text.chars()
            .filter(|c| {
                !matches!(
                    *c,
                    '\u{200B}' // zero-width space
                    | '\u{200C}' // zero-width non-joiner
                    | '\u{200D}' // zero-width joiner
                    | '\u{200E}' // left-to-right mark
                    | '\u{200F}' // right-to-left mark
                    | '\u{FEFF}' // byte-order mark / zero-width no-break space
                    | '\u{00AD}' // soft hyphen
                    | '\u{034F}' // combining grapheme joiner
                    | '\u{2060}' // word joiner
                    | '\u{2061}' // function application
                    | '\u{2062}' // invisible times
                    | '\u{2063}' // invisible separator
                    | '\u{2064}' // invisible plus
                    | '\u{FE00}'..='\u{FE0F}' // variation selectors
                    | '\u{0300}'..='\u{036F}' // combining diacritical marks
                    | '\u{1AB0}'..='\u{1AFF}' // combining diacritical marks extended
                    | '\u{1DC0}'..='\u{1DFF}' // combining diacritical marks supplement
                    | '\u{20D0}'..='\u{20FF}' // combining diacritical marks for symbols
                    | '\u{FE20}'..='\u{FE2F}' // combining half marks
                    | '\u{202A}'..='\u{202E}' // bidi control (LRE, RLE, PDF, LRO, RLO)
                    | '\u{2066}'..='\u{2069}' // bidi isolates (LRI, RLI, FSI, PDI)
                    | '\u{E0100}'..='\u{E01EF}' // variation selectors supplement
                )
            })
            .collect()
    }

    /// Scan text for prompt injection patterns. Returns all matches found.
    pub fn scan(&self, text: &str) -> Vec<InjectionMatch> {
        let normalized = Self::normalize(text);
        let mut matches = Vec::new();
        for pattern in &self.patterns {
            for m in pattern.regex.find_iter(&normalized) {
                matches.push(InjectionMatch {
                    category: pattern.category.clone(),
                    pattern_name: pattern.name,
                    matched_text: m.as_str().to_string(),
                });
            }
        }
        matches
    }

    /// Returns true if any match was found (for block/warn decisions).
    pub fn should_block(&self, text: &str) -> bool {
        let normalized = Self::normalize(text);
        self.patterns.iter().any(|p| p.regex.is_match(&normalized))
    }
}

#[cfg(test)]
mod tests;
