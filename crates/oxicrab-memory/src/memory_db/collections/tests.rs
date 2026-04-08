use super::*;
use crate::memory_db::MemoryDB;

fn test_db() -> MemoryDB {
    MemoryDB::new(":memory:").expect("in-memory DB")
}

fn simple_schema() -> CollectionSchema {
    CollectionSchema {
        fields: vec![
            FieldDef {
                name: "title".into(),
                field_type: FieldType::Text,
                required: true,
                values: vec![],
            },
            FieldDef {
                name: "count".into(),
                field_type: FieldType::Number,
                required: false,
                values: vec![],
            },
        ],
    }
}

// --- Collection CRUD ---

#[test]
fn create_and_get_collection() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "my tasks", &schema).unwrap();

    let info = db.get_collection("tasks").unwrap().unwrap();
    assert_eq!(info.name, "tasks");
    assert_eq!(info.description, "my tasks");
    assert_eq!(info.schema.fields.len(), 2);
    assert_eq!(info.record_count, 0);
}

#[test]
fn list_collections() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("a", "", &schema).unwrap();
    db.create_collection("b", "", &schema).unwrap();

    let list = db.list_collections().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].name, "a");
    assert_eq!(list[1].name, "b");
}

#[test]
fn delete_collection() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    assert!(db.delete_collection("tasks").unwrap());
    assert!(!db.delete_collection("tasks").unwrap());
    assert!(db.get_collection("tasks").unwrap().is_none());
}

#[test]
fn delete_collection_cascades_records() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();
    db.insert_record("tasks", serde_json::json!({"title": "a"}))
        .unwrap();
    db.insert_record("tasks", serde_json::json!({"title": "b"}))
        .unwrap();

    db.delete_collection("tasks").unwrap();

    // Records should be gone (cascade)
    let count: u64 = db
        .lock_conn()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM collection_records", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn duplicate_collection_name_rejected() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    let err = db.create_collection("tasks", "", &schema).unwrap_err();
    assert!(err.to_string().contains("already exists"));
}

// --- Schema validation ---

#[test]
fn invalid_collection_name_empty() {
    let db = test_db();
    let schema = simple_schema();
    let err = db.create_collection("", "", &schema).unwrap_err();
    assert!(err.to_string().contains("1-64 characters"));
}

#[test]
fn invalid_collection_name_special_chars() {
    let db = test_db();
    let schema = simple_schema();
    let err = db.create_collection("my-tasks", "", &schema).unwrap_err();
    assert!(err.to_string().contains("alphanumeric"));
}

#[test]
fn invalid_collection_name_too_long() {
    let db = test_db();
    let schema = simple_schema();
    let name = "a".repeat(65);
    let err = db.create_collection(&name, "", &schema).unwrap_err();
    assert!(err.to_string().contains("1-64 characters"));
}

#[test]
fn too_many_fields() {
    let db = test_db();
    let schema = CollectionSchema {
        fields: (0..11)
            .map(|i| FieldDef {
                name: format!("field_{i}"),
                field_type: FieldType::Text,
                required: false,
                values: vec![],
            })
            .collect(),
    };
    let err = db.create_collection("test", "", &schema).unwrap_err();
    assert!(err.to_string().contains("at most 10 fields"));
}

#[test]
fn empty_schema_rejected() {
    let db = test_db();
    let schema = CollectionSchema { fields: vec![] };
    let err = db.create_collection("test", "", &schema).unwrap_err();
    assert!(err.to_string().contains("at least one field"));
}

#[test]
fn enum_without_values_rejected() {
    let db = test_db();
    let schema = CollectionSchema {
        fields: vec![FieldDef {
            name: "status".into(),
            field_type: FieldType::Enum,
            required: false,
            values: vec![],
        }],
    };
    let err = db.create_collection("test", "", &schema).unwrap_err();
    assert!(err.to_string().contains("at least one value"));
}

// --- Record insert with validation ---

#[test]
fn insert_and_query_record() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    let id = db
        .insert_record("tasks", serde_json::json!({"title": "hello", "count": 5}))
        .unwrap();
    assert!(!id.is_empty());

    let records = db.query_records("tasks", &[], None, None).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].data["title"], "hello");
    assert_eq!(records[0].data["count"], 5);
}

#[test]
fn insert_missing_required_field() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    let err = db
        .insert_record("tasks", serde_json::json!({"count": 5}))
        .unwrap_err();
    assert!(err.to_string().contains("required field 'title'"));
}

#[test]
fn insert_wrong_type() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    let err = db
        .insert_record("tasks", serde_json::json!({"title": 123}))
        .unwrap_err();
    assert!(err.to_string().contains("expects text"));
}

#[test]
fn insert_unknown_field_rejected() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    let err = db
        .insert_record(
            "tasks",
            serde_json::json!({"title": "hi", "unknown_field": "x"}),
        )
        .unwrap_err();
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn insert_enum_validation() {
    let db = test_db();
    let schema = CollectionSchema {
        fields: vec![FieldDef {
            name: "status".into(),
            field_type: FieldType::Enum,
            required: true,
            values: vec!["open".into(), "closed".into()],
        }],
    };
    db.create_collection("items", "", &schema).unwrap();

    db.insert_record("items", serde_json::json!({"status": "open"}))
        .unwrap();

    let err = db
        .insert_record("items", serde_json::json!({"status": "invalid"}))
        .unwrap_err();
    assert!(err.to_string().contains("not in allowed values"));
}

#[test]
fn insert_bool_coercion() {
    let db = test_db();
    let schema = CollectionSchema {
        fields: vec![FieldDef {
            name: "done".into(),
            field_type: FieldType::Bool,
            required: true,
            values: vec![],
        }],
    };
    db.create_collection("items", "", &schema).unwrap();

    // String "true" coerced to bool
    let id = db
        .insert_record("items", serde_json::json!({"done": "true"}))
        .unwrap();
    let records = db.query_records("items", &[], None, None).unwrap();
    assert_eq!(records[0].data["done"], true);
    assert_eq!(records[0].id, id);
}

#[test]
fn insert_number_coercion_from_string() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    db.insert_record("tasks", serde_json::json!({"title": "test", "count": "42"}))
        .unwrap();

    let records = db.query_records("tasks", &[], None, None).unwrap();
    assert_eq!(records[0].data["count"], 42.0);
}

// --- Record query with filters ---

#[test]
fn query_with_eq_filter() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    db.insert_record("tasks", serde_json::json!({"title": "alpha", "count": 1}))
        .unwrap();
    db.insert_record("tasks", serde_json::json!({"title": "beta", "count": 2}))
        .unwrap();

    let filters = vec![RecordFilter {
        field: "title".into(),
        op: FilterOp::Eq,
        value: serde_json::json!("alpha"),
    }];
    let records = db.query_records("tasks", &filters, None, None).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].data["title"], "alpha");
}

#[test]
fn query_with_neq_filter() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    db.insert_record("tasks", serde_json::json!({"title": "alpha", "count": 1}))
        .unwrap();
    db.insert_record("tasks", serde_json::json!({"title": "beta", "count": 2}))
        .unwrap();

    let filters = vec![RecordFilter {
        field: "title".into(),
        op: FilterOp::Neq,
        value: serde_json::json!("alpha"),
    }];
    let records = db.query_records("tasks", &filters, None, None).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].data["title"], "beta");
}

#[test]
fn query_with_gt_lt_filters() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    for i in 1..=5 {
        db.insert_record(
            "tasks",
            serde_json::json!({"title": format!("t{i}"), "count": i}),
        )
        .unwrap();
    }

    let filters = vec![RecordFilter {
        field: "count".into(),
        op: FilterOp::Gt,
        value: serde_json::json!(3),
    }];
    let records = db.query_records("tasks", &filters, None, None).unwrap();
    assert_eq!(records.len(), 2);

    let filters = vec![RecordFilter {
        field: "count".into(),
        op: FilterOp::Lte,
        value: serde_json::json!(2),
    }];
    let records = db.query_records("tasks", &filters, None, None).unwrap();
    assert_eq!(records.len(), 2);
}

#[test]
fn query_with_contains_filter() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    db.insert_record("tasks", serde_json::json!({"title": "buy groceries"}))
        .unwrap();
    db.insert_record("tasks", serde_json::json!({"title": "buy clothes"}))
        .unwrap();
    db.insert_record("tasks", serde_json::json!({"title": "sell car"}))
        .unwrap();

    let filters = vec![RecordFilter {
        field: "title".into(),
        op: FilterOp::Contains,
        value: serde_json::json!("buy"),
    }];
    let records = db.query_records("tasks", &filters, None, None).unwrap();
    assert_eq!(records.len(), 2);
}

#[test]
fn query_with_limit_and_offset() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    for i in 1..=10 {
        db.insert_record("tasks", serde_json::json!({"title": format!("task_{i}")}))
            .unwrap();
    }

    let records = db.query_records("tasks", &[], Some(3), Some(0)).unwrap();
    assert_eq!(records.len(), 3);

    let records = db.query_records("tasks", &[], Some(3), Some(8)).unwrap();
    assert_eq!(records.len(), 2);
}

#[test]
fn query_contains_on_non_text_rejected() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    let filters = vec![RecordFilter {
        field: "count".into(),
        op: FilterOp::Contains,
        value: serde_json::json!("3"),
    }];
    let err = db.query_records("tasks", &filters, None, None).unwrap_err();
    assert!(err.to_string().contains("only works on text"));
}

// --- Record update ---

#[test]
fn update_record_partial() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    let id = db
        .insert_record(
            "tasks",
            serde_json::json!({"title": "original", "count": 1}),
        )
        .unwrap();

    let updated = db
        .update_record("tasks", &id, serde_json::json!({"count": 99}))
        .unwrap();
    assert!(updated);

    let records = db.query_records("tasks", &[], None, None).unwrap();
    assert_eq!(records[0].data["title"], "original");
    assert_eq!(records[0].data["count"], 99);
}

#[test]
fn update_nonexistent_record() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    let updated = db
        .update_record("tasks", "nonexistent", serde_json::json!({"title": "x"}))
        .unwrap();
    assert!(!updated);
}

#[test]
fn update_validates_types() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    let id = db
        .insert_record("tasks", serde_json::json!({"title": "test"}))
        .unwrap();

    let err = db
        .update_record("tasks", &id, serde_json::json!({"title": 123}))
        .unwrap_err();
    assert!(err.to_string().contains("expects text"));
}

// --- Delete record ---

#[test]
fn delete_record_works() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    let id = db
        .insert_record("tasks", serde_json::json!({"title": "test"}))
        .unwrap();

    assert!(db.delete_record("tasks", &id).unwrap());
    assert!(!db.delete_record("tasks", &id).unwrap());
}

// --- Count records ---

#[test]
fn count_records_with_filters() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    for i in 1..=5 {
        db.insert_record(
            "tasks",
            serde_json::json!({"title": format!("t{i}"), "count": i}),
        )
        .unwrap();
    }

    assert_eq!(db.count_records("tasks", &[]).unwrap(), 5);

    let filters = vec![RecordFilter {
        field: "count".into(),
        op: FilterOp::Gte,
        value: serde_json::json!(3),
    }];
    assert_eq!(db.count_records("tasks", &filters).unwrap(), 3);
}

// --- Aggregations ---

#[test]
fn aggregate_sum() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    for i in 1..=5 {
        db.insert_record(
            "tasks",
            serde_json::json!({"title": format!("t{i}"), "count": i}),
        )
        .unwrap();
    }

    let results = db
        .aggregate_records(
            "tasks",
            &AggregationRequest {
                function: AggFunction::Sum,
                field: "count".into(),
                group_by: None,
                filters: None,
            },
        )
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].value, serde_json::json!(15.0));
}

#[test]
fn aggregate_avg() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    for i in [10, 20, 30] {
        db.insert_record("tasks", serde_json::json!({"title": "x", "count": i}))
            .unwrap();
    }

    let results = db
        .aggregate_records(
            "tasks",
            &AggregationRequest {
                function: AggFunction::Avg,
                field: "count".into(),
                group_by: None,
                filters: None,
            },
        )
        .unwrap();

    assert_eq!(results[0].value, serde_json::json!(20.0));
}

#[test]
fn aggregate_count() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    for i in 1..=3 {
        db.insert_record(
            "tasks",
            serde_json::json!({"title": format!("t{i}"), "count": i}),
        )
        .unwrap();
    }

    let results = db
        .aggregate_records(
            "tasks",
            &AggregationRequest {
                function: AggFunction::Count,
                field: "count".into(),
                group_by: None,
                filters: None,
            },
        )
        .unwrap();

    assert_eq!(results[0].value, serde_json::json!(3));
}

#[test]
fn aggregate_min_max() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    for i in [5, 2, 8, 1, 9] {
        db.insert_record("tasks", serde_json::json!({"title": "x", "count": i}))
            .unwrap();
    }

    let min_results = db
        .aggregate_records(
            "tasks",
            &AggregationRequest {
                function: AggFunction::Min,
                field: "count".into(),
                group_by: None,
                filters: None,
            },
        )
        .unwrap();
    assert_eq!(min_results[0].value, serde_json::json!(1));

    let max_results = db
        .aggregate_records(
            "tasks",
            &AggregationRequest {
                function: AggFunction::Max,
                field: "count".into(),
                group_by: None,
                filters: None,
            },
        )
        .unwrap();
    assert_eq!(max_results[0].value, serde_json::json!(9));
}

#[test]
fn aggregate_with_group_by() {
    let db = test_db();
    let schema = CollectionSchema {
        fields: vec![
            FieldDef {
                name: "category".into(),
                field_type: FieldType::Text,
                required: true,
                values: vec![],
            },
            FieldDef {
                name: "amount".into(),
                field_type: FieldType::Number,
                required: true,
                values: vec![],
            },
        ],
    };
    db.create_collection("expenses", "", &schema).unwrap();

    db.insert_record(
        "expenses",
        serde_json::json!({"category": "food", "amount": 10}),
    )
    .unwrap();
    db.insert_record(
        "expenses",
        serde_json::json!({"category": "food", "amount": 20}),
    )
    .unwrap();
    db.insert_record(
        "expenses",
        serde_json::json!({"category": "transport", "amount": 15}),
    )
    .unwrap();

    let results = db
        .aggregate_records(
            "expenses",
            &AggregationRequest {
                function: AggFunction::Sum,
                field: "amount".into(),
                group_by: Some("category".into()),
                filters: None,
            },
        )
        .unwrap();

    assert_eq!(results.len(), 2);
    let food = results.iter().find(|r| r.group.as_deref() == Some("food"));
    let transport = results
        .iter()
        .find(|r| r.group.as_deref() == Some("transport"));
    assert_eq!(food.unwrap().value, serde_json::json!(30.0));
    assert_eq!(transport.unwrap().value, serde_json::json!(15.0));
}

#[test]
fn aggregate_sum_on_non_number_rejected() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    let err = db
        .aggregate_records(
            "tasks",
            &AggregationRequest {
                function: AggFunction::Sum,
                field: "title".into(),
                group_by: None,
                filters: None,
            },
        )
        .unwrap_err();
    assert!(err.to_string().contains("only works on number fields"));
}

#[test]
fn aggregate_with_filters() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    for i in 1..=5 {
        db.insert_record(
            "tasks",
            serde_json::json!({"title": format!("t{i}"), "count": i}),
        )
        .unwrap();
    }

    let results = db
        .aggregate_records(
            "tasks",
            &AggregationRequest {
                function: AggFunction::Sum,
                field: "count".into(),
                group_by: None,
                filters: Some(vec![RecordFilter {
                    field: "count".into(),
                    op: FilterOp::Gte,
                    value: serde_json::json!(3),
                }]),
            },
        )
        .unwrap();

    // 3 + 4 + 5 = 12
    assert_eq!(results[0].value, serde_json::json!(12.0));
}

// --- Schema alteration ---

#[test]
fn alter_schema_add_field() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    db.alter_collection_schema(
        "tasks",
        &[FieldDef {
            name: "priority".into(),
            field_type: FieldType::Number,
            required: false,
            values: vec![],
        }],
        &[],
    )
    .unwrap();

    let info = db.get_collection("tasks").unwrap().unwrap();
    assert_eq!(info.schema.fields.len(), 3);
    assert!(info.schema.fields.iter().any(|f| f.name == "priority"));
}

#[test]
fn alter_schema_remove_field_strips_data() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    db.insert_record("tasks", serde_json::json!({"title": "test", "count": 5}))
        .unwrap();

    db.alter_collection_schema("tasks", &[], &["count".into()])
        .unwrap();

    let info = db.get_collection("tasks").unwrap().unwrap();
    assert_eq!(info.schema.fields.len(), 1);

    let records = db.query_records("tasks", &[], None, None).unwrap();
    assert!(records[0].data.get("count").is_none());
    assert_eq!(records[0].data["title"], "test");
}

#[test]
fn alter_schema_exceeds_max_fields() {
    let db = test_db();
    let schema = CollectionSchema {
        fields: (0..10)
            .map(|i| FieldDef {
                name: format!("f{i}"),
                field_type: FieldType::Text,
                required: false,
                values: vec![],
            })
            .collect(),
    };
    db.create_collection("tasks", "", &schema).unwrap();

    let err = db
        .alter_collection_schema(
            "tasks",
            &[FieldDef {
                name: "extra".into(),
                field_type: FieldType::Text,
                required: false,
                values: vec![],
            }],
            &[],
        )
        .unwrap_err();
    assert!(err.to_string().contains("exceed 10 fields"));
}

#[test]
fn alter_schema_empty_result_rejected() {
    let db = test_db();
    let schema = CollectionSchema {
        fields: vec![FieldDef {
            name: "only_field".into(),
            field_type: FieldType::Text,
            required: false,
            values: vec![],
        }],
    };
    db.create_collection("tasks", "", &schema).unwrap();

    let err = db
        .alter_collection_schema("tasks", &[], &["only_field".into()])
        .unwrap_err();
    assert!(err.to_string().contains("at least one field"));
}

// --- Date/datetime parsing ---

#[test]
fn parse_iso_date() {
    assert_eq!(parse_natural_date("2026-04-07").unwrap(), "2026-04-07");
}

#[test]
fn parse_iso_datetime_as_date() {
    assert_eq!(
        parse_natural_date("2026-04-07T14:30:00").unwrap(),
        "2026-04-07"
    );
}

#[test]
fn parse_relative_dates() {
    let today_str = today().format("%Y-%m-%d").to_string();
    assert_eq!(parse_natural_date("today").unwrap(), today_str);

    let tomorrow = (today() + Duration::days(1)).format("%Y-%m-%d").to_string();
    assert_eq!(parse_natural_date("tomorrow").unwrap(), tomorrow);

    let yesterday = (today() - Duration::days(1)).format("%Y-%m-%d").to_string();
    assert_eq!(parse_natural_date("yesterday").unwrap(), yesterday);
}

#[test]
fn parse_relative_offsets() {
    let in_3_days = (today() + Duration::days(3)).format("%Y-%m-%d").to_string();
    assert_eq!(parse_natural_date("in 3 days").unwrap(), in_3_days);

    let ago_5_days = (today() - Duration::days(5)).format("%Y-%m-%d").to_string();
    assert_eq!(parse_natural_date("5 days ago").unwrap(), ago_5_days);
}

#[test]
fn parse_next_last_week_month() {
    let next_week = (today() + Duration::weeks(1))
        .format("%Y-%m-%d")
        .to_string();
    assert_eq!(parse_natural_date("next week").unwrap(), next_week);

    let last_week = (today() - Duration::weeks(1))
        .format("%Y-%m-%d")
        .to_string();
    assert_eq!(parse_natural_date("last week").unwrap(), last_week);

    // next month / last month produce valid dates
    let next_month = parse_natural_date("next month").unwrap();
    assert_eq!(next_month.len(), 10);

    let last_month = parse_natural_date("last month").unwrap();
    assert_eq!(last_month.len(), 10);
}

#[test]
fn parse_day_names() {
    // "friday" -> next friday
    let result = parse_natural_date("friday").unwrap();
    let parsed = NaiveDate::parse_from_str(&result, "%Y-%m-%d").unwrap();
    assert_eq!(parsed.weekday(), Weekday::Fri);
    assert!(parsed > today());

    // "next monday"
    let result = parse_natural_date("next monday").unwrap();
    let parsed = NaiveDate::parse_from_str(&result, "%Y-%m-%d").unwrap();
    assert_eq!(parsed.weekday(), Weekday::Mon);
    assert!(parsed > today());

    // "last wednesday"
    let result = parse_natural_date("last wednesday").unwrap();
    let parsed = NaiveDate::parse_from_str(&result, "%Y-%m-%d").unwrap();
    assert_eq!(parsed.weekday(), Weekday::Wed);
    assert!(parsed < today());
}

#[test]
fn parse_common_date_formats() {
    assert_eq!(parse_natural_date("April 7, 2026").unwrap(), "2026-04-07");
    assert_eq!(parse_natural_date("7 Apr 2026").unwrap(), "2026-04-07");
    assert_eq!(parse_natural_date("4/7/2026").unwrap(), "2026-04-07");
}

#[test]
fn parse_datetime_full() {
    assert_eq!(
        parse_natural_datetime("2026-04-07T14:30:00").unwrap(),
        "2026-04-07T14:30:00"
    );
}

#[test]
fn parse_datetime_with_time_component() {
    let result = parse_natural_datetime("tomorrow at 3pm").unwrap();
    let tomorrow = (today() + Duration::days(1)).format("%Y-%m-%d").to_string();
    assert_eq!(result, format!("{tomorrow}T15:00:00"));
}

#[test]
fn parse_datetime_with_24h_time() {
    let result = parse_natural_datetime("tomorrow at 14:30").unwrap();
    let tomorrow = (today() + Duration::days(1)).format("%Y-%m-%d").to_string();
    assert_eq!(result, format!("{tomorrow}T14:30:00"));
}

#[test]
fn parse_invalid_date_returns_error() {
    assert!(parse_natural_date("not a date").is_err());
    assert!(parse_natural_date("").is_err());
}

// --- Date/datetime fields in records ---

#[test]
fn insert_date_field_with_natural_language() {
    let db = test_db();
    let schema = CollectionSchema {
        fields: vec![
            FieldDef {
                name: "name".into(),
                field_type: FieldType::Text,
                required: true,
                values: vec![],
            },
            FieldDef {
                name: "due".into(),
                field_type: FieldType::Date,
                required: false,
                values: vec![],
            },
        ],
    };
    db.create_collection("tasks", "", &schema).unwrap();

    db.insert_record(
        "tasks",
        serde_json::json!({"name": "test", "due": "tomorrow"}),
    )
    .unwrap();

    let records = db.query_records("tasks", &[], None, None).unwrap();
    let due = records[0].data["due"].as_str().unwrap();
    let tomorrow = (today() + Duration::days(1)).format("%Y-%m-%d").to_string();
    assert_eq!(due, tomorrow);
}

#[test]
fn insert_datetime_field_coercion() {
    let db = test_db();
    let schema = CollectionSchema {
        fields: vec![FieldDef {
            name: "start".into(),
            field_type: FieldType::Datetime,
            required: true,
            values: vec![],
        }],
    };
    db.create_collection("events", "", &schema).unwrap();

    db.insert_record(
        "events",
        serde_json::json!({"start": "2026-04-07T14:30:00"}),
    )
    .unwrap();

    let records = db.query_records("events", &[], None, None).unwrap();
    assert_eq!(records[0].data["start"], "2026-04-07T14:30:00");
}

// --- Edge cases ---

#[test]
fn collection_limit_enforced() {
    let db = test_db();
    let schema = simple_schema();

    for i in 0..50 {
        db.create_collection(&format!("c{i}"), "", &schema).unwrap();
    }

    let err = db.create_collection("c50", "", &schema).unwrap_err();
    assert!(err.to_string().contains("maximum of 50"));
}

#[test]
fn record_limit_enforced() {
    let db = test_db();
    let schema = CollectionSchema {
        fields: vec![FieldDef {
            name: "val".into(),
            field_type: FieldType::Number,
            required: true,
            values: vec![],
        }],
    };
    db.create_collection("big", "", &schema).unwrap();

    // Insert up to the limit using raw SQL for speed
    {
        let mut conn = db.lock_conn().unwrap();
        let tx = conn.transaction().unwrap();
        for i in 0..10_000 {
            tx.execute(
                "INSERT INTO collection_records (id, collection_name, data_json) \
                 VALUES (?1, 'big', ?2)",
                params![format!("id-{i}"), format!("{{\"val\":{i}}}")],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }

    let err = db
        .insert_record("big", serde_json::json!({"val": 99999}))
        .unwrap_err();
    assert!(err.to_string().contains("maximum of 10000"));
}

#[test]
fn empty_filters_returns_all() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    db.insert_record("tasks", serde_json::json!({"title": "a"}))
        .unwrap();
    db.insert_record("tasks", serde_json::json!({"title": "b"}))
        .unwrap();

    let records = db.query_records("tasks", &[], None, None).unwrap();
    assert_eq!(records.len(), 2);
}

#[test]
fn nonexistent_collection_rejected() {
    let db = test_db();
    let err = db
        .insert_record("nonexistent", serde_json::json!({"title": "x"}))
        .unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn field_limit_exact_boundary() {
    let db = test_db();
    let schema = CollectionSchema {
        fields: (0..10)
            .map(|i| FieldDef {
                name: format!("f{i}"),
                field_type: FieldType::Text,
                required: false,
                values: vec![],
            })
            .collect(),
    };
    // 10 fields is OK
    db.create_collection("ok", "", &schema).unwrap();
    assert_eq!(
        db.get_collection("ok")
            .unwrap()
            .unwrap()
            .schema
            .fields
            .len(),
        10
    );
}

#[test]
fn collection_name_with_underscores() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("my_cool_list_2", "", &schema).unwrap();
    assert!(db.get_collection("my_cool_list_2").unwrap().is_some());
}

#[test]
fn insert_into_nonexistent_collection() {
    let db = test_db();
    let err = db.query_records("nope", &[], None, None).unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn aggregate_on_empty_collection() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("empty", "", &schema).unwrap();

    let results = db
        .aggregate_records(
            "empty",
            &AggregationRequest {
                function: AggFunction::Count,
                field: "count".into(),
                group_by: None,
                filters: None,
            },
        )
        .unwrap();
    assert_eq!(results[0].value, serde_json::json!(0));
}

// ── Null equality filter tests ──────────────────────────

#[test]
fn filter_null_equality() {
    let db = test_db();
    let schema = simple_schema(); // title (required Text), count (optional Number)
    db.create_collection("items", "", &schema).unwrap();

    db.insert_record(
        "items",
        serde_json::json!({"title": "with_count", "count": 5}),
    )
    .unwrap();
    db.insert_record("items", serde_json::json!({"title": "no_count"}))
        .unwrap();

    // Eq null: records missing the field
    let filters = vec![RecordFilter {
        field: "count".into(),
        op: FilterOp::Eq,
        value: serde_json::Value::Null,
    }];
    let records = db.query_records("items", &filters, None, None).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].data["title"], "no_count");

    // Neq null: records having the field
    let filters = vec![RecordFilter {
        field: "count".into(),
        op: FilterOp::Neq,
        value: serde_json::Value::Null,
    }];
    let records = db.query_records("items", &filters, None, None).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].data["title"], "with_count");
}

// ── Contains filter escapes wildcards ───────────────────

#[test]
fn contains_filter_escapes_wildcards() {
    let db = test_db();
    let schema = simple_schema();
    db.create_collection("tasks", "", &schema).unwrap();

    db.insert_record("tasks", serde_json::json!({"title": "50% done"}))
        .unwrap();
    db.insert_record("tasks", serde_json::json!({"title": "100 percent"}))
        .unwrap();

    let filters = vec![RecordFilter {
        field: "title".into(),
        op: FilterOp::Contains,
        value: serde_json::json!("50%"),
    }];
    let records = db.query_records("tasks", &filters, None, None).unwrap();
    assert_eq!(
        records.len(),
        1,
        "only the record with literal '50%' should match"
    );
    assert_eq!(records[0].data["title"], "50% done");
}

// ── Collection name validation ──────────────────────────

#[test]
fn collection_name_starting_with_digit_rejected() {
    let db = test_db();
    let schema = simple_schema();
    let err = db.create_collection("1tasks", "", &schema).unwrap_err();
    assert!(
        err.to_string().contains("start with a letter"),
        "expected 'start with a letter' error, got: {err}"
    );
}

#[test]
fn collection_name_starting_with_underscore_rejected() {
    let db = test_db();
    let schema = simple_schema();
    let err = db.create_collection("_tasks", "", &schema).unwrap_err();
    assert!(
        err.to_string().contains("start with a letter"),
        "expected 'start with a letter' error, got: {err}"
    );
}
