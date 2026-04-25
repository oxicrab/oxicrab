use super::*;
use tempfile::tempdir;

#[test]
fn skill_path_resolves_flat_layout() {
    let dir = tempdir().unwrap();
    let skills = dir.path().to_path_buf();
    std::fs::write(skills.join("demo.md"), "x").unwrap();
    assert!(skill_file_path(&skills, "demo").is_some());
}

#[test]
fn skill_path_resolves_folder_layout() {
    let dir = tempdir().unwrap();
    let skills = dir.path().to_path_buf();
    std::fs::create_dir_all(skills.join("demo")).unwrap();
    std::fs::write(skills.join("demo").join("SKILL.md"), "x").unwrap();
    assert!(skill_file_path(&skills, "demo").is_some());
}

#[test]
fn changelog_sidecar_path() {
    let p = std::path::PathBuf::from("/x/skills/demo.md");
    assert_eq!(changelog_path(&p).file_name().unwrap(), "demo-CHANGELOG.md");
}

#[test]
fn parse_json_handles_code_fence() {
    let text = "```json\n{\"should_patch\": true, \"confidence\": 0.85, \"reason\": \"foo\"}\n```";
    let parsed: RoundOneResponse = parse_json(text).unwrap();
    assert!(parsed.should_patch);
    assert!((parsed.confidence - 0.85).abs() < 0.001);
}

#[test]
fn parse_json_handles_leading_prose() {
    let text = "Sure! Here is the JSON:\n{\"should_patch\": false, \"confidence\": 0.2, \"reason\": \"r\"}\nHope that helps.";
    let parsed: RoundOneResponse = parse_json(text).unwrap();
    assert!(!parsed.should_patch);
}

#[test]
fn apply_patch_writes_atomically_and_appends_changelog() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("demo.md");
    std::fs::write(&path, "old body").unwrap();

    apply_patch(&path, "new body", "tightened wording", "1.1.0").unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(after, "new body");

    let cl = std::fs::read_to_string(changelog_path(&path)).unwrap();
    assert!(cl.contains("v1.1.0"));
    assert!(cl.contains("tightened wording"));

    apply_patch(&path, "newer body", "second pass", "1.2.0").unwrap();
    let cl = std::fs::read_to_string(changelog_path(&path)).unwrap();
    assert!(cl.contains("v1.1.0"));
    assert!(cl.contains("v1.2.0"));
}

#[test]
fn excerpt_caps_at_max() {
    let s = "abcdefghij";
    assert_eq!(excerpt(s, 5), "abcde…");
    assert_eq!(excerpt(s, 100), "abcdefghij");
}
