use std::sync::LazyLock;

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

/// Compiled prompt guard patterns, initialized once on first access.
static GUARD_PATTERNS: LazyLock<Vec<GuardPattern>> = LazyLock::new(|| {
    let pattern_defs: Vec<(InjectionCategory, &str, &str)> = vec![
        // Role switching
        // (?is) = case-insensitive + dotall (`.` matches `\n`) to prevent
        // multi-line bypass where attacker splits injection across lines.
        (
            InjectionCategory::RoleSwitch,
            "ignore_previous",
            r"(?is)\b(?:ignore|disregard|forget)\b.{0,50}\b(?:previous|above|prior|all)\b.{0,50}\b(?:instructions?|prompts?|rules?|guidelines?)\b",
        ),
        (
            InjectionCategory::RoleSwitch,
            "you_are_now",
            r"(?is)\byou are now\b.{0,50}\b(?:acting as|pretending|roleplaying|playing|a new)\b",
        ),
        (
            InjectionCategory::RoleSwitch,
            "new_persona",
            r"(?is)\b(?:from now on|henceforth)\b.{0,50}\b(?:you are|act as|behave as|respond as)\b",
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
            r"(?is)\b(?:override|replace|overwrite)\b.{0,50}\b(?:system|original|initial)\b.{0,50}\b(?:prompt|instructions?|rules?)\b",
        ),
        // Secret extraction
        (
            InjectionCategory::SecretExtraction,
            "reveal_prompt",
            r"(?is)\b(?:repeat|show|display|output|print|reveal|tell me)\b.{0,50}\b(?:your|the|its|system)\s+(?:system prompt|instructions?|initial prompt|rules|guidelines)\b",
        ),
        (
            InjectionCategory::SecretExtraction,
            "what_are_your",
            r"(?is)\bwhat (?:are|is|were) your\b.{0,50}\b(?:instructions?|rules?|system prompt|guidelines)\b",
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
        // Few-shot prefix injection (impersonating conversation roles)
        (
            InjectionCategory::RoleSwitch,
            "few_shot_prefix",
            r"(?ism)^\s*(?:assistant|system|human)\s*:",
        ),
        // Persona assignment without "you are now"
        (
            InjectionCategory::RoleSwitch,
            "persona_assignment",
            r"(?is)(?:pretend|act as if|respond as if|take (?:on )?the role of).{0,30}(?:you (?:are|were)|a |an )",
        ),
        // Base64/encoded instruction references
        (
            InjectionCategory::InstructionOverride,
            "encoded_instructions",
            r"(?is)(?:decode|base64|rot13).{0,30}(?:follow|execute|obey|instructions)",
        ),
    ];

    pattern_defs
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
        .collect()
});

/// Regex-based prompt injection detection guard.
///
/// Scans text for patterns across 4 categories:
/// 1. Role switching — attempts to change the LLM's persona
/// 2. Instruction override — attempts to replace system prompts
/// 3. Secret extraction — attempts to extract system prompts or secrets
/// 4. Jailbreak patterns — common jailbreak prefixes
///
/// Patterns are compiled once globally via `LazyLock` and shared across
/// all instances. Disabled by default; enabled via `agents.defaults.promptGuard.enabled`.
pub struct PromptGuard;

impl Default for PromptGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptGuard {
    pub fn new() -> Self {
        Self
    }

    /// Strip zero-width, invisible, and combining Unicode characters that attackers
    /// use to evade regex-based detection (e.g. "ig\u{200B}nore" → "ignore").
    /// Also strips RTL/LTR overrides, combining diacriticals, and variation selectors.
    fn normalize(text: &str) -> String {
        let stripped: String = text
            .chars()
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
            .collect();
        Self::transliterate_confusables(&stripped)
    }

    /// Transliterate common Unicode confusables (homoglyphs) to ASCII equivalents.
    fn transliterate_confusables(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                // Cyrillic + Greek lookalikes merged by target letter
                '\u{0410}' | '\u{0430}' | '\u{0391}' | '\u{03B1}' => 'a',
                '\u{0412}' | '\u{0432}' => 'b',
                '\u{0421}' | '\u{0441}' => 'c',
                '\u{0415}' | '\u{0435}' | '\u{0395}' | '\u{03B5}' => 'e',
                '\u{041D}' | '\u{043D}' => 'h',
                '\u{0406}' | '\u{0456}' => 'i',
                '\u{041E}' | '\u{043E}' | '\u{039F}' | '\u{03BF}' => 'o',
                '\u{0420}' | '\u{0440}' => 'p',
                '\u{0405}' | '\u{0455}' => 's',
                '\u{0422}' | '\u{0442}' => 't',
                '\u{0425}' | '\u{0445}' => 'x',
                '\u{0423}' | '\u{0443}' => 'y',
                '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFF01 + 0x21).unwrap_or(c),
                _ => c,
            })
            .collect()
    }

    /// Scan text for prompt injection patterns. Returns all matches found.
    pub fn scan(&self, text: &str) -> Vec<InjectionMatch> {
        let normalized = Self::normalize(text);
        let mut matches = Vec::new();
        for pattern in GUARD_PATTERNS.iter() {
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
        !self.scan(text).is_empty()
    }
}

#[cfg(test)]
mod tests;
