pub mod scanner;

use aho_corasick::AhoCorasick;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use walkdir::WalkDir;

/// Maximum size for a single skill file (1 MB)
const MAX_SKILL_FILE_SIZE: u64 = 1024 * 1024;

/// Maximum total chars of skill content injected into the system prompt
const MAX_SKILL_CONTEXT_CHARS: usize = 20_000;

/// Parse schedule strings like "7am", "9am, 1pm, 5pm" into cron expressions.
/// Returns a list of cron expressions (one per time).
pub fn parse_schedule(schedule: &str) -> Vec<String> {
    schedule
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(parse_time_to_cron)
        .collect()
}

fn parse_time_to_cron(time_str: &str) -> Option<String> {
    let time_str = time_str.trim().to_lowercase();
    let (hour, minute) = if time_str.ends_with("am") || time_str.ends_with("pm") {
        let is_pm = time_str.ends_with("pm");
        let num_part = time_str.trim_end_matches("am").trim_end_matches("pm");
        let parts: Vec<&str> = num_part.split(':').collect();
        let mut hour: u32 = parts[0].parse().ok()?;
        let minute: u32 = if parts.len() > 1 {
            parts[1].parse().ok()?
        } else {
            0
        };
        if is_pm && hour != 12 {
            hour += 12;
        }
        if !is_pm && hour == 12 {
            hour = 0;
        }
        (hour, minute)
    } else if time_str.contains(':') {
        let parts: Vec<&str> = time_str.split(':').collect();
        let hour: u32 = parts[0].parse().ok()?;
        let minute: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        (hour, minute)
    } else {
        return None;
    };

    if hour > 23 || minute > 59 {
        return None;
    }
    // Cron format: minute hour * * *
    Some(format!("{minute} {hour} * * *"))
}

pub struct SkillsLoader {
    workspace_skills: PathBuf,
    builtin_skills: Option<PathBuf>,
}

impl SkillsLoader {
    pub fn new(workspace: impl AsRef<Path>, builtin_skills_dir: Option<PathBuf>) -> Self {
        let workspace = workspace.as_ref().to_path_buf();
        let workspace_skills = workspace.join("skills");
        let loader = Self {
            workspace_skills,
            builtin_skills: builtin_skills_dir,
        };

        // Log skill discovery at construction time
        let all_skills = loader.list_skills(false);
        if all_skills.is_empty() {
            debug!("no skills found");
        } else {
            let mut total_hints = 0;
            let mut total_chars = 0;
            let names: Vec<String> = all_skills
                .iter()
                .filter_map(|s| {
                    let name = s.get("name").map(String::as_str)?;
                    total_hints += loader.get_skill_hints(name).len();
                    if let Some(content) = loader.load_skill(name) {
                        total_chars += Self::strip_frontmatter(&content).len();
                    }
                    let emoji = loader
                        .get_skill_metadata(name)
                        .and_then(|m| m.get("emoji").and_then(|v| v.as_str().map(String::from)))
                        .unwrap_or_else(|| "\u{1f527}".to_string());
                    Some(format!("{emoji} {name}"))
                })
                .collect();
            info!(
                "discovered {} skill(s): {} (~{} tokens, {} hint keywords)",
                all_skills.len(),
                names.join(", "),
                total_chars / 4,
                total_hints
            );
        }

        loader
    }

    pub fn list_skills(&self, filter_unavailable: bool) -> Vec<HashMap<String, String>> {
        let mut skills = Vec::new();

        // Workspace skills (highest priority)
        if self.workspace_skills.exists() {
            for entry in WalkDir::new(&self.workspace_skills)
                .max_depth(1)
                .follow_links(false)
                .into_iter()
                .flatten()
            {
                if entry.file_type().is_dir() && entry.path() != self.workspace_skills {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let skill_file = entry.path().join(format!("{name}.md"));
                    if skill_file.exists() {
                        skills.push({
                            let mut map = HashMap::new();
                            map.insert("name".to_string(), name.clone());
                            map.insert(
                                "path".to_string(),
                                skill_file.to_string_lossy().to_string(),
                            );
                            map.insert("source".to_string(), "workspace".to_string());
                            map
                        });
                    }
                }
            }
        }

        // Built-in skills
        if let Some(ref builtin) = self.builtin_skills
            && builtin.exists()
        {
            for entry in WalkDir::new(builtin)
                .max_depth(1)
                .follow_links(false)
                .into_iter()
                .flatten()
            {
                if entry.file_type().is_dir() && entry.path() != builtin {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let skill_file = entry.path().join(format!("{name}.md"));
                    if skill_file.exists() && !skills.iter().any(|s| s.get("name") == Some(&name)) {
                        skills.push({
                            let mut map = HashMap::new();
                            map.insert("name".to_string(), name);
                            map.insert(
                                "path".to_string(),
                                skill_file.to_string_lossy().to_string(),
                            );
                            map.insert("source".to_string(), "builtin".to_string());
                            map
                        });
                    }
                }
            }
        }

        // Filter by requirements
        if filter_unavailable {
            skills
                .into_iter()
                .filter(|s| {
                    if let Some(name) = s.get("name") {
                        let meta = self.get_skill_metadata(name);
                        Self::check_requirements(meta.as_ref())
                    } else {
                        false
                    }
                })
                .collect()
        } else {
            skills
        }
    }

    pub fn load_skill(&self, name: &str) -> Option<String> {
        // Validate name — must be a simple directory name, no path components
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name.contains("..")
            || name == "."
        {
            warn!("rejecting skill name with path components: {:?}", name);
            return None;
        }

        // Check workspace first
        let workspace_skill = self.workspace_skills.join(name).join(format!("{name}.md"));
        if let Some(content) = Self::read_skill_file(&workspace_skill) {
            debug!("loaded skill '{}'", name);
            return Some(content);
        }

        // Check built-in
        if let Some(ref builtin) = self.builtin_skills {
            let builtin_skill = builtin.join(name).join(format!("{name}.md"));
            if let Some(content) = Self::read_skill_file(&builtin_skill) {
                debug!("loaded skill '{}'", name);
                return Some(content);
            }
        }

        None
    }

    /// Read a skill file with size validation.
    fn read_skill_file(path: &Path) -> Option<String> {
        if !path.exists() {
            return None;
        }
        if let Ok(meta) = std::fs::metadata(path)
            && meta.len() > MAX_SKILL_FILE_SIZE
        {
            warn!(
                "skill file too large ({} bytes, max {}): {}",
                meta.len(),
                MAX_SKILL_FILE_SIZE,
                path.display()
            );
            return None;
        }
        std::fs::read_to_string(path).ok()
    }

    pub fn load_skills_for_context(&self, skill_names: &[String]) -> String {
        let mut parts = Vec::new();
        let mut total_chars = 0;
        for name in skill_names {
            if let Some(content) = self.load_skill(name) {
                // Security scan before injection into system prompt
                let scan = scanner::scan_skill(&content);
                if !scan.is_clean() {
                    for finding in &scan.blocked {
                        warn!(
                            "skill '{}' blocked ({}:{}): {} at line {}",
                            name,
                            finding.category,
                            finding.pattern_name,
                            finding.matched_text,
                            finding.line_number
                        );
                    }
                    continue; // Skip blocked skills entirely
                }
                for finding in &scan.warnings {
                    warn!(
                        "skill '{}' warning ({}:{}): {} at line {}",
                        name,
                        finding.category,
                        finding.pattern_name,
                        finding.matched_text,
                        finding.line_number
                    );
                }
                let stripped = Self::strip_frontmatter(&content);
                // Budget check
                if total_chars + stripped.len() > MAX_SKILL_CONTEXT_CHARS {
                    warn!(
                        "skill context budget exceeded ({} chars), skipping '{}'",
                        total_chars, name
                    );
                    break;
                }
                total_chars += stripped.len();
                parts.push(format!("### Skill: {name}\n\n{stripped}"));
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            info!("injected {} skill(s) into context", parts.len());
            metrics::gauge!("oxicrab_skills_loaded").set(parts.len() as f64);
            parts.join("\n\n---\n\n")
        }
    }

    /// Generate a compact skill summary for the system prompt.
    /// Each skill is one line: name, description, and hint keywords.
    /// Full skill content is loaded on demand when hints match the user message.
    pub fn build_skills_summary(&self) -> String {
        let skills = self.list_skills(true);
        if skills.is_empty() {
            return String::new();
        }
        let mut lines = vec!["## Available Skills (loaded on demand)".to_string()];
        for skill in &skills {
            let name = skill.get("name").cloned().unwrap_or_default();
            let hints = self.get_skill_hints(&name);
            if let Some(meta) = self.get_skill_metadata(&name) {
                let desc = meta
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let emoji = meta
                    .get("emoji")
                    .and_then(|v| v.as_str())
                    .unwrap_or("\u{1f527}");
                let schedule_note = meta
                    .get("schedule")
                    .and_then(|v| v.as_str())
                    .map(|s| format!(" [scheduled: {s}]"))
                    .unwrap_or_default();
                if hints.is_empty() {
                    lines.push(format!("- {emoji} **{name}**: {desc}{schedule_note}"));
                } else {
                    lines.push(format!(
                        "- {emoji} **{name}**: {desc}{schedule_note} (triggers: {})",
                        hints.join(", ")
                    ));
                }
            }
        }
        lines.join("\n")
    }

    fn strip_frontmatter(content: &str) -> String {
        if let Some(rest) = content.strip_prefix("---")
            && let Some(end_idx) = rest.find("\n---\n")
        {
            let after = end_idx + 5; // skip past "\n---\n"
            return rest[after..].trim().to_string();
        }
        content.to_string()
    }

    fn check_requirements(meta: Option<&Value>) -> bool {
        if let Some(meta) = meta
            && let Some(requires) = meta.get("requires")
        {
            if let Some(bins) = requires.get("bins").and_then(|v| v.as_array()) {
                for bin in bins {
                    if let Some(bin_str) = bin.as_str()
                        && which::which(bin_str).is_err()
                    {
                        return false;
                    }
                }
            }
            if let Some(env) = requires.get("env").and_then(|v| v.as_array()) {
                for env_var in env {
                    if let Some(env_str) = env_var.as_str()
                        && std::env::var(env_str).is_err()
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    pub fn get_skill_metadata(&self, name: &str) -> Option<Value> {
        let content = self.load_skill(name)?;
        let rest = content.strip_prefix("---")?;
        {
            let end_idx = rest.find("\n---")?;
            let yaml_content = rest[..end_idx].trim();
            match serde_yaml_ng::from_str::<Value>(yaml_content) {
                Ok(val) => Some(val),
                Err(e) => {
                    warn!(
                        "failed to parse skill YAML frontmatter for '{}': {}",
                        name, e
                    );
                    None
                }
            }
        }
    }

    /// Build an Aho-Corasick automaton from all skill hints for fast matching.
    /// Returns (automaton, mapping from pattern index to skill name).
    pub fn build_hint_matcher(&self) -> (AhoCorasick, Vec<String>) {
        let skills = self.list_skills(true); // only available skills
        let mut patterns = Vec::new();
        let mut skill_names = Vec::new();

        for skill in &skills {
            let name = skill.get("name").cloned().unwrap_or_default();
            let hints = self.get_skill_hints(&name);
            for hint in hints {
                patterns.push(hint.to_lowercase());
                skill_names.push(name.clone());
            }
        }

        let empty: &[&str] = &[];
        let ac = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(&patterns)
            .unwrap_or_else(|_| AhoCorasick::builder().build(empty).unwrap());
        (ac, skill_names)
    }

    /// Get hints for a skill. Uses `hints` from frontmatter, falls back to
    /// extracting keywords from name and description.
    fn get_skill_hints(&self, name: &str) -> Vec<String> {
        if let Some(meta) = self.get_skill_metadata(name) {
            // Try explicit hints first
            if let Some(hints) = meta.get("hints").and_then(|v| v.as_array()) {
                let explicit: Vec<String> = hints
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(ToString::to_string)
                    .collect();
                if !explicit.is_empty() {
                    return explicit;
                }
            }
            // Fall back to keywords from name and description
            let mut keywords = Vec::new();
            // Add name parts (split on hyphens)
            for part in name.split('-') {
                if part.len() >= 3 {
                    keywords.push(part.to_string());
                }
            }
            // Add description words (skip common stop words, keep 4+ char words)
            if let Some(desc) = meta.get("description").and_then(|v| v.as_str()) {
                let stop_words = [
                    "the", "and", "for", "from", "with", "that", "this", "into", "via",
                ];
                for word in desc.split_whitespace() {
                    let clean = word
                        .trim_matches(|c: char| !c.is_alphanumeric())
                        .to_lowercase();
                    if clean.len() >= 4 && !stop_words.contains(&clean.as_str()) {
                        keywords.push(clean);
                    }
                }
            }
            keywords.dedup();
            keywords
        } else {
            // No metadata — use name parts
            name.split('-')
                .filter(|p| p.len() >= 3)
                .map(String::from)
                .collect()
        }
    }

    /// Returns skills that have a `schedule` field, with their parsed cron expressions.
    pub fn get_scheduled_skills(&self) -> Vec<(String, Vec<String>)> {
        let skills = self.list_skills(true);
        let mut scheduled = Vec::new();
        for skill in &skills {
            let name = skill.get("name").cloned().unwrap_or_default();
            if let Some(meta) = self.get_skill_metadata(&name)
                && let Some(sched) = meta.get("schedule").and_then(|v| v.as_str())
            {
                let crons = parse_schedule(sched);
                if !crons.is_empty() {
                    scheduled.push((name, crons));
                }
            }
        }
        scheduled
    }

    /// Match an inbound message against skill hints. Returns the names of
    /// skills whose hints match.
    pub fn match_skills(
        &self,
        message: &str,
        ac: &AhoCorasick,
        skill_names: &[String],
    ) -> Vec<String> {
        let mut matched = Vec::new();
        let mut seen = HashSet::new();
        for mat in ac.find_iter(&message.to_lowercase()) {
            let name = &skill_names[mat.pattern().as_usize()];
            if seen.insert(name.clone()) {
                matched.push(name.clone());
            }
        }
        matched
    }
}

#[cfg(test)]
mod tests;
