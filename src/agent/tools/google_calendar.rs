use crate::actions;
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::google_common::GoogleApiClient;
use crate::agent::tools::{Tool, ToolResult};
use crate::auth::google::GoogleCredentials;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::fmt::Write;

pub struct GoogleCalendarTool {
    api: GoogleApiClient,
}

impl GoogleCalendarTool {
    pub fn new(credentials: GoogleCredentials) -> Self {
        Self {
            api: GoogleApiClient::new(credentials, "https://www.googleapis.com/calendar/v3"),
        }
    }
}

#[async_trait]
impl Tool for GoogleCalendarTool {
    fn name(&self) -> &'static str {
        "google_calendar"
    }

    fn description(&self) -> &'static str {
        "Interact with Google Calendar. Actions: list_events, get_event, create_event, update_event, delete_event, list_calendars."
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: actions![
                list_events: ro,
                get_event: ro,
                create_event,
                update_event,
                delete_event,
                list_calendars: ro,
            ],
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_events", "get_event", "create_event", "update_event", "delete_event", "list_calendars"],
                    "description": "Action to perform"
                },
                "calendar_id": {
                    "type": "string",
                    "description": "Calendar ID (default: 'primary')"
                },
                "event_id": {
                    "type": "string",
                    "description": "Event ID (for get/update/delete)"
                },
                "time_min": {
                    "type": "string",
                    "description": "Start of time range (ISO 8601, for list_events). Defaults to now."
                },
                "time_max": {
                    "type": "string",
                    "description": "End of time range (ISO 8601, for list_events). Defaults to 7 days from now."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Max events to return (for list_events, default 20)",
                    "minimum": 1,
                    "maximum": 100
                },
                "summary": {
                    "type": "string",
                    "description": "Event title (for create/update)"
                },
                "description": {
                    "type": "string",
                    "description": "Event description (for create/update)"
                },
                "location": {
                    "type": "string",
                    "description": "Event location (for create/update)"
                },
                "start": {
                    "type": "string",
                    "description": "Event start time in ISO 8601 (for create/update). e.g. '2026-02-06T10:00:00'"
                },
                "end": {
                    "type": "string",
                    "description": "Event end time in ISO 8601 (for create/update). e.g. '2026-02-06T11:00:00'"
                },
                "timezone": {
                    "type": "string",
                    "description": "Timezone for the event (e.g. 'America/New_York'). Defaults to UTC."
                },
                "attendees": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of attendee email addresses (for create/update)"
                },
                "all_day": {
                    "type": "boolean",
                    "description": "If true, create an all-day event (use date instead of dateTime)"
                }
            },
            "required": ["action"]
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        let cal_id = params["calendar_id"].as_str().unwrap_or("primary");

        match action {
            "list_events" => {
                let now = Utc::now();
                let time_min = params["time_min"]
                    .as_str()
                    .map_or_else(|| now.to_rfc3339(), ensure_rfc3339_tz);
                let time_max = params["time_max"].as_str().map_or_else(
                    || (now + chrono::Duration::days(7)).to_rfc3339(),
                    ensure_rfc3339_tz,
                );
                let max_results = params["max_results"].as_u64().unwrap_or(20).min(100) as u32;

                let endpoint = format!(
                    "calendars/{}/events?timeMin={}&timeMax={}&maxResults={}&singleEvents=true&orderBy=startTime",
                    urlencoding::encode(cal_id),
                    urlencoding::encode(&time_min),
                    urlencoding::encode(&time_max),
                    max_results
                );
                let result = self.api.call(&endpoint, "GET", None).await?;
                let empty_vec: Vec<serde_json::Value> = vec![];
                let events = result["items"].as_array().unwrap_or(&empty_vec);

                if events.is_empty() {
                    return Ok(ToolResult::new("No upcoming events found.".to_string()));
                }

                let mut lines = vec![format!("Found {} event(s):\n", events.len())];
                for ev in events {
                    let start = ev["start"]["dateTime"]
                        .as_str()
                        .or_else(|| ev["start"]["date"].as_str())
                        .unwrap_or("?");
                    let end = ev["end"]["dateTime"]
                        .as_str()
                        .or_else(|| ev["end"]["date"].as_str())
                        .unwrap_or("?");
                    let summary = ev["summary"].as_str().unwrap_or("(no title)");
                    let location = ev["location"].as_str().unwrap_or("");
                    let loc_str = if location.is_empty() {
                        String::new()
                    } else {
                        format!("\n  Location: {}", location)
                    };
                    let empty_attendees: Vec<serde_json::Value> = vec![];
                    let attendees = ev["attendees"].as_array().unwrap_or(&empty_attendees);
                    let att_str = if attendees.is_empty() {
                        String::new()
                    } else {
                        let names: Vec<String> = attendees
                            .iter()
                            .take(5)
                            .filter_map(|a| a["email"].as_str().map(ToString::to_string))
                            .collect();
                        let mut s = format!("\n  Attendees: {}", names.join(", "));
                        if attendees.len() > 5 {
                            let _ = write!(s, " (+{} more)", attendees.len() - 5);
                        }
                        s
                    };

                    lines.push(format!(
                        "- {}\n  ID: {}\n  Start: {}\n  End: {}{}{}",
                        summary,
                        ev["id"].as_str().unwrap_or("?"),
                        start,
                        end,
                        loc_str,
                        att_str
                    ));
                }
                Ok(ToolResult::new(lines.join("\n")))
            }
            "get_event" => {
                let event_id = params["event_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'event_id' parameter"))?;

                let endpoint = format!(
                    "calendars/{}/events/{}",
                    urlencoding::encode(cal_id),
                    urlencoding::encode(event_id)
                );
                let ev = self.api.call(&endpoint, "GET", None).await?;
                Ok(ToolResult::new(format_event_detail(&ev)))
            }
            "create_event" => {
                let summary = params["summary"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'summary' parameter"))?;
                let start_raw = params["start"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'start' parameter"))?;

                let tz = params["timezone"].as_str().unwrap_or("UTC");
                let all_day = params["all_day"].as_bool().unwrap_or(false);

                let mut body = serde_json::json!({
                    "summary": summary
                });

                if let Some(desc) = params["description"].as_str() {
                    body["description"] = Value::String(desc.to_string());
                }
                if let Some(loc) = params["location"].as_str() {
                    body["location"] = Value::String(loc.to_string());
                }

                if all_day {
                    // Validate date format (YYYY-MM-DD) for all-day events
                    let start_date = if start_raw.len() >= 10 {
                        &start_raw[..10]
                    } else {
                        return Ok(ToolResult::error(format!(
                            "invalid date format for all-day event: '{}' (expected YYYY-MM-DD)",
                            start_raw
                        )));
                    };
                    if chrono::NaiveDate::parse_from_str(start_date, "%Y-%m-%d").is_err() {
                        return Ok(ToolResult::error(format!(
                            "invalid date: '{}' (expected YYYY-MM-DD)",
                            start_date
                        )));
                    }
                    body["start"] = serde_json::json!({"date": start_date});
                    let end_raw = params["end"].as_str().unwrap_or(start_raw);
                    let end_date = if end_raw.len() >= 10 {
                        &end_raw[..10]
                    } else {
                        start_date
                    };
                    body["end"] = serde_json::json!({"date": end_date});
                } else {
                    let (start_obj, end_obj) =
                        build_event_times(start_raw, params["end"].as_str(), tz);
                    body["start"] = start_obj;
                    body["end"] = end_obj;
                }

                if let Some(attendees) = params["attendees"].as_array() {
                    body["attendees"] = Value::Array(
                        attendees
                            .iter()
                            .filter_map(|a| a.as_str())
                            .map(|email| serde_json::json!({"email": email}))
                            .collect(),
                    );
                }

                let endpoint = format!("calendars/{}/events", urlencoding::encode(cal_id));
                let ev = self.api.call(&endpoint, "POST", Some(body)).await?;
                Ok(ToolResult::new(format!(
                    "Event created: {} (ID: {})\nLink: {}",
                    ev["summary"].as_str().unwrap_or("?"),
                    ev["id"].as_str().unwrap_or("?"),
                    ev["htmlLink"].as_str().unwrap_or("")
                )))
            }
            "update_event" => {
                let event_id = params["event_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'event_id' parameter"))?;

                let endpoint = format!(
                    "calendars/{}/events/{}",
                    urlencoding::encode(cal_id),
                    urlencoding::encode(event_id)
                );
                let mut ev = self.api.call(&endpoint, "GET", None).await?;

                let tz = params["timezone"].as_str().unwrap_or("UTC");

                if let Some(s) = params["summary"].as_str() {
                    ev["summary"] = Value::String(s.to_string());
                }
                if let Some(d) = params["description"].as_str() {
                    ev["description"] = Value::String(d.to_string());
                }
                if let Some(l) = params["location"].as_str() {
                    ev["location"] = Value::String(l.to_string());
                }
                if let Some(s) = params["start"].as_str() {
                    if params["all_day"].as_bool().unwrap_or(false) {
                        ev["start"] = serde_json::json!({"date": &s[..10.min(s.len())]});
                    } else {
                        ev["start"] = serde_json::json!({"dateTime": s, "timeZone": tz});
                    }
                }
                if let Some(e) = params["end"].as_str() {
                    if params["all_day"].as_bool().unwrap_or(false) {
                        ev["end"] = serde_json::json!({"date": &e[..10.min(e.len())]});
                    } else {
                        ev["end"] = serde_json::json!({"dateTime": e, "timeZone": tz});
                    }
                }
                if let Some(attendees) = params["attendees"].as_array() {
                    ev["attendees"] = Value::Array(
                        attendees
                            .iter()
                            .filter_map(|a| a.as_str())
                            .map(|email| serde_json::json!({"email": email}))
                            .collect(),
                    );
                }

                let updated = self.api.call(&endpoint, "PUT", Some(ev)).await?;
                Ok(ToolResult::new(format!(
                    "Event updated: {} (ID: {})",
                    updated["summary"].as_str().unwrap_or("?"),
                    updated["id"].as_str().unwrap_or("?")
                )))
            }
            "delete_event" => {
                let event_id = params["event_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'event_id' parameter"))?;

                let endpoint = format!(
                    "calendars/{}/events/{}",
                    urlencoding::encode(cal_id),
                    urlencoding::encode(event_id)
                );
                self.api.call(&endpoint, "DELETE", None).await?;
                Ok(ToolResult::new(format!("Event {} deleted.", event_id)))
            }
            "list_calendars" => {
                let result = self.api.call("users/me/calendarList", "GET", None).await?;
                let empty_cals: Vec<serde_json::Value> = vec![];
                let cals = result["items"].as_array().unwrap_or(&empty_cals);
                if cals.is_empty() {
                    return Ok(ToolResult::new("No calendars found.".to_string()));
                }
                let mut lines = vec!["Your calendars:\n".to_string()];
                for cal in cals {
                    let primary = if cal["primary"].as_bool().unwrap_or(false) {
                        " (primary)"
                    } else {
                        ""
                    };
                    lines.push(format!(
                        "- {}{}\n  ID: {}",
                        cal["summary"].as_str().unwrap_or("?"),
                        primary,
                        cal["id"].as_str().unwrap_or("?")
                    ));
                }
                Ok(ToolResult::new(lines.join("\n")))
            }
            _ => Ok(ToolResult::error(format!("unknown action: {}", action))),
        }
    }
}

/// Build the start/end JSON objects for a timed (non all-day) event.
/// Bare timestamps are passed through without appending Z so that the
/// timeZone field controls interpretation. If no end time is provided,
/// defaults to start + 1 hour.
fn build_event_times(start_raw: &str, end_raw: Option<&str>, tz: &str) -> (Value, Value) {
    let start_obj = serde_json::json!({"dateTime": start_raw, "timeZone": tz});
    let end_str = end_raw.map_or_else(
        || {
            DateTime::parse_from_rfc3339(&ensure_rfc3339_tz(start_raw)).map_or_else(
                |_| start_raw.to_string(),
                |dt| {
                    (dt + chrono::Duration::hours(1))
                        .format("%Y-%m-%dT%H:%M:%S")
                        .to_string()
                },
            )
        },
        ToString::to_string,
    );
    let end_obj = serde_json::json!({"dateTime": &end_str, "timeZone": tz});
    (start_obj, end_obj)
}

/// Ensure a timestamp string has a timezone suffix for RFC 3339 compliance.
/// If the string already ends with 'Z' or has an offset like '+00:00'/'-05:00', return as-is.
/// Otherwise, append 'Z' (UTC) so the Google Calendar API accepts it.
fn ensure_rfc3339_tz(s: &str) -> String {
    let trimmed = s.trim();
    // Already has 'Z' suffix
    if trimmed.ends_with('Z') || trimmed.ends_with('z') {
        return trimmed.to_string();
    }
    // Already has a +HH:MM or -HH:MM offset (e.g. "2026-03-07T10:00:00+05:00")
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 6 {
        let tail = &trimmed[trimmed.len() - 6..];
        if (tail.starts_with('+') || tail.starts_with('-'))
            && tail[1..3].chars().all(|c| c.is_ascii_digit())
            && tail.as_bytes()[3] == b':'
            && tail[4..6].chars().all(|c| c.is_ascii_digit())
        {
            return trimmed.to_string();
        }
    }
    // Only append Z if this looks like a datetime (contains 'T'), not a date-only string
    if trimmed.contains('T') {
        format!("{}Z", trimmed)
    } else {
        trimmed.to_string()
    }
}

fn format_event_detail(ev: &Value) -> String {
    let start = ev["start"]["dateTime"]
        .as_str()
        .or_else(|| ev["start"]["date"].as_str())
        .unwrap_or("?");
    let end = ev["end"]["dateTime"]
        .as_str()
        .or_else(|| ev["end"]["date"].as_str())
        .unwrap_or("?");
    let mut parts = vec![
        format!(
            "Summary: {}",
            ev["summary"].as_str().unwrap_or("(no title)")
        ),
        format!("ID: {}", ev["id"].as_str().unwrap_or("")),
        format!("Start: {}", start),
        format!("End: {}", end),
    ];
    if let Some(loc) = ev["location"].as_str() {
        parts.push(format!("Location: {}", loc));
    }
    if let Some(desc) = ev["description"].as_str() {
        parts.push(format!("Description: {}", desc));
    }
    if let Some(attendees) = ev["attendees"].as_array() {
        let att: Vec<String> = attendees
            .iter()
            .filter_map(|a| a["email"].as_str().map(ToString::to_string))
            .collect();
        parts.push(format!("Attendees: {}", att.join(", ")));
    }
    if let Some(link) = ev["htmlLink"].as_str() {
        parts.push(format!("Link: {}", link));
    }
    if let Some(status) = ev["status"].as_str() {
        parts.push(format!("Status: {}", status));
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
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
        // Offset already present — passed through as-is
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
        let (start, _) =
            build_event_times("2026-02-15T14:00:00", Some("2026-02-15T15:00:00"), "UTC");
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
                "action '{}' in schema but not in capabilities()",
                action
            );
        }
        for action in &cap_actions {
            assert!(
                schema_actions.contains(action),
                "action '{}' in capabilities() but not in schema",
                action
            );
        }
    }
}
