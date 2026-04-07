use super::*;

#[test]
fn test_valid_collection_names() {
    assert!(is_valid_collection_name("grocery_list"));
    assert!(is_valid_collection_name("a"));
    assert!(is_valid_collection_name("tasks123"));
    assert!(is_valid_collection_name("my_data_2024"));
}

#[test]
fn test_invalid_collection_names() {
    assert!(!is_valid_collection_name(""));
    assert!(!is_valid_collection_name("123start"));
    assert!(!is_valid_collection_name("_underscore_start"));
    assert!(!is_valid_collection_name("has space"));
    assert!(!is_valid_collection_name("has-dash"));
    assert!(!is_valid_collection_name("has.dot"));
    // 65 chars
    assert!(!is_valid_collection_name(&"a".repeat(65)));
    // Exactly 64 is fine
    assert!(is_valid_collection_name(&format!("a{}", "b".repeat(63))));
}

#[test]
fn test_parse_field_defs_valid() {
    let input = serde_json::json!([
        {"name": "title", "type": "text", "required": true},
        {"name": "count", "type": "number"},
        {"name": "done", "type": "bool"},
        {"name": "priority", "type": "enum", "values": ["low", "medium", "high"]},
        {"name": "due", "type": "date"},
        {"name": "created", "type": "datetime"},
    ]);

    let fields = parse_field_defs(&input).unwrap();
    assert_eq!(fields.len(), 6);
    assert_eq!(fields[0].name, "title");
    assert!(fields[0].required);
    assert!(matches!(fields[0].field_type, FieldType::Text));
    assert!(matches!(fields[1].field_type, FieldType::Number));
    assert!(matches!(fields[2].field_type, FieldType::Bool));
    assert!(matches!(fields[3].field_type, FieldType::Enum));
    assert_eq!(fields[3].values, vec!["low", "medium", "high"]);
    assert!(matches!(fields[4].field_type, FieldType::Date));
    assert!(matches!(fields[5].field_type, FieldType::Datetime));
}

#[test]
fn test_parse_field_defs_enum_requires_values() {
    let input = serde_json::json!([
        {"name": "status", "type": "enum"}
    ]);

    let err = parse_field_defs(&input).unwrap_err();
    assert!(err.contains("requires a non-empty 'values' array"));
}

#[test]
fn test_parse_field_defs_unknown_type() {
    let input = serde_json::json!([
        {"name": "x", "type": "blob"}
    ]);

    let err = parse_field_defs(&input).unwrap_err();
    assert!(err.contains("unknown field type 'blob'"));
}

#[test]
fn test_format_field_type() {
    assert_eq!(format_field_type(&FieldType::Text), "text");
    assert_eq!(format_field_type(&FieldType::Number), "number");
    assert_eq!(format_field_type(&FieldType::Bool), "bool");
    assert_eq!(format_field_type(&FieldType::Enum), "enum");
    assert_eq!(format_field_type(&FieldType::Date), "date");
    assert_eq!(format_field_type(&FieldType::Datetime), "datetime");
}

#[test]
fn test_parse_field_defs_empty_name() {
    let input = serde_json::json!([
        {"name": "", "type": "text"}
    ]);

    let err = parse_field_defs(&input).unwrap_err();
    assert!(err.contains("1-64 chars"));
}

#[test]
fn test_parse_field_defs_long_name() {
    let long_name = "a".repeat(65);
    let input = serde_json::json!([
        {"name": long_name, "type": "text"}
    ]);

    let err = parse_field_defs(&input).unwrap_err();
    assert!(err.contains("1-64 chars"));
}
