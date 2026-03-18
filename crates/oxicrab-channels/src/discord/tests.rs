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
    assert!(parse_components_from_metadata(&meta, None).is_empty());
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
    let rows = parse_components_from_metadata(&meta, None);
    assert_eq!(rows.len(), 1);
}

#[test]
fn test_parse_components_empty_buttons_skipped() {
    let mut meta = HashMap::new();
    meta.insert(
        "discord_components".to_string(),
        serde_json::json!([{"buttons": []}]),
    );
    let rows = parse_components_from_metadata(&meta, None);
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
    let rows = parse_components_from_metadata(&meta, None);
    // Button without custom_id is filter_map'd out
    assert!(rows.is_empty());
}

// --- Unified buttons format tests ---

#[test]
fn test_parse_unified_buttons() {
    let mut meta = HashMap::new();
    meta.insert(
        oxicrab_core::bus::events::meta::BUTTONS.to_string(),
        serde_json::json!([
            {"id": "approve", "label": "Approve", "style": "success"},
            {"id": "deny", "label": "Deny", "style": "danger"}
        ]),
    );
    let rows = parse_unified_buttons(&meta, None);
    assert_eq!(rows.len(), 1); // one action row
}

#[test]
fn test_parse_unified_buttons_empty() {
    let meta = HashMap::new();
    assert!(parse_unified_buttons(&meta, None).is_empty());
}

#[test]
fn test_discord_components_takes_precedence() {
    let mut meta = HashMap::new();
    // Both keys present — discord_components should win
    meta.insert(
        "discord_components".to_string(),
        serde_json::json!([
            {"buttons": [{"custom_id": "legacy", "label": "Legacy", "style": "primary"}]}
        ]),
    );
    meta.insert(
        oxicrab_core::bus::events::meta::BUTTONS.to_string(),
        serde_json::json!([{"id": "unified", "label": "Unified"}]),
    );
    let rows = parse_components_from_metadata(&meta, None);
    assert_eq!(rows.len(), 1);
    // parse_components_from_metadata checks discord_components first
}

#[test]
fn test_parse_unified_buttons_missing_id_skipped() {
    let mut meta = HashMap::new();
    meta.insert(
        oxicrab_core::bus::events::meta::BUTTONS.to_string(),
        serde_json::json!([{"label": "No ID"}]),
    );
    let rows = parse_unified_buttons(&meta, None);
    assert!(rows.is_empty());
}

#[test]
fn test_parse_unified_buttons_stores_dispatch_payload() {
    let store = crate::dispatch::DispatchContextStore::new(100);
    let payload = serde_json::json!({"tool": "rss", "params": {"action": "accept"}});
    let mut meta = HashMap::new();
    meta.insert(
        oxicrab_core::bus::events::meta::BUTTONS.to_string(),
        serde_json::json!([
            {
                "id": "rss_accept",
                "label": "Accept",
                "style": "success",
                "context": payload.to_string()
            }
        ]),
    );
    let rows = parse_unified_buttons(&meta, Some(&store));
    assert_eq!(rows.len(), 1);
    let stored = store.get("rss_accept");
    assert!(stored.is_some());
    assert_eq!(stored.unwrap().tool, "rss");
}

#[test]
fn test_parse_unified_buttons_no_store_no_panic() {
    // context field present but no store — should silently skip, still render button
    let mut meta = HashMap::new();
    meta.insert(
        oxicrab_core::bus::events::meta::BUTTONS.to_string(),
        serde_json::json!([
            {
                "id": "btn1",
                "label": "Click",
                "context": r#"{"tool":"x","params":{}}"#
            }
        ]),
    );
    let rows = parse_unified_buttons(&meta, None);
    assert_eq!(rows.len(), 1);
}

#[test]
fn test_components_to_api_json_unified() {
    let mut meta = HashMap::new();
    meta.insert(
        oxicrab_core::bus::events::meta::BUTTONS.to_string(),
        serde_json::json!([
            {"id": "yes", "label": "Yes", "style": "primary"},
            {"id": "no", "label": "No", "style": "danger"}
        ]),
    );
    let json = components_to_api_json(&meta);
    assert!(json.is_some());
    let components = json.unwrap();
    let rows = components.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let buttons = rows[0]["components"].as_array().unwrap();
    assert_eq!(buttons.len(), 2);
    assert_eq!(buttons[0]["custom_id"], "yes");
    assert_eq!(buttons[0]["label"], "Yes");
    assert_eq!(buttons[0]["style"], 1); // primary = 1
    assert_eq!(buttons[1]["custom_id"], "no");
    assert_eq!(buttons[1]["style"], 4); // danger = 4
}
