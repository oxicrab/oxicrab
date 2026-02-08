use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

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
        if let Some(ref builtin) = self.builtin_skills {
            if builtin.exists() {
                for entry in WalkDir::new(builtin).max_depth(1).into_iter().flatten() {
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
        }

        // Filter by requirements
        if filter_unavailable {
            skills
                .into_iter()
                .filter(|s| {
                    if let Some(name) = s.get("name") {
                        let meta = self.get_skill_metadata(name);
                        self.check_requirements(meta.as_ref())
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
        // Check workspace first
        let workspace_skill = self.workspace_skills.join(name).join("SKILL.md");
        if workspace_skill.exists() {
            return std::fs::read_to_string(&workspace_skill).ok();
        }

        // Check built-in
        if let Some(ref builtin) = self.builtin_skills {
            let builtin_skill = builtin.join(name).join("SKILL.md");
            if builtin_skill.exists() {
                return std::fs::read_to_string(&builtin_skill).ok();
            }
        }

        None
    }

    pub fn load_skills_for_context(&self, skill_names: &[String]) -> String {
        let mut parts = Vec::new();
        for name in skill_names {
            if let Some(content) = self.load_skill(name) {
                let stripped = self.strip_frontmatter(&content);
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
        let all_skills = self.list_skills(false);
        if all_skills.is_empty() {
            return String::new();
        }

        fn escape_xml(s: &str) -> String {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        }

        let mut lines = vec!["<skills>".to_string()];
        for s in all_skills {
            let name = s.get("name").map(|s| s.as_str()).unwrap_or("unknown");
            let path = s.get("path").map(|s| s.as_str()).unwrap_or("");
            let desc = escape_xml(&self.get_skill_description(name));
            let meta = self.get_skill_metadata(name);
            let available = self.check_requirements(meta.as_ref());

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
                let missing = self.get_missing_requirements(meta.as_ref());
                if !missing.is_empty() {
                    lines.push(format!("    <requires>{}</requires>", escape_xml(&missing)));
                }
            }

            lines.push("  </skill>".to_string());
        }
        lines.push("</skills>".to_string());
        lines.join("\n")
    }

    fn get_missing_requirements(&self, meta: Option<&Value>) -> String {
        let mut missing = Vec::new();
        if let Some(meta) = meta {
            if let Some(requires) = meta.get("requires") {
                if let Some(bins) = requires.get("bins").and_then(|v| v.as_array()) {
                    for bin in bins {
                        if let Some(bin_str) = bin.as_str() {
                            if which::which(bin_str).is_err() {
                                missing.push(format!("CLI: {}", bin_str));
                            }
                        }
                    }
                }
                if let Some(env) = requires.get("env").and_then(|v| v.as_array()) {
                    for env_var in env {
                        if let Some(env_str) = env_var.as_str() {
                            if std::env::var(env_str).is_err() {
                                missing.push(format!("ENV: {}", env_str));
                            }
                        }
                    }
                }
            }
        }
        missing.join(", ")
    }

    fn get_skill_description(&self, name: &str) -> String {
        let meta = self.get_skill_metadata(name);
        meta.and_then(|m| {
            m.get("description")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
        })
        .unwrap_or_else(|| name.to_string())
    }

    fn strip_frontmatter(&self, content: &str) -> String {
        if let Some(rest) = content.strip_prefix("---") {
            if let Some(end_idx) = rest.find("\n---\n") {
                let after = end_idx + 5; // skip past "\n---\n"
                return rest[after..].trim().to_string();
            }
        }
        content.to_string()
    }

    fn check_requirements(&self, meta: Option<&Value>) -> bool {
        if let Some(meta) = meta {
            if let Some(requires) = meta.get("requires") {
                if let Some(bins) = requires.get("bins").and_then(|v| v.as_array()) {
                    for bin in bins {
                        if let Some(bin_str) = bin.as_str() {
                            if which::which(bin_str).is_err() {
                                return false;
                            }
                        }
                    }
                }
                if let Some(env) = requires.get("env").and_then(|v| v.as_array()) {
                    for env_var in env {
                        if let Some(env_str) = env_var.as_str() {
                            if std::env::var(env_str).is_err() {
                                return false;
                            }
                        }
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
            match serde_yaml::from_str::<Value>(yaml_content) {
                Ok(val) => Some(val),
                Err(e) => {
                    tracing::debug!("Failed to parse skill YAML frontmatter: {}", e);
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
                    .and_then(|v| v.as_bool())
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
