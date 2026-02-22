use super::create_workspace_templates;

#[test]
fn test_create_workspace_templates() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    create_workspace_templates(&workspace).unwrap();

    // Core template files should exist
    assert!(workspace.join("USER.md").exists());
    assert!(workspace.join("AGENTS.md").exists());
    assert!(workspace.join("TOOLS.md").exists());
    assert!(workspace.join("memory").join("MEMORY.md").exists());
}

#[test]
fn test_create_workspace_templates_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    create_workspace_templates(&workspace).unwrap();

    // Write custom content to USER.md
    let user_path = workspace.join("USER.md");
    std::fs::write(&user_path, "custom content").unwrap();

    // Second run should not overwrite
    create_workspace_templates(&workspace).unwrap();

    let content = std::fs::read_to_string(&user_path).unwrap();
    assert_eq!(content, "custom content");
}

#[test]
fn test_create_workspace_templates_content() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    create_workspace_templates(&workspace).unwrap();

    let agents = std::fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
    assert!(agents.contains("oxicrab"));
    assert!(agents.contains("Personality"));

    let tools = std::fs::read_to_string(workspace.join("TOOLS.md")).unwrap();
    assert!(tools.contains("Tool Notes"));

    let memory = std::fs::read_to_string(workspace.join("memory").join("MEMORY.md")).unwrap();
    assert!(memory.contains("Long-term Memory"));
}
