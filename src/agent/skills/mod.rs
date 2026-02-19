use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::debug;
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
        if let Some(ref builtin) = self.builtin_skills
            && builtin.exists()
        {
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
            let desc = escape_xml(&self.get_skill_description(name));
            let meta = self.get_skill_metadata(name);
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

    fn get_skill_description(&self, name: &str) -> String {
        let meta = self.get_skill_metadata(name);
        meta.and_then(|m| {
            m.get("description")
                .and_then(|v| v.as_str().map(std::string::ToString::to_string))
        })
        .unwrap_or_else(|| name.to_string())
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
                    debug!("Failed to parse skill YAML frontmatter: {}", e);
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
mod tests {
    use super::*;

    #[test]
    fn test_strip_frontmatter_with_yaml() {
        let content = "---\nname: test\ndescription: a test skill\n---\n\nSkill body here.";
        let result = SkillsLoader::strip_frontmatter(content);
        assert_eq!(result, "Skill body here.");
    }

    #[test]
    fn test_strip_frontmatter_no_frontmatter() {
        let content = "Just regular content without frontmatter.";
        let result = SkillsLoader::strip_frontmatter(content);
        assert_eq!(result, content);
    }

    #[test]
    fn test_strip_frontmatter_incomplete() {
        let content = "---\nname: test\nno closing delimiter";
        let result = SkillsLoader::strip_frontmatter(content);
        assert_eq!(result, content);
    }

    #[test]
    fn test_check_requirements_no_meta() {
        assert!(SkillsLoader::check_requirements(None));
    }

    #[test]
    fn test_check_requirements_no_requires() {
        let meta = serde_json::json!({"name": "test"});
        assert!(SkillsLoader::check_requirements(Some(&meta)));
    }

    #[test]
    fn test_check_requirements_existing_binary() {
        // "ls" should exist on any system
        let meta = serde_json::json!({"requires": {"bins": ["ls"]}});
        assert!(SkillsLoader::check_requirements(Some(&meta)));
    }

    #[test]
    fn test_check_requirements_missing_binary() {
        let meta =
            serde_json::json!({"requires": {"bins": ["totally_nonexistent_binary_xyz_12345"]}});
        assert!(!SkillsLoader::check_requirements(Some(&meta)));
    }

    #[test]
    fn test_get_missing_requirements_reports_missing_binary() {
        let meta =
            serde_json::json!({"requires": {"bins": ["totally_nonexistent_binary_xyz_12345"]}});
        let missing = SkillsLoader::get_missing_requirements(Some(&meta));
        assert!(missing.contains("CLI: totally_nonexistent_binary_xyz_12345"));
    }

    #[test]
    fn test_get_missing_requirements_none() {
        let missing = SkillsLoader::get_missing_requirements(None);
        assert!(missing.is_empty());
    }

    #[test]
    fn test_list_skills_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let loader = SkillsLoader::new(dir.path(), None);
        let skills = loader.list_skills(false);
        assert!(skills.is_empty());
    }

    #[test]
    fn test_list_skills_finds_workspace_skills() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: test\n---\n\nContent",
        )
        .unwrap();

        let loader = SkillsLoader::new(dir.path(), None);
        let skills = loader.list_skills(false);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].get("name").unwrap(), "my-skill");
        assert_eq!(skills[0].get("source").unwrap(), "workspace");
    }

    #[test]
    fn test_load_skill_from_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let content = "---\nname: my-skill\n---\n\nSkill content here.";
        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();

        let loader = SkillsLoader::new(dir.path(), None);
        let result = loader.load_skill("my-skill");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), content);
    }

    #[test]
    fn test_load_skill_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let loader = SkillsLoader::new(dir.path(), None);
        assert!(loader.load_skill("nonexistent").is_none());
    }

    #[test]
    fn test_workspace_skill_overrides_builtin() {
        let workspace = tempfile::tempdir().unwrap();
        let builtin = tempfile::tempdir().unwrap();

        // Create same skill in both
        let ws_skill = workspace.path().join("skills").join("shared-skill");
        std::fs::create_dir_all(&ws_skill).unwrap();
        std::fs::write(ws_skill.join("SKILL.md"), "workspace version").unwrap();

        let bi_skill = builtin.path().join("shared-skill");
        std::fs::create_dir_all(&bi_skill).unwrap();
        std::fs::write(bi_skill.join("SKILL.md"), "builtin version").unwrap();

        let loader = SkillsLoader::new(workspace.path(), Some(builtin.path().to_path_buf()));
        let skills = loader.list_skills(false);
        // Should only have one entry (workspace takes priority)
        let matching: Vec<_> = skills
            .iter()
            .filter(|s| s.get("name") == Some(&"shared-skill".to_string()))
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].get("source").unwrap(), "workspace");
    }

    #[test]
    fn test_load_skills_for_context_empty() {
        let dir = tempfile::tempdir().unwrap();
        let loader = SkillsLoader::new(dir.path(), None);
        let context = loader.load_skills_for_context(&[]);
        assert!(context.is_empty());
    }

    #[test]
    fn test_load_skills_for_context_strips_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\n---\n\nSkill body only.",
        )
        .unwrap();

        let loader = SkillsLoader::new(dir.path(), None);
        let context = loader.load_skills_for_context(&["my-skill".to_string()]);
        assert!(context.contains("Skill body only."));
        assert!(!context.contains("name: my-skill"));
    }

    #[test]
    fn test_get_skill_metadata_parses_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: test-skill\ndescription: a test\nalways: true\n---\n\nBody",
        )
        .unwrap();

        let loader = SkillsLoader::new(dir.path(), None);
        let meta = loader.get_skill_metadata("test-skill");
        assert!(meta.is_some());
        let meta = meta.unwrap();
        assert_eq!(
            meta.get("name").and_then(|v| v.as_str()),
            Some("test-skill")
        );
        assert_eq!(
            meta.get("always").and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn test_get_always_skills() {
        let dir = tempfile::tempdir().unwrap();

        // Create an always-on skill
        let skill1 = dir.path().join("skills").join("always-skill");
        std::fs::create_dir_all(&skill1).unwrap();
        std::fs::write(
            skill1.join("SKILL.md"),
            "---\nname: always-skill\nalways: true\n---\n\nAlways on.",
        )
        .unwrap();

        // Create a non-always skill
        let skill2 = dir.path().join("skills").join("normal-skill");
        std::fs::create_dir_all(&skill2).unwrap();
        std::fs::write(
            skill2.join("SKILL.md"),
            "---\nname: normal-skill\nalways: false\n---\n\nNot always.",
        )
        .unwrap();

        let loader = SkillsLoader::new(dir.path(), None);
        let always = loader.get_always_skills();
        assert!(always.contains(&"always-skill".to_string()));
        assert!(!always.contains(&"normal-skill".to_string()));
    }
}
