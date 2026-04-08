use super::*;

#[test]
fn test_generate_skill_content() {
    let fields = vec![
        FieldDef {
            name: "item".into(),
            field_type: FieldType::Text,
            required: true,
            values: vec![],
        },
        FieldDef {
            name: "quantity".into(),
            field_type: FieldType::Number,
            required: false,
            values: vec![],
        },
        FieldDef {
            name: "category".into(),
            field_type: FieldType::Enum,
            required: false,
            values: vec!["produce".into(), "dairy".into()],
        },
    ];

    let content = generate_collection_skill("grocery_list", "Weekly shopping list", &fields);

    assert!(content.contains("# grocery_list Collection"));
    assert!(content.contains("Weekly shopping list"));
    assert!(content.contains("**item** (text, required)"));
    assert!(content.contains("**quantity** (number)"));
    assert!(content.contains("values: [\"produce\", \"dairy\"]"));
    assert!(content.contains("action \"add\""));
    assert!(content.contains("action \"query\""));
}
