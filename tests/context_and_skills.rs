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
        skill_dir.join("my-skill.md"),
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
        skill_dir.join("parser-test.md"),
        "---\nname: parser-test\ndescription: Test parsing\nhints:\n  - parse\n  - test\n---\n\nBody content.",
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
        builtin_skill_dir.join("shared-skill.md"),
        "---\nname: shared-skill\ndescription: Builtin version\n---\n\nBuiltin body.",
    )
    .expect("write test file");

    // Create workspace skill with same name (should override)
    let ws_skills = tmp.path().join("workspace").join("skills");
    let ws_skill_dir = ws_skills.join("shared-skill");
    std::fs::create_dir_all(&ws_skill_dir).expect("create test dir");
    std::fs::write(
        ws_skill_dir.join("shared-skill.md"),
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
fn test_skills_hint_matching() {
    let tmp = TempDir::new().expect("create temp dir");
    let skills_dir = tmp.path().join("skills");

    // Create a skill with explicit hints
    let hint_dir = skills_dir.join("weather-check");
    std::fs::create_dir_all(&hint_dir).expect("create test dir");
    std::fs::write(
        hint_dir.join("weather-check.md"),
        "---\nname: weather-check\ndescription: Check weather conditions\nhints:\n  - weather\n  - forecast\n  - rain\n---\n\nWeather body.",
    )
    .expect("write test file");

    // Create a skill without explicit hints (auto-extract from name/description)
    let normal_dir = skills_dir.join("code-review");
    std::fs::create_dir_all(&normal_dir).expect("create test dir");
    std::fs::write(
        normal_dir.join("code-review.md"),
        "---\nname: code-review\ndescription: Review pull requests\n---\n\nReview body.",
    )
    .expect("write test file");

    let loader = SkillsLoader::new(tmp.path(), None);
    let (ac, names) = loader.build_hint_matcher();

    // Explicit hint match
    let matched = loader.match_skills("What's the weather like?", &ac, &names);
    assert!(
        matched.contains(&"weather-check".to_string()),
        "Should match weather-check on 'weather' hint"
    );
    assert!(
        !matched.contains(&"code-review".to_string()),
        "Should not match code-review on weather message"
    );

    // Auto-extracted hint match
    let matched = loader.match_skills("Please review my code", &ac, &names);
    assert!(
        matched.contains(&"code-review".to_string()),
        "Should match code-review on auto-extracted 'review' or 'code' keyword"
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
        dep_dir.join("needs-binary.md"),
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

#[tokio::test]
async fn test_context_includes_skill_summary() {
    let tmp = TempDir::new().expect("create temp dir");
    let skill_dir = tmp.path().join("skills").join("my-skill");
    std::fs::create_dir_all(&skill_dir).expect("create test dir");
    std::fs::write(
        skill_dir.join("my-skill.md"),
        "---\nname: my-skill\ndescription: A useful skill\nhints:\n  - useful\n  - skill\n---\n\nSkill body.",
    )
    .expect("write test file");

    let mut builder = ContextBuilder::new(tmp.path()).expect("create context builder");
    let prompt = builder
        .build_system_prompt(None, None)
        .expect("build system prompt");

    assert!(
        prompt.contains("Available Skills"),
        "system prompt should include skill summary"
    );
    assert!(
        prompt.contains("my-skill"),
        "skill summary should contain skill name"
    );
}

#[tokio::test]
async fn test_context_loads_matching_skills() {
    let tmp = TempDir::new().expect("create temp dir");
    let skill_dir = tmp.path().join("skills").join("weather");
    std::fs::create_dir_all(&skill_dir).expect("create test dir");
    std::fs::write(
        skill_dir.join("weather.md"),
        "---\nname: weather\ndescription: Get weather data\nhints:\n  - weather\n  - forecast\n---\n\nWeather skill body content here.",
    )
    .expect("write test file");

    let mut builder = ContextBuilder::new(tmp.path()).expect("create context builder");

    // Query that matches hint should load full skill content
    let prompt = builder
        .build_system_prompt(None, Some("What's the weather today?"))
        .expect("build system prompt");

    assert!(
        prompt.contains("Weather skill body content here"),
        "system prompt should include full skill content when hint matches"
    );
}
