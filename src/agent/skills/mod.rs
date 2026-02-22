use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;
use walkdir::WalkDir;

/// Maximum size for a single SKILL.md file (1 MB)
const MAX_SKILL_FILE_SIZE: u64 = 1024 * 1024;

pub struct SkillsLoader {
    workspace_skills: PathBuf,
    builtin_skills: Option<PathBuf>,
}

impl SkillsLoader {
    pub fn new(workspace: impl AsRef<Path>, builtin_skills_dir: Option<PathBuf>) -> Self {
        let workspace = workspace.as_ref().to_path_buf();
        let workspace_skills = workspace.join("skills");
        Self {
            workspace_skills,
            builtin_skills: builtin_skills_dir,
        }
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
                    let skill_file = entry.path().join("SKILL.md");
                    if skill_file.exists() {
                        let name = entry.file_name().to_string_lossy().to_string();
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
                    let skill_file = entry.path().join("SKILL.md");
                    if skill_file.exists() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if !skills.iter().any(|s| s.get("name") == Some(&name)) {
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
        let workspace_skill = self.workspace_skills.join(name).join("SKILL.md");
        if let Some(content) = Self::read_skill_file(&workspace_skill) {
            return Some(content);
        }

        // Check built-in
        if let Some(ref builtin) = self.builtin_skills {
            let builtin_skill = builtin.join(name).join("SKILL.md");
            if let Some(content) = Self::read_skill_file(&builtin_skill) {
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
        for name in skill_names {
            if let Some(content) = self.load_skill(name) {
                let stripped = Self::strip_frontmatter(&content);
                parts.push(format!("### Skill: {}\n\n{}", name, stripped));
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            parts.join("\n\n---\n\n")
        }
    }

    pub fn build_skills_summary(&self) -> String {
        fn escape_xml(s: &str) -> String {
            html_escape::encode_text(s).to_string()
        }

        let all_skills = self.list_skills(false);
        if all_skills.is_empty() {
            return String::new();
        }

        let mut lines = vec!["<skills>".to_string()];
        for s in all_skills {
            let name = s.get("name").map_or("unknown", std::string::String::as_str);
            let path = s.get("path").map_or("", std::string::String::as_str);
            // Load metadata once — avoids duplicate file reads per skill
            let meta = self.get_skill_metadata(name);
            let desc_str = meta
                .as_ref()
                .and_then(|m| m.get("description")?.as_str().map(String::from))
                .unwrap_or_else(|| name.to_string());
            let desc = escape_xml(&desc_str);
            let available = Self::check_requirements(meta.as_ref());

            let name_escaped = escape_xml(name);
            let path_escaped = escape_xml(path);
            lines.push(format!(
                "  <skill available=\"{}\">",
                available.to_string().to_lowercase()
            ));
            lines.push(format!("    <name>{}</name>", name_escaped));
            lines.push(format!("    <description>{}</description>", desc));
            lines.push(format!("    <location>{}</location>", path_escaped));

            if !available {
                let missing = Self::get_missing_requirements(meta.as_ref());
                if !missing.is_empty() {
                    lines.push(format!("    <requires>{}</requires>", escape_xml(&missing)));
                }
            }

            lines.push("  </skill>".to_string());
        }
        lines.push("</skills>".to_string());
        lines.join("\n")
    }

    fn get_missing_requirements(meta: Option<&Value>) -> String {
        let mut missing = Vec::new();
        if let Some(meta) = meta
            && let Some(requires) = meta.get("requires")
        {
            if let Some(bins) = requires.get("bins").and_then(|v| v.as_array()) {
                for bin in bins {
                    if let Some(bin_str) = bin.as_str()
                        && which::which(bin_str).is_err()
                    {
                        missing.push(format!("CLI: {}", bin_str));
                    }
                }
            }
            if let Some(env) = requires.get("env").and_then(|v| v.as_array()) {
                for env_var in env {
                    if let Some(env_str) = env_var.as_str()
                        && std::env::var(env_str).is_err()
                    {
                        missing.push(format!("ENV: {}", env_str));
                    }
                }
            }
        }
        missing.join(", ")
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

    fn get_skill_metadata(&self, name: &str) -> Option<Value> {
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

    pub fn get_always_skills(&self) -> Vec<String> {
        self.list_skills(true)
            .into_iter()
            .filter_map(|s| {
                let name = s.get("name")?;
                let meta = self.get_skill_metadata(name)?;
                if meta
                    .get("always")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
