use super::*;

#[test]
fn test_ensure_rfc3339_tz_bare_timestamp() {
    assert_eq!(
        ensure_rfc3339_tz("2026-03-07T00:00:00"),
        "2026-03-07T00:00:00Z"
    );
}

#[test]
fn test_ensure_rfc3339_tz_already_z() {
    assert_eq!(
        ensure_rfc3339_tz("2026-03-07T00:00:00Z"),
        "2026-03-07T00:00:00Z"
    );
}

#[test]
fn test_ensure_rfc3339_tz_already_offset() {
    assert_eq!(
        ensure_rfc3339_tz("2026-03-07T10:00:00+05:00"),
        "2026-03-07T10:00:00+05:00"
    );
    assert_eq!(
        ensure_rfc3339_tz("2026-03-07T10:00:00-08:00"),
        "2026-03-07T10:00:00-08:00"
    );
}

#[test]
fn test_ensure_rfc3339_tz_with_whitespace() {
    assert_eq!(
        ensure_rfc3339_tz("  2026-03-07T00:00:00  "),
        "2026-03-07T00:00:00Z"
    );
}

#[test]
fn test_ensure_rfc3339_tz_date_only() {
    // Date-only strings should not get Z appended (invalid RFC 3339)
    assert_eq!(ensure_rfc3339_tz("2026-03-07"), "2026-03-07");
}

#[test]
fn test_build_event_times_bare_timestamp_not_z_suffixed() {
    // The core bug: bare timestamps must NOT get Z appended, otherwise
    // the timeZone field is ignored and the event lands at the wrong time.
    let (start, end) = build_event_times(
        "2026-02-15T14:00:00",
        Some("2026-02-15T15:00:00"),
        "America/New_York",
    );
    assert_eq!(start["dateTime"], "2026-02-15T14:00:00");
    assert_eq!(start["timeZone"], "America/New_York");
    assert_eq!(end["dateTime"], "2026-02-15T15:00:00");
    assert_eq!(end["timeZone"], "America/New_York");
}

#[test]
fn test_build_event_times_preserves_existing_offset() {
    let (start, _) = build_event_times(
        "2026-02-15T14:00:00-05:00",
        Some("2026-02-15T15:00:00-05:00"),
        "America/New_York",
    );
    // Offset already present -- passed through as-is
    assert_eq!(start["dateTime"], "2026-02-15T14:00:00-05:00");
}

#[test]
fn test_build_event_times_default_end_plus_one_hour() {
    let (start, end) = build_event_times("2026-02-15T14:00:00", None, "America/New_York");
    assert_eq!(start["dateTime"], "2026-02-15T14:00:00");
    // Default end = start + 1hr, also bare (no Z)
    assert_eq!(end["dateTime"], "2026-02-15T15:00:00");
    assert_eq!(end["timeZone"], "America/New_York");
}

#[test]
fn test_build_event_times_utc_default_tz() {
    let (start, _) = build_event_times("2026-02-15T14:00:00", Some("2026-02-15T15:00:00"), "UTC");
    assert_eq!(start["timeZone"], "UTC");
}

fn test_credentials() -> crate::auth::google::GoogleCredentials {
    crate::auth::google::GoogleCredentials {
        token: "fake".to_string(),
        refresh_token: None,
        token_uri: "https://oauth2.googleapis.com/token".to_string(),
        client_id: "fake".to_string(),
        client_secret: "fake".to_string(),
        scopes: vec![],
        expiry: None,
    }
}

#[test]
fn test_google_calendar_capabilities() {
    use crate::agent::tools::base::SubagentAccess;
    let tool = GoogleCalendarTool::new(test_credentials());
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::ReadOnly);
    let read_only: Vec<&str> = caps
        .actions
        .iter()
        .filter(|a| a.read_only)
        .map(|a| a.name)
        .collect();
    let mutating: Vec<&str> = caps
        .actions
        .iter()
        .filter(|a| !a.read_only)
        .map(|a| a.name)
        .collect();
    assert!(read_only.contains(&"list_events"));
    assert!(read_only.contains(&"get_event"));
    assert!(read_only.contains(&"list_calendars"));
    assert!(mutating.contains(&"create_event"));
    assert!(mutating.contains(&"update_event"));
    assert!(mutating.contains(&"delete_event"));
}

#[test]
fn test_google_calendar_actions_match_schema() {
    let tool = GoogleCalendarTool::new(test_credentials());
    let caps = tool.capabilities();
    let params = tool.parameters();
    let schema_actions: Vec<String> = params["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let cap_actions: Vec<String> = caps.actions.iter().map(|a| a.name.to_string()).collect();
    for action in &schema_actions {
        assert!(
            cap_actions.contains(action),
            "action '{action}' in schema but not in capabilities()"
        );
    }
    for action in &cap_actions {
        assert!(
            schema_actions.contains(action),
            "action '{action}' in capabilities() but not in schema"
        );
    }
}

// --- Suggested buttons tests ---

#[test]
fn test_build_event_buttons_list_view_rsvp_yes() {
    let events = vec![serde_json::json!({
        "id": "evt1",
        "summary": "Team standup",
        "status": "confirmed",
        "start": {"dateTime": "2026-03-15T10:00:00Z"},
        "end": {"dateTime": "2026-03-15T10:30:00Z"}
    })];
    let buttons = build_event_buttons(&events, "primary", false);
    assert_eq!(buttons.len(), 1);
    assert_eq!(buttons[0]["id"], "rsvp-yes-evt1");
    assert!(
        buttons[0]["label"]
            .as_str()
            .unwrap()
            .starts_with("RSVP Yes:")
    );
    assert_eq!(buttons[0]["style"], "primary");

    let ctx: serde_json::Value =
        serde_json::from_str(buttons[0]["context"].as_str().unwrap()).unwrap();
    assert_eq!(ctx["tool"], "google_calendar");
    assert_eq!(ctx["event_id"], "evt1");
    assert_eq!(ctx["calendar_id"], "primary");
    assert_eq!(ctx["action"], "rsvp_yes");
}

#[test]
fn test_build_event_buttons_list_view_skips_already_accepted() {
    let events = vec![serde_json::json!({
        "id": "evt1",
        "summary": "Team standup",
        "status": "confirmed",
        "attendees": [
            {"email": "me@example.com", "self": true, "responseStatus": "accepted"}
        ]
    })];
    let buttons = build_event_buttons(&events, "primary", false);
    assert!(buttons.is_empty(), "should skip already-accepted events");
}

#[test]
fn test_build_event_buttons_list_view_includes_needs_action() {
    let events = vec![serde_json::json!({
        "id": "evt1",
        "summary": "Team standup",
        "status": "confirmed",
        "attendees": [
            {"email": "me@example.com", "self": true, "responseStatus": "needsAction"}
        ]
    })];
    let buttons = build_event_buttons(&events, "primary", false);
    assert_eq!(buttons.len(), 1);
}

#[test]
fn test_build_event_buttons_list_view_no_attendees_included() {
    // Events without attendees (no self attendee) should still get a button
    let events = vec![serde_json::json!({
        "id": "evt1",
        "summary": "Solo work",
        "status": "confirmed"
    })];
    let buttons = build_event_buttons(&events, "primary", false);
    assert_eq!(buttons.len(), 1);
}

#[test]
fn test_build_event_buttons_detail_view_full_buttons() {
    let events = vec![serde_json::json!({
        "id": "evt1",
        "summary": "Design review",
        "status": "confirmed",
        "attendees": [
            {"email": "me@example.com", "self": true, "responseStatus": "needsAction"}
        ]
    })];
    let buttons = build_event_buttons(&events, "work-cal", true);
    // Should have: RSVP Yes, RSVP No, Delete
    assert_eq!(buttons.len(), 3);

    assert_eq!(buttons[0]["id"], "rsvp-yes-evt1");
    assert_eq!(buttons[0]["style"], "primary");

    assert_eq!(buttons[1]["id"], "rsvp-no-evt1");
    assert_eq!(buttons[1]["style"], "danger");

    assert_eq!(buttons[2]["id"], "delete-evt1");
    assert_eq!(buttons[2]["style"], "danger");

    // Verify context carries calendar_id
    let ctx: serde_json::Value =
        serde_json::from_str(buttons[2]["context"].as_str().unwrap()).unwrap();
    assert_eq!(ctx["calendar_id"], "work-cal");
    assert_eq!(ctx["action"], "delete");
}

#[test]
fn test_build_event_buttons_detail_view_already_accepted() {
    let events = vec![serde_json::json!({
        "id": "evt1",
        "summary": "Lunch",
        "status": "confirmed",
        "attendees": [
            {"email": "me@example.com", "self": true, "responseStatus": "accepted"}
        ]
    })];
    let buttons = build_event_buttons(&events, "primary", true);
    // RSVP Yes skipped, RSVP No + Delete remain
    assert_eq!(buttons.len(), 2);
    assert_eq!(buttons[0]["id"], "rsvp-no-evt1");
    assert_eq!(buttons[1]["id"], "delete-evt1");
}

#[test]
fn test_build_event_buttons_detail_view_already_declined() {
    let events = vec![serde_json::json!({
        "id": "evt1",
        "summary": "Lunch",
        "status": "confirmed",
        "attendees": [
            {"email": "me@example.com", "self": true, "responseStatus": "declined"}
        ]
    })];
    let buttons = build_event_buttons(&events, "primary", true);
    // RSVP No skipped, RSVP Yes + Delete remain
    assert_eq!(buttons.len(), 2);
    assert_eq!(buttons[0]["id"], "rsvp-yes-evt1");
    assert_eq!(buttons[1]["id"], "delete-evt1");
}

#[test]
fn test_build_event_buttons_skips_cancelled() {
    let events = vec![serde_json::json!({
        "id": "evt1",
        "summary": "Cancelled meeting",
        "status": "cancelled"
    })];
    let buttons = build_event_buttons(&events, "primary", false);
    assert!(buttons.is_empty());
}

#[test]
fn test_build_event_buttons_max_five() {
    let events: Vec<serde_json::Value> = (0..10)
        .map(|i| {
            serde_json::json!({
                "id": format!("evt{i}"),
                "summary": format!("Meeting {i}"),
                "status": "confirmed"
            })
        })
        .collect();
    let buttons = build_event_buttons(&events, "primary", false);
    assert_eq!(buttons.len(), 5);
}

#[test]
fn test_build_event_buttons_skips_empty_id() {
    let events = vec![serde_json::json!({
        "id": "",
        "summary": "No ID event",
        "status": "confirmed"
    })];
    let buttons = build_event_buttons(&events, "primary", false);
    assert!(buttons.is_empty());
}

#[test]
fn test_truncate_label_short() {
    assert_eq!(truncate_label("hello", 10), "hello");
}

#[test]
fn test_truncate_label_exact() {
    assert_eq!(truncate_label("hello", 5), "hello");
}

#[test]
fn test_truncate_label_long() {
    let result = truncate_label("a very long event name here", 15);
    assert!(result.ends_with("..."));
    assert!(result.chars().count() <= 15);
}

#[test]
fn test_truncate_label_unicode() {
    // Ensure we don't panic on multi-byte chars
    let result = truncate_label("caf\u{00e9} meeting with the team", 10);
    assert!(result.ends_with("..."));
    assert!(result.chars().count() <= 10);
}

#[test]
fn test_with_buttons_empty() {
    let result = ToolResult::new("test".to_string());
    let result = with_buttons(result, vec![]);
    assert!(result.metadata.is_none());
}

#[test]
fn test_with_buttons_non_empty() {
    let result = ToolResult::new("test".to_string());
    let buttons = vec![serde_json::json!({"id": "b1", "label": "Test"})];
    let result = with_buttons(result, buttons);
    let meta = result.metadata.expect("should have metadata");
    let btns = meta["suggested_buttons"].as_array().unwrap();
    assert_eq!(btns.len(), 1);
}
