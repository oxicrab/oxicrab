use super::*;

#[test]
fn test_extract_from_todoist_json() {
    let result = r#"[{"id":"12345","content":"Call Sun Logistics","due":{"date":"2026-02-25"},"priority":4}]"#;
    let entities = DiscourseRegister::extract_from_tool_result("todoist", result, 1);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].entity_type, "task");
    assert_eq!(entities[0].entity_id, "12345");
    assert_eq!(entities[0].label, "Call Sun Logistics");
}

#[test]
fn test_extract_from_github_json() {
    let result = r#"{"issues":[{"number":42,"title":"Fix login bug","state":"open"}]}"#;
    let entities = DiscourseRegister::extract_from_tool_result("github", result, 2);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].entity_type, "issue");
    assert_eq!(entities[0].entity_id, "42");
    assert_eq!(entities[0].label, "Fix login bug");
}

#[test]
fn test_deduplication() {
    let mut register = DiscourseRegister::default();
    let e1 = vec![DiscourseEntity {
        entity_type: "task".into(),
        entity_id: "123".into(),
        label: "Old label".into(),
        source_tool: "todoist".into(),
        last_turn: 1,
    }];
    register.register(e1);
    assert_eq!(register.entities.len(), 1);

    let e2 = vec![DiscourseEntity {
        entity_type: "task".into(),
        entity_id: "123".into(),
        label: "Updated label".into(),
        source_tool: "todoist".into(),
        last_turn: 3,
    }];
    register.register(e2);
    assert_eq!(register.entities.len(), 1);
    assert_eq!(register.entities[0].label, "Updated label");
    assert_eq!(register.entities[0].last_turn, 3);
}

#[test]
fn test_pruning_by_age() {
    let mut register = DiscourseRegister {
        turn: 15,
        ..Default::default()
    };
    register.entities.push(DiscourseEntity {
        entity_type: "task".into(),
        entity_id: "old".into(),
        label: "Old task".into(),
        source_tool: "todoist".into(),
        last_turn: 1, // 14 turns ago — beyond MAX_AGE_TURNS
    });
    register.entities.push(DiscourseEntity {
        entity_type: "task".into(),
        entity_id: "recent".into(),
        label: "Recent task".into(),
        source_tool: "todoist".into(),
        last_turn: 10, // 5 turns ago — within limit
    });
    register.prune();
    assert_eq!(register.entities.len(), 1);
    assert_eq!(register.entities[0].entity_id, "recent");
}

#[test]
fn test_context_string() {
    let mut register = DiscourseRegister::default();
    register.entities.push(DiscourseEntity {
        entity_type: "task".into(),
        entity_id: "123".into(),
        label: "Call Sun Logistics".into(),
        source_tool: "todoist".into(),
        last_turn: 1,
    });
    let ctx = register.to_context_string().unwrap();
    assert!(ctx.contains("task [123]: Call Sun Logistics"));
    assert!(ctx.contains("todoist"));
}

#[test]
fn test_empty_register_no_context() {
    let register = DiscourseRegister::default();
    assert!(register.to_context_string().is_none());
}

#[test]
fn test_extract_from_wrapper_object() {
    let result = r#"{"tasks":[{"id":"1","content":"Task A"},{"id":"2","content":"Task B"}]}"#;
    let entities = DiscourseRegister::extract_from_tool_result("todoist", result, 1);
    assert_eq!(entities.len(), 2);
}

#[test]
fn test_truncate_label_short() {
    assert_eq!(truncate_label("short"), "short");
}

#[test]
fn test_truncate_label_long() {
    let long = "a".repeat(100);
    let truncated = truncate_label(&long);
    assert!(truncated.len() <= 80);
    assert!(truncated.ends_with("..."));
}

#[test]
fn test_session_metadata_roundtrip() {
    let mut register = DiscourseRegister {
        turn: 5,
        ..Default::default()
    };
    register.entities.push(DiscourseEntity {
        entity_type: "task".into(),
        entity_id: "42".into(),
        label: "Test task".into(),
        source_tool: "todoist".into(),
        last_turn: 5,
    });

    let mut metadata = HashMap::new();
    register.to_session_metadata(&mut metadata);

    let loaded = DiscourseRegister::from_session_metadata(&metadata);
    assert_eq!(loaded.turn, 5);
    assert_eq!(loaded.entities.len(), 1);
    assert_eq!(loaded.entities[0].entity_id, "42");
}
