use oxicrab::agent::context::ContextBuilder;
use oxicrab::agent::skills::SkillsLoader;
use tempfile::TempDir;

#[test]
fn test_skills_loading_from_disk() {
    let tmp = TempDir::new().expect("create temp dir");
    let skills_dir = tmp.path().join("skills");
    let skill_dir = skills_dir.join("my-skill");
    std::fs::create_dir_all(&skill_dir).expect("create test dir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: my-skill\ndescription: A test skill\n---\n\nSkill instructions here.",
    )
    .expect("write test file");

    let loader = SkillsLoader::new(tmp.path(), None);
    let skills = loader.list_skills(false);

    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].get("name").unwrap(), "my-skill");
    assert_eq!(skills[0].get("source").unwrap(), "workspace");
}

#[test]
fn test_skills_frontmatter_parsing() {
    let tmp = TempDir::new().expect("create temp dir");
    let skills_dir = tmp.path().join("skills");
    let skill_dir = skills_dir.join("parser-test");
    std::fs::create_dir_all(&skill_dir).expect("create test dir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: parser-test\ndescription: Test parsing\nalways: true\n---\n\nBody content.",
    )
    .expect("write test file");

    let loader = SkillsLoader::new(tmp.path(), None);
    let content = loader.load_skill("parser-test");
    assert!(content.is_some());
    let content = content.unwrap();
    assert!(content.contains("description: Test parsing"));
    assert!(content.contains("Body content."));
}

#[test]
fn test_skills_workspace_overrides_builtin() {
    let tmp = TempDir::new().expect("create temp dir");

    // Create builtin skill
    let builtin_dir = tmp.path().join("builtin");
    let builtin_skill_dir = builtin_dir.join("shared-skill");
    std::fs::create_dir_all(&builtin_skill_dir).expect("create test dir");
    std::fs::write(
        builtin_skill_dir.join("SKILL.md"),
        "---\nname: shared-skill\ndescription: Builtin version\n---\n\nBuiltin body.",
    )
    .expect("write test file");

    // Create workspace skill with same name (should override)
    let ws_skills = tmp.path().join("workspace").join("skills");
    let ws_skill_dir = ws_skills.join("shared-skill");
    std::fs::create_dir_all(&ws_skill_dir).expect("create test dir");
    std::fs::write(
        ws_skill_dir.join("SKILL.md"),
        "---\nname: shared-skill\ndescription: Workspace version\n---\n\nWorkspace body.",
    )
    .expect("write test file");

    let workspace = tmp.path().join("workspace");
    let loader = SkillsLoader::new(&workspace, Some(builtin_dir));
    let skills = loader.list_skills(false);

    // Should only have 1 skill (workspace overrides builtin)
    let shared: Vec<_> = skills
        .iter()
        .filter(|s| s.get("name") == Some(&"shared-skill".to_string()))
        .collect();
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].get("source").unwrap(), "workspace");

    // Loading should return workspace version
    let content = loader.load_skill("shared-skill").unwrap();
    assert!(content.contains("Workspace body."));
}

#[test]
fn test_skills_always_include() {
    let tmp = TempDir::new().expect("create temp dir");
    let skills_dir = tmp.path().join("skills");

    // Create an always-include skill
    let always_dir = skills_dir.join("always-on");
    std::fs::create_dir_all(&always_dir).expect("create test dir");
    std::fs::write(
        always_dir.join("SKILL.md"),
        "---\nname: always-on\ndescription: Always included\nalways: true\n---\n\nAlways body.",
    )
    .expect("write test file");

    // Create a normal skill
    let normal_dir = skills_dir.join("normal");
    std::fs::create_dir_all(&normal_dir).expect("create test dir");
    std::fs::write(
        normal_dir.join("SKILL.md"),
        "---\nname: normal\ndescription: Normal skill\n---\n\nNormal body.",
    )
    .expect("write test file");

    let loader = SkillsLoader::new(tmp.path(), None);
    let always = loader.get_always_skills();

    assert!(
        always.contains(&"always-on".to_string()),
        "Should include always-on skill"
    );
    assert!(
        !always.contains(&"normal".to_string()),
        "Should not include normal skill"
    );
}

#[test]
fn test_skills_dependency_check_filters() {
    let tmp = TempDir::new().expect("create temp dir");
    let skills_dir = tmp.path().join("skills");

    // Create a skill that requires a nonexistent binary
    let dep_dir = skills_dir.join("needs-binary");
    std::fs::create_dir_all(&dep_dir).expect("create test dir");
    std::fs::write(
        dep_dir.join("SKILL.md"),
        "---\nname: needs-binary\ndescription: Needs missing binary\nrequires:\n  bins:\n    - nonexistent_binary_xyz_123\n---\n\nBody.",
    )
    .expect("write test file");

    let loader = SkillsLoader::new(tmp.path(), None);

    // Without filtering: should show the skill
    let all = loader.list_skills(false);
    assert_eq!(all.len(), 1);

    // With filtering: should hide the skill (missing dep)
    let available = loader.list_skills(true);
    assert_eq!(
        available.len(),
        0,
        "Skill with missing deps should be filtered out"
    );
}

#[tokio::test]
async fn test_context_includes_agents_md() {
    let tmp = TempDir::new().expect("create temp dir");
    std::fs::write(
        tmp.path().join("AGENTS.md"),
        "# Custom Agent\n\nYou are a specialized assistant.",
    )
    .expect("write test file");

    let mut builder = ContextBuilder::new(tmp.path()).expect("create context builder");
    let prompt = builder
        .build_system_prompt(None, None)
        .expect("build system prompt");

    assert!(
        prompt.contains("specialized assistant"),
        "System prompt should include AGENTS.md content: {}",
        &prompt[..200.min(prompt.len())]
    );
}

#[tokio::test]
async fn test_context_includes_bootstrap_files() {
    let tmp = TempDir::new().expect("create temp dir");
    std::fs::write(tmp.path().join("USER.md"), "User name: Alice").expect("write test file");
    std::fs::write(
        tmp.path().join("TOOLS.md"),
        "Custom tool instructions here.",
    )
    .expect("write test file");

    let mut builder = ContextBuilder::new(tmp.path()).expect("create context builder");
    let prompt = builder
        .build_system_prompt(None, None)
        .expect("build system prompt");

    assert!(
        prompt.contains("User name: Alice"),
        "System prompt should include USER.md"
    );
    assert!(
        prompt.contains("Custom tool instructions"),
        "System prompt should include TOOLS.md"
    );
}

#[tokio::test]
async fn test_context_bootstrap_file_refresh() {
    let tmp = TempDir::new().expect("create temp dir");
    std::fs::write(tmp.path().join("USER.md"), "Version 1").expect("write test file");

    let mut builder = ContextBuilder::new(tmp.path()).expect("create context builder");

    // First call caches
    let prompt1 = builder
        .build_system_prompt(None, None)
        .expect("build system prompt");
    assert!(prompt1.contains("Version 1"));

    // Modify file — mtime resolution is 1s on many filesystems
    std::thread::sleep(std::time::Duration::from_secs(1));
    std::fs::write(tmp.path().join("USER.md"), "Version 2").expect("write test file");

    // Second call should detect the change
    let prompt2 = builder
        .build_system_prompt(None, None)
        .expect("build system prompt");
    assert!(
        prompt2.contains("Version 2"),
        "Should pick up updated USER.md"
    );
}
