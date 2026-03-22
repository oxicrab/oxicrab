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
        skill_dir.join("my-skill.md"),
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
    std::fs::write(skill_dir.join("my-skill.md"), content).unwrap();

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
    std::fs::write(ws_skill.join("shared-skill.md"), "workspace version").unwrap();

    let bi_skill = builtin.path().join("shared-skill");
    std::fs::create_dir_all(&bi_skill).unwrap();
    std::fs::write(bi_skill.join("shared-skill.md"), "builtin version").unwrap();

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
        skill_dir.join("my-skill.md"),
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
        skill_dir.join("test-skill.md"),
        "---\nname: test-skill\ndescription: a test\nhints:\n  - testing\n  - check\n---\n\nBody",
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
    assert!(meta.get("hints").and_then(|v| v.as_array()).is_some());
}

// ── Hint matching tests ──────────────────────────

#[test]
fn test_hint_matching_explicit_hints() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("skills").join("weather-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("weather-skill.md"),
        "---\nname: weather-skill\ndescription: Get weather forecasts\nhints:\n  - weather\n  - forecast\n  - temperature\n---\n\nWeather instructions.",
    )
    .unwrap();

    let loader = SkillsLoader::new(dir.path(), None);
    let (ac, names) = loader.build_hint_matcher();

    // Should match on hint keyword
    let matched = loader.match_skills("What's the weather today?", &ac, &names);
    assert!(
        matched.contains(&"weather-skill".to_string()),
        "should match on 'weather' hint"
    );

    // Should match on different hint
    let matched = loader.match_skills("Show me the forecast", &ac, &names);
    assert!(
        matched.contains(&"weather-skill".to_string()),
        "should match on 'forecast' hint"
    );

    // Should not match unrelated message
    let matched = loader.match_skills("Tell me a joke", &ac, &names);
    assert!(matched.is_empty(), "should not match on unrelated message");
}

#[test]
fn test_hint_matching_auto_extracted_from_name_and_description() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("skills").join("code-review");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("code-review.md"),
        "---\nname: code-review\ndescription: Review pull requests and suggest improvements\n---\n\nReview instructions.",
    )
    .unwrap();

    let loader = SkillsLoader::new(dir.path(), None);
    let (ac, names) = loader.build_hint_matcher();

    // Should match on auto-extracted name part "code"
    let matched = loader.match_skills("Can you review this code?", &ac, &names);
    assert!(
        matched.contains(&"code-review".to_string()),
        "should match on auto-extracted 'code' keyword"
    );

    // Should match on auto-extracted description word "review"
    let matched = loader.match_skills("Please review my PR", &ac, &names);
    assert!(
        matched.contains(&"code-review".to_string()),
        "should match on auto-extracted 'review' keyword"
    );
}

#[test]
fn test_hint_matching_no_hints_uses_name() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("skills").join("deploy-helper");
    std::fs::create_dir_all(&skill_dir).unwrap();
    // No hints, no description — should extract from name
    std::fs::write(
        skill_dir.join("deploy-helper.md"),
        "---\nname: deploy-helper\n---\n\nDeploy instructions.",
    )
    .unwrap();

    let loader = SkillsLoader::new(dir.path(), None);
    let (ac, names) = loader.build_hint_matcher();

    let matched = loader.match_skills("Help me deploy this app", &ac, &names);
    assert!(
        matched.contains(&"deploy-helper".to_string()),
        "should match on name-extracted 'deploy' keyword"
    );
}

#[test]
fn test_hint_matching_case_insensitive() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("skills").join("test-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("test-skill.md"),
        "---\nname: test-skill\ndescription: Run tests\nhints:\n  - testing\n  - unittest\n---\n\nTest instructions.",
    )
    .unwrap();

    let loader = SkillsLoader::new(dir.path(), None);
    let (ac, names) = loader.build_hint_matcher();

    let matched = loader.match_skills("Can you run TESTING for me?", &ac, &names);
    assert!(
        matched.contains(&"test-skill".to_string()),
        "should match case-insensitively"
    );
}

#[test]
fn test_hint_matching_deduplicates() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("skills").join("test-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("test-skill.md"),
        "---\nname: test-skill\ndescription: Test things\nhints:\n  - test\n  - testing\n---\n\nBody.",
    )
    .unwrap();

    let loader = SkillsLoader::new(dir.path(), None);
    let (ac, names) = loader.build_hint_matcher();

    // Both hints match but skill should appear only once
    let matched = loader.match_skills("run test and testing suite", &ac, &names);
    let count = matched.iter().filter(|n| *n == "test-skill").count();
    assert_eq!(
        count, 1,
        "skill should appear only once even if multiple hints match"
    );
}

#[test]
fn test_build_skill_summary_format() {
    let dir = tempfile::tempdir().unwrap();

    let skill1 = dir.path().join("skills").join("weather");
    std::fs::create_dir_all(&skill1).unwrap();
    std::fs::write(
        skill1.join("weather.md"),
        "---\nname: weather\ndescription: Get weather forecasts\nhints:\n  - weather\n  - forecast\n---\n\nBody.",
    )
    .unwrap();

    let skill2 = dir.path().join("skills").join("deploy");
    std::fs::create_dir_all(&skill2).unwrap();
    std::fs::write(
        skill2.join("deploy.md"),
        "---\nname: deploy\ndescription: Deploy applications\n---\n\nBody.",
    )
    .unwrap();

    let loader = SkillsLoader::new(dir.path(), None);
    let summary = loader.build_skills_summary();

    assert!(summary.contains("## Available Skills (loaded on demand)"));
    assert!(summary.contains("**weather**"));
    assert!(summary.contains("Get weather forecasts"));
    assert!(summary.contains("triggers: weather, forecast"));
    assert!(summary.contains("**deploy**"));
    assert!(summary.contains("Deploy applications"));
    // Default emoji should be present
    assert!(
        summary.contains("\u{1f527}"),
        "default emoji should appear in summary"
    );
}

#[test]
fn test_build_skill_summary_with_emoji_and_schedule() {
    let dir = tempfile::tempdir().unwrap();

    let skill1 = dir.path().join("skills").join("briefing");
    std::fs::create_dir_all(&skill1).unwrap();
    std::fs::write(
        skill1.join("briefing.md"),
        "---\nname: briefing\ndescription: Daily briefing\nemoji: \u{1f4cb}\nschedule: \"7am, 5pm\"\nhints:\n  - briefing\n---\n\nBody.",
    )
    .unwrap();

    let loader = SkillsLoader::new(dir.path(), None);
    let summary = loader.build_skills_summary();

    assert!(
        summary.contains("\u{1f4cb}"),
        "custom emoji should appear in summary"
    );
    assert!(
        summary.contains("[scheduled: 7am, 5pm]"),
        "schedule note should appear in summary"
    );
}

#[test]
fn test_load_skills_budget_enforcement() {
    let dir = tempfile::tempdir().unwrap();

    // Create a skill with content that is exactly at budget
    let big_content = "x".repeat(MAX_SKILL_CONTEXT_CHARS + 1);
    let skill1 = dir.path().join("skills").join("big-skill");
    std::fs::create_dir_all(&skill1).unwrap();
    std::fs::write(
        skill1.join("big-skill.md"),
        format!("---\nname: big-skill\n---\n\n{big_content}"),
    )
    .unwrap();

    let loader = SkillsLoader::new(dir.path(), None);
    let context = loader.load_skills_for_context(&["big-skill".to_string()]);
    // The single skill exceeds budget, so it should be skipped
    assert!(
        context.is_empty(),
        "skill exceeding budget should be skipped"
    );
}

#[test]
fn test_load_skills_budget_stops_at_limit() {
    let dir = tempfile::tempdir().unwrap();

    // Create two skills, first one fits, second would exceed budget
    let body_size = MAX_SKILL_CONTEXT_CHARS / 2 + 100;
    let body = "y".repeat(body_size);

    let skill1 = dir.path().join("skills").join("skill-a");
    std::fs::create_dir_all(&skill1).unwrap();
    std::fs::write(
        skill1.join("skill-a.md"),
        format!("---\nname: skill-a\n---\n\n{body}"),
    )
    .unwrap();

    let skill2 = dir.path().join("skills").join("skill-b");
    std::fs::create_dir_all(&skill2).unwrap();
    std::fs::write(
        skill2.join("skill-b.md"),
        format!("---\nname: skill-b\n---\n\n{body}"),
    )
    .unwrap();

    let skills_loader = SkillsLoader::new(dir.path(), None);
    let result =
        skills_loader.load_skills_for_context(&["skill-a".to_string(), "skill-b".to_string()]);
    // First skill should be included, second should be skipped
    assert!(result.contains("skill-a"), "first skill should be loaded");
    assert!(
        !result.contains("skill-b"),
        "second skill should be skipped due to budget"
    );
}

// ── Schedule parsing tests ──────────────────────────

#[test]
fn test_parse_schedule_single_time() {
    let crons = parse_schedule("7am");
    assert_eq!(crons, vec!["0 7 * * *"]);
}

#[test]
fn test_parse_schedule_multiple_times() {
    let crons = parse_schedule("9am, 1pm, 5pm");
    assert_eq!(crons, vec!["0 9 * * *", "0 13 * * *", "0 17 * * *"]);
}

#[test]
fn test_parse_schedule_with_minutes() {
    let crons = parse_schedule("7:30am, 5:45pm");
    assert_eq!(crons, vec!["30 7 * * *", "45 17 * * *"]);
}

#[test]
fn test_parse_schedule_24h_format() {
    let crons = parse_schedule("13:00, 17:30");
    assert_eq!(crons, vec!["0 13 * * *", "30 17 * * *"]);
}

#[test]
fn test_parse_schedule_noon_midnight() {
    let crons = parse_schedule("12am, 12pm");
    assert_eq!(crons, vec!["0 0 * * *", "0 12 * * *"]);
}

#[test]
fn test_parse_schedule_invalid() {
    let crons = parse_schedule("invalid");
    assert!(crons.is_empty());
}

#[test]
fn test_get_scheduled_skills() {
    let dir = tempfile::tempdir().unwrap();

    let skill1 = dir.path().join("skills").join("briefing");
    std::fs::create_dir_all(&skill1).unwrap();
    std::fs::write(
        skill1.join("briefing.md"),
        "---\nname: briefing\ndescription: Daily briefing\nschedule: \"7am, 5pm\"\nhints:\n  - briefing\n---\n\nBody.",
    )
    .unwrap();

    let skill2 = dir.path().join("skills").join("weather");
    std::fs::create_dir_all(&skill2).unwrap();
    std::fs::write(
        skill2.join("weather.md"),
        "---\nname: weather\ndescription: Weather check\nhints:\n  - weather\n---\n\nBody.",
    )
    .unwrap();

    let loader = SkillsLoader::new(dir.path(), None);
    let scheduled = loader.get_scheduled_skills();

    assert_eq!(scheduled.len(), 1, "only one skill has a schedule");
    assert_eq!(scheduled[0].0, "briefing");
    assert_eq!(scheduled[0].1, vec!["0 7 * * *", "0 17 * * *"]);
}
