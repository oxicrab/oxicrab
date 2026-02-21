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
    let meta = serde_json::json!({"requires": {"bins": ["totally_nonexistent_binary_xyz_12345"]}});
    assert!(!SkillsLoader::check_requirements(Some(&meta)));
}

#[test]
fn test_get_missing_requirements_reports_missing_binary() {
    let meta = serde_json::json!({"requires": {"bins": ["totally_nonexistent_binary_xyz_12345"]}});
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
