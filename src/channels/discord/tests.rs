use super::*;

#[test]
fn test_parse_button_style_primary() {
    assert_eq!(parse_button_style("primary"), ButtonStyle::Primary);
}

#[test]
fn test_parse_button_style_success() {
    assert_eq!(parse_button_style("success"), ButtonStyle::Success);
}

#[test]
fn test_parse_button_style_danger() {
    assert_eq!(parse_button_style("danger"), ButtonStyle::Danger);
}

#[test]
fn test_parse_button_style_default_secondary() {
    assert_eq!(parse_button_style("secondary"), ButtonStyle::Secondary);
    assert_eq!(parse_button_style("unknown"), ButtonStyle::Secondary);
    assert_eq!(parse_button_style(""), ButtonStyle::Secondary);
}

#[test]
fn test_parse_embeds_empty_metadata() {
    let meta = HashMap::new();
    assert!(parse_embeds_from_metadata(&meta).is_empty());
}

#[test]
fn test_parse_embeds_no_array() {
    let mut meta = HashMap::new();
    meta.insert(
        "discord_embeds".to_string(),
        serde_json::json!("not an array"),
    );
    assert!(parse_embeds_from_metadata(&meta).is_empty());
}

#[test]
fn test_parse_embeds_with_entries() {
    let mut meta = HashMap::new();
    meta.insert(
        "discord_embeds".to_string(),
        serde_json::json!([
            {"title": "Test", "description": "A test embed", "color": 0x00FF_0000_u64}
        ]),
    );
    let embeds = parse_embeds_from_metadata(&meta);
    assert_eq!(embeds.len(), 1);
}

#[test]
fn test_parse_components_empty_metadata() {
    let meta = HashMap::new();
    assert!(parse_components_from_metadata(&meta).is_empty());
}

#[test]
fn test_parse_components_with_buttons() {
    let mut meta = HashMap::new();
    meta.insert(
        "discord_components".to_string(),
        serde_json::json!([
            {
                "buttons": [
                    {"custom_id": "btn_ok", "label": "OK", "style": "primary"},
                    {"custom_id": "btn_cancel", "label": "Cancel", "style": "danger"}
                ]
            }
        ]),
    );
    let rows = parse_components_from_metadata(&meta);
    assert_eq!(rows.len(), 1);
}

#[test]
fn test_parse_components_empty_buttons_skipped() {
    let mut meta = HashMap::new();
    meta.insert(
        "discord_components".to_string(),
        serde_json::json!([{"buttons": []}]),
    );
    let rows = parse_components_from_metadata(&meta);
    assert!(rows.is_empty());
}

#[test]
fn test_parse_components_missing_custom_id_skipped() {
    let mut meta = HashMap::new();
    meta.insert(
        "discord_components".to_string(),
        serde_json::json!([
            {"buttons": [{"label": "no_id"}]}
        ]),
    );
    let rows = parse_components_from_metadata(&meta);
    // Button without custom_id is filter_map'd out
    assert!(rows.is_empty());
}
