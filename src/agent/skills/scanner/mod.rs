use regex::Regex;
use std::sync::LazyLock;

/// A dangerous pattern found in a skill file.
#[derive(Debug)]
pub struct SkillFinding {
    pub category: &'static str,
    pub pattern_name: &'static str,
    pub matched_text: String,
    pub line_number: usize,
}

/// Result of scanning a skill file for dangerous patterns.
#[derive(Debug)]
pub struct SkillScanResult {
    pub blocked: Vec<SkillFinding>,
    pub warnings: Vec<SkillFinding>,
}

impl SkillScanResult {
    pub fn is_clean(&self) -> bool {
        self.blocked.is_empty()
    }
}

struct ScanPattern {
    category: &'static str,
    name: &'static str,
    regex: Regex,
    block: bool, // true = block, false = warn
}

static SCAN_PATTERNS: LazyLock<Vec<ScanPattern>> = LazyLock::new(|| {
    let defs: Vec<(&str, &str, &str, bool)> = vec![
        // --- BLOCKED: Prompt injection ---
        (
            "prompt_injection",
            "role_override",
            r"(?i)\b(?:ignore|disregard|forget)\b.{0,30}\b(?:previous|prior|above|all)\b.{0,20}\b(?:instructions?|prompts?|rules?)\b",
            true,
        ),
        (
            "prompt_injection",
            "new_identity",
            r"(?i)\byou\s+are\s+now\b.{0,50}\b(?:new|different|another)\b",
            true,
        ),
        (
            "prompt_injection",
            "system_prompt_override",
            r"(?i)\b(?:new|override|replace|overwrite)\b.{0,20}\b(?:system\s+prompt|instructions)\b",
            true,
        ),
        (
            "prompt_injection",
            "secret_extraction",
            r"(?i)\b(?:reveal|show|output|print|display|leak|expose)\b.{0,30}\b(?:system\s+prompt|api\s*key|secret|credential|password|token)\b",
            true,
        ),
        // --- BLOCKED: Credential exfiltration commands ---
        (
            "credential_exfiltration",
            "curl_env",
            r"(?i)\bcurl\b.{0,60}\$\{?\w*(?:KEY|SECRET|TOKEN|PASSWORD|CREDENTIAL)\w*\}?",
            true,
        ),
        (
            "credential_exfiltration",
            "wget_env",
            r"(?i)\bwget\b.{0,60}\$\{?\w*(?:KEY|SECRET|TOKEN|PASSWORD|CREDENTIAL)\w*\}?",
            true,
        ),
        (
            "credential_exfiltration",
            "printenv_exfil",
            r"(?i)\b(?:printenv|env\b|set\b).{0,30}\b(?:curl|wget|nc|ncat)\b",
            true,
        ),
        (
            "credential_exfiltration",
            "cat_sensitive",
            r"(?i)\bcat\b.{0,20}(?:/etc/(?:passwd|shadow)|\.env\b|\.ssh/|credentials)",
            true,
        ),
        // --- BLOCKED: Reverse shell patterns ---
        (
            "reverse_shell",
            "netcat_exec",
            r"(?i)\bnc\b.{0,30}-[elp]",
            true,
        ),
        (
            "reverse_shell",
            "bash_interactive",
            r"(?i)\bbash\s+-i\b.{0,30}/dev/tcp/",
            true,
        ),
        ("reverse_shell", "dev_tcp", r"/dev/tcp/\d", true),
        (
            "reverse_shell",
            "mkfifo_pipe",
            r"(?i)\bmkfifo\b.{0,60}\bnc\b",
            true,
        ),
        // --- WARNED: Suspicious patterns ---
        (
            "suspicious",
            "base64_decode_pipe",
            r"(?i)\bbase64\b.{0,20}(?:-d|--decode).{0,20}\|\s*(?:sh|bash|zsh)\b",
            false,
        ),
        (
            "suspicious",
            "eval_exec",
            r"(?i)\b(?:eval|exec)\b.{0,30}\$\(",
            false,
        ),
        (
            "suspicious",
            "python_exec",
            r"(?i)\bpython[23]?\s+-c\b.{0,60}(?:import\s+(?:os|subprocess|socket)|exec\(|eval\()",
            false,
        ),
    ];

    defs.into_iter()
        .filter_map(|(category, name, pattern, block)| {
            Regex::new(pattern).ok().map(|regex| ScanPattern {
                category,
                name,
                regex,
                block,
            })
        })
        .collect()
});

/// Scan skill content for dangerous patterns before injection into the system prompt.
///
/// Returns blocked findings (skill should not be loaded) and warnings
/// (skill can be loaded but operator should review).
///
/// Scans both per-line (for most patterns) and with a sliding window of
/// adjacent lines joined together (to catch multi-line split evasion).
pub fn scan_skill(content: &str) -> SkillScanResult {
    let mut blocked = Vec::new();
    let mut warnings = Vec::new();

    let lines: Vec<&str> = content.lines().collect();

    for (line_idx, line) in lines.iter().enumerate() {
        // Skip code fence markers themselves
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            continue;
        }

        for pattern in SCAN_PATTERNS.iter() {
            if let Some(m) = pattern.regex.find(line) {
                let finding = SkillFinding {
                    category: pattern.category,
                    pattern_name: pattern.name,
                    matched_text: {
                        let s = m.as_str();
                        s[..s.floor_char_boundary(100)].to_string()
                    },
                    line_number: line_idx + 1,
                };
                if pattern.block {
                    blocked.push(finding);
                } else {
                    warnings.push(finding);
                }
            }
        }
    }

    // Multi-line sliding window: join adjacent lines to catch split evasion
    // (e.g., "ignore all previous\ninstructions and rules")
    for window_size in [2, 3, 4, 5] {
        if lines.len() < window_size {
            continue;
        }
        for start in 0..lines.len() - (window_size - 1) {
            let joined: String = lines[start..start + window_size].join(" ");
            for pattern in SCAN_PATTERNS.iter() {
                if let Some(m) = pattern.regex.find(&joined) {
                    // Only report if this wasn't already caught by single-line scan
                    let ms = m.as_str();
                    let matched = ms[..ms.floor_char_boundary(100)].to_string();
                    let already_found = if pattern.block {
                        blocked
                            .iter()
                            .any(|f| f.pattern_name == pattern.name && f.matched_text == matched)
                    } else {
                        warnings
                            .iter()
                            .any(|f| f.pattern_name == pattern.name && f.matched_text == matched)
                    };
                    if !already_found {
                        let finding = SkillFinding {
                            category: pattern.category,
                            pattern_name: pattern.name,
                            matched_text: matched,
                            line_number: start + 1,
                        };
                        if pattern.block {
                            blocked.push(finding);
                        } else {
                            warnings.push(finding);
                        }
                    }
                }
            }
        }
    }

    SkillScanResult { blocked, warnings }
}

#[cfg(test)]
mod tests;
