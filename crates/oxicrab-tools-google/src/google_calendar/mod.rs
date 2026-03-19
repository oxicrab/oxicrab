use crate::credentials::GoogleCredentials;
use crate::google_common::GoogleApiClient;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use oxicrab_core::actions;
use oxicrab_core::require_param;
use oxicrab_core::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory};
use oxicrab_core::tools::base::{Tool, ToolResult};
use oxicrab_core::utils::url_params::validate_url_segment;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write;
use std::time::SystemTime;
use tracing::warn;

pub struct GoogleCalendarTool {
    api: GoogleApiClient,
}

impl GoogleCalendarTool {
    pub fn new(credentials: GoogleCredentials) -> Self {
        Self {
            api: GoogleApiClient::new(credentials, "https://www.googleapis.com/calendar/v3"),
        }
    }

    /// Fetch the user's timezone from Google Calendar settings.
    /// Falls back to "UTC" on any error.
    async fn get_user_timezone(&self) -> String {
        match self
            .api
            .call("users/me/settings/timezone", "GET", None)
            .await
        {
            Ok(data) => data["value"].as_str().unwrap_or("UTC").to_string(),
            Err(e) => {
                warn!("failed to fetch user timezone: {e}, defaulting to UTC");
                "UTC".to_string()
            }
        }
    }
}

#[async_trait]
impl Tool for GoogleCalendarTool {
    fn name(&self) -> &'static str {
        "google_calendar"
    }

    fn description(&self) -> &'static str {
        "Interact with Google Calendar. Actions: list_events, get_event, create_event, update_event, delete_event, rsvp, list_calendars."
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
                rsvp,
                list_calendars: ro,
            ],
            category: ToolCategory::Scheduling,
        }
    }

    fn requires_approval_for_action(&self, action: &str) -> bool {
        matches!(action, "create_event" | "update_event" | "delete_event")
    }

    fn usage_examples(&self) -> Vec<oxicrab_core::tools::base::ToolExample> {
        vec![
            oxicrab_core::tools::base::ToolExample {
                user_request: "what's on my calendar today".into(),
                params: serde_json::json!({"action": "list_events"}),
            },
            oxicrab_core::tools::base::ToolExample {
                user_request: "schedule a meeting tomorrow at 2pm".into(),
                params: serde_json::json!({"action": "create_event", "summary": "Meeting", "start": "tomorrow 14:00"}),
            },
        ]
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_events", "get_event", "create_event", "update_event", "delete_event", "rsvp", "list_calendars"],
                    "description": "Action to perform. 'list_events' shows upcoming \
                     events (defaults to next 7 days). 'list_calendars' shows available calendars \
                     and their IDs. 'rsvp' responds to an event (accepted/declined/tentative)."
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
                    "description": "Timezone for the event (e.g. 'America/New_York'). Defaults to the user's Google Calendar timezone."
                },
                "attendees": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of attendee email addresses (for create/update)"
                },
                "all_day": {
                    "type": "boolean",
                    "description": "If true, create an all-day event (use date instead of dateTime)"
                },
                "response": {
                    "type": "string",
                    "enum": ["accepted", "declined", "tentative"],
                    "description": "RSVP response status (for rsvp action)"
                },
                "send_updates": {
                    "type": "string",
                    "enum": ["all", "externalOnly", "none"],
                    "description": "Controls who receives email notifications. Default: 'all'"
                }
            },
            "required": ["action"]
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = require_param!(params, "action");

        let cal_id = params["calendar_id"].as_str().unwrap_or("primary");
        if let Err(e) = validate_url_segment(cal_id, "calendar_id") {
            return Ok(ToolResult::error(e));
        }

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

                let base_endpoint = format!(
                    "calendars/{}/events?timeMin={}&timeMax={}&maxResults={}&singleEvents=true&orderBy=startTime",
                    urlencoding::encode(cal_id),
                    urlencoding::encode(&time_min),
                    urlencoding::encode(&time_max),
                    max_results
                );

                let mut all_events: Vec<Value> = Vec::new();
                let mut page_token: Option<String> = None;
                let max_pages = 5;

                for _ in 0..max_pages {
                    let endpoint = if let Some(ref token) = page_token {
                        format!("{}&pageToken={}", base_endpoint, urlencoding::encode(token))
                    } else {
                        base_endpoint.clone()
                    };

                    let data = self.api.call(&endpoint, "GET", None).await?;

                    if let Some(items) = data["items"].as_array() {
                        all_events.extend(items.iter().cloned());
                    }

                    match data["nextPageToken"].as_str() {
                        Some(token) if !token.is_empty() => {
                            page_token = Some(token.to_string());
                        }
                        _ => break,
                    }
                }

                if all_events.is_empty() {
                    return Ok(ToolResult::new("No upcoming events found.".to_string()));
                }

                let mut lines = vec![format!("Found {} event(s):\n", all_events.len())];
                for ev in &all_events {
                    let start = ev["start"]["dateTime"]
                        .as_str()
                        .or_else(|| ev["start"]["date"].as_str())
                        .unwrap_or("?");
                    let end = ev["end"]["dateTime"]
                        .as_str()
                        .or_else(|| ev["end"]["date"].as_str())
                        .unwrap_or("?");
                    let summary = ev["summary"].as_str().unwrap_or("(no title)");
                    let location = ev["location"].as_str().unwrap_or_default();
                    let loc_str = if location.is_empty() {
                        String::new()
                    } else {
                        format!("\n  Location: {location}")
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
                let buttons = build_event_buttons(&all_events, cal_id, false);
                Ok(ToolResult::new(lines.join("\n")).with_buttons(buttons))
            }
            "get_event" => {
                let event_id = require_param!(params, "event_id");
                if let Err(e) = validate_url_segment(event_id, "event_id") {
                    return Ok(ToolResult::error(e));
                }

                let endpoint = format!(
                    "calendars/{}/events/{}",
                    urlencoding::encode(cal_id),
                    urlencoding::encode(event_id)
                );
                let ev = self.api.call(&endpoint, "GET", None).await?;
                let buttons = build_event_buttons(std::slice::from_ref(&ev), cal_id, true);
                let now = SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0));
                let mut metadata: HashMap<String, Value> = HashMap::new();
                metadata.insert(
                    "active_tool".to_string(),
                    serde_json::json!("google_calendar"),
                );
                metadata.insert(
                    "action_directives".to_string(),
                    serde_json::json!([
                        {
                            "trigger": {"OneOf": ["yes", "accept", "rsvp yes"]},
                            "tool": "google_calendar",
                            "params": {"action": "rsvp", "event_id": event_id, "calendar_id": cal_id, "response": "accepted"},
                            "single_use": true,
                            "ttl_ms": 300_000,
                            "created_at_ms": now
                        },
                        {
                            "trigger": {"OneOf": ["no", "decline", "rsvp no"]},
                            "tool": "google_calendar",
                            "params": {"action": "rsvp", "event_id": event_id, "calendar_id": cal_id, "response": "declined"},
                            "single_use": true,
                            "ttl_ms": 300_000,
                            "created_at_ms": now
                        }
                    ]),
                );
                if !buttons.is_empty() {
                    metadata.insert("suggested_buttons".to_string(), Value::Array(buttons));
                }
                Ok(ToolResult::new(format_event_detail(&ev)).with_metadata(metadata))
            }
            "create_event" => {
                let summary = require_param!(params, "summary");
                let start_raw = require_param!(params, "start");

                let user_tz = if params["timezone"].is_string() {
                    None
                } else {
                    Some(self.get_user_timezone().await)
                };
                let tz = params["timezone"]
                    .as_str()
                    .unwrap_or_else(|| user_tz.as_deref().unwrap_or("UTC"));
                let all_day = params["all_day"].as_bool().unwrap_or_default();
                let send_updates = params["send_updates"].as_str().unwrap_or("all");

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
                        start_raw.get(..10).unwrap_or(start_raw)
                    } else {
                        return Ok(ToolResult::error(format!(
                            "invalid date format for all-day event: '{start_raw}' (expected YYYY-MM-DD)"
                        )));
                    };
                    if chrono::NaiveDate::parse_from_str(start_date, "%Y-%m-%d").is_err() {
                        return Ok(ToolResult::error(format!(
                            "invalid date: '{start_date}' (expected YYYY-MM-DD)"
                        )));
                    }
                    body["start"] = serde_json::json!({"date": start_date});
                    let end_date = if let Some(end_raw) = params["end"].as_str() {
                        if end_raw.len() >= 10 {
                            end_raw.get(..10).unwrap_or(end_raw).to_string()
                        } else {
                            end_raw.to_string()
                        }
                    } else {
                        // Google Calendar all-day events use exclusive end dates,
                        // so a 1-day event on "2026-03-04" needs end = "2026-03-05"
                        match chrono::NaiveDate::parse_from_str(start_date, "%Y-%m-%d") {
                            Ok(d) => {
                                let next = d + chrono::Duration::days(1);
                                next.format("%Y-%m-%d").to_string()
                            }
                            Err(_) => start_date.to_string(),
                        }
                    };
                    body["end"] = serde_json::json!({"date": end_date});
                } else {
                    match build_event_times(start_raw, params["end"].as_str(), tz) {
                        Ok((start_obj, end_obj)) => {
                            body["start"] = start_obj;
                            body["end"] = end_obj;
                        }
                        Err(msg) => return Ok(ToolResult::error(msg)),
                    }
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

                let endpoint = format!(
                    "calendars/{}/events?sendUpdates={}",
                    urlencoding::encode(cal_id),
                    urlencoding::encode(send_updates)
                );
                let ev = self.api.call(&endpoint, "POST", Some(body)).await?;
                Ok(ToolResult::new(format!(
                    "Event created: {} (ID: {})\nLink: {}",
                    ev["summary"].as_str().unwrap_or("?"),
                    ev["id"].as_str().unwrap_or("?"),
                    ev["htmlLink"].as_str().unwrap_or_default()
                )))
            }
            "update_event" => {
                let event_id = require_param!(params, "event_id");
                if let Err(e) = validate_url_segment(event_id, "event_id") {
                    return Ok(ToolResult::error(e));
                }
                let send_updates = params["send_updates"].as_str().unwrap_or("all");

                let user_tz = if params["timezone"].is_string() {
                    None
                } else {
                    Some(self.get_user_timezone().await)
                };
                let tz = params["timezone"]
                    .as_str()
                    .unwrap_or_else(|| user_tz.as_deref().unwrap_or("UTC"));
                let all_day = params["all_day"].as_bool().unwrap_or_default();

                // Build a partial update object with only the fields being changed
                let mut patch = serde_json::Map::new();

                if let Some(s) = params["summary"].as_str() {
                    patch.insert("summary".to_string(), Value::String(s.to_string()));
                }
                if let Some(d) = params["description"].as_str() {
                    patch.insert("description".to_string(), Value::String(d.to_string()));
                }
                if let Some(l) = params["location"].as_str() {
                    patch.insert("location".to_string(), Value::String(l.to_string()));
                }

                if all_day {
                    if let Some(start) = params["start"].as_str() {
                        let date_str = start.get(..10).unwrap_or(start);
                        if chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").is_err() {
                            return Ok(ToolResult::error(format!(
                                "invalid start date format: '{date_str}' (expected YYYY-MM-DD)"
                            )));
                        }
                        patch.insert("start".to_string(), serde_json::json!({"date": date_str}));
                    }
                    if let Some(end) = params["end"].as_str() {
                        let date_str = end.get(..10).unwrap_or(end);
                        if chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").is_err() {
                            return Ok(ToolResult::error(format!(
                                "invalid end date format: '{date_str}' (expected YYYY-MM-DD)"
                            )));
                        }
                        patch.insert("end".to_string(), serde_json::json!({"date": date_str}));
                    }
                } else {
                    if let Some(s) = params["start"].as_str() {
                        patch.insert(
                            "start".to_string(),
                            serde_json::json!({"dateTime": s, "timeZone": tz}),
                        );
                    }
                    if let Some(e) = params["end"].as_str() {
                        patch.insert(
                            "end".to_string(),
                            serde_json::json!({"dateTime": e, "timeZone": tz}),
                        );
                    }
                }

                if let Some(attendees) = params["attendees"].as_array() {
                    patch.insert(
                        "attendees".to_string(),
                        Value::Array(
                            attendees
                                .iter()
                                .filter_map(|a| a.as_str())
                                .map(|email| serde_json::json!({"email": email}))
                                .collect(),
                        ),
                    );
                }

                let endpoint = format!(
                    "calendars/{}/events/{}?sendUpdates={}",
                    urlencoding::encode(cal_id),
                    urlencoding::encode(event_id),
                    urlencoding::encode(send_updates)
                );
                let updated = self
                    .api
                    .call(&endpoint, "PATCH", Some(Value::Object(patch)))
                    .await?;
                Ok(ToolResult::new(format!(
                    "Event updated: {} (ID: {})",
                    updated["summary"].as_str().unwrap_or("?"),
                    updated["id"].as_str().unwrap_or("?")
                )))
            }
            "delete_event" => {
                let event_id = require_param!(params, "event_id");
                if let Err(e) = validate_url_segment(event_id, "event_id") {
                    return Ok(ToolResult::error(e));
                }
                let send_updates = params["send_updates"].as_str().unwrap_or("all");

                let endpoint = format!(
                    "calendars/{}/events/{}?sendUpdates={}",
                    urlencoding::encode(cal_id),
                    urlencoding::encode(event_id),
                    urlencoding::encode(send_updates)
                );
                self.api.call(&endpoint, "DELETE", None).await?;
                Ok(ToolResult::new(format!("Event {event_id} deleted.")))
            }
            "rsvp" => {
                let event_id = require_param!(params, "event_id");
                if let Err(e) = validate_url_segment(event_id, "event_id") {
                    return Ok(ToolResult::error(e));
                }
                let response = require_param!(params, "response");
                if !matches!(response, "accepted" | "declined" | "tentative") {
                    return Ok(ToolResult::error(format!(
                        "invalid response '{response}'. Must be accepted, declined, or tentative"
                    )));
                }
                let send_updates = params["send_updates"].as_str().unwrap_or("all");

                let get_endpoint = format!(
                    "calendars/{}/events/{}",
                    urlencoding::encode(cal_id),
                    urlencoding::encode(event_id)
                );
                let mut ev = self.api.call(&get_endpoint, "GET", None).await?;

                // Find or add the current user in the attendees list
                let attendees = ev["attendees"].as_array().cloned().unwrap_or_default();

                let mut found = false;
                let updated_attendees: Vec<Value> = attendees
                    .into_iter()
                    .map(|mut a| {
                        if a["self"].as_bool().unwrap_or(false) {
                            a["responseStatus"] = Value::String(response.to_string());
                            found = true;
                        }
                        a
                    })
                    .collect();

                if !found {
                    return Ok(ToolResult::error(
                        "You are not listed as an attendee for this event. You can only RSVP to events you've been invited to.",
                    ));
                }

                ev["attendees"] = Value::Array(updated_attendees);

                let put_endpoint = format!(
                    "calendars/{}/events/{}?sendUpdates={}",
                    urlencoding::encode(cal_id),
                    urlencoding::encode(event_id),
                    urlencoding::encode(send_updates)
                );
                let updated = self.api.call(&put_endpoint, "PUT", Some(ev)).await?;
                Ok(ToolResult::new(format!(
                    "RSVP '{}' for event: {} (ID: {})",
                    response,
                    updated["summary"].as_str().unwrap_or("?"),
                    updated["id"].as_str().unwrap_or("?")
                )))
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
                    let primary = if cal["primary"].as_bool().unwrap_or_default() {
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
            _ => Ok(ToolResult::error(format!("unknown action: {action}"))),
        }
    }
}

/// Build the start/end JSON objects for a timed (non all-day) event.
/// Bare timestamps are passed through without appending Z so that the
/// timeZone field controls interpretation. If no end time is provided,
/// defaults to start + 1 hour. Returns an error if the start time cannot
/// be parsed and no explicit end time is given.
fn build_event_times(
    start_raw: &str,
    end_raw: Option<&str>,
    tz: &str,
) -> std::result::Result<(Value, Value), String> {
    let start_obj = serde_json::json!({"dateTime": start_raw, "timeZone": tz});
    let end_str = match end_raw {
        Some(e) => e.to_string(),
        None => match DateTime::parse_from_rfc3339(&ensure_rfc3339_tz(start_raw)) {
            Ok(dt) => (dt + chrono::Duration::hours(1))
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string(),
            Err(_) => {
                return Err(format!(
                    "could not parse start time '{}' to compute default end time \
                         -- please provide an explicit end time",
                    start_raw
                ));
            }
        },
    };
    let end_obj = serde_json::json!({"dateTime": &end_str, "timeZone": tz});
    Ok((start_obj, end_obj))
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
        // Timezone offsets are ASCII, safe to index by byte
        let tail = &bytes[bytes.len() - 6..];
        if (tail[0] == b'+' || tail[0] == b'-')
            && tail[1].is_ascii_digit()
            && tail[2].is_ascii_digit()
            && tail[3] == b':'
            && tail[4].is_ascii_digit()
            && tail[5].is_ascii_digit()
        {
            return trimmed.to_string();
        }
    }
    // Only append Z if this looks like a datetime (contains 'T'), not a date-only string
    if trimmed.contains('T') {
        format!("{trimmed}Z")
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
        format!("ID: {}", ev["id"].as_str().unwrap_or_default()),
        format!("Start: {}", start),
        format!("End: {}", end),
    ];
    if let Some(loc) = ev["location"].as_str() {
        parts.push(format!("Location: {loc}"));
    }
    if let Some(desc) = ev["description"].as_str() {
        parts.push(format!("Description: {desc}"));
    }
    if let Some(attendees) = ev["attendees"].as_array() {
        let att: Vec<String> = attendees
            .iter()
            .filter_map(|a| a["email"].as_str().map(ToString::to_string))
            .collect();
        parts.push(format!("Attendees: {}", att.join(", ")));
    }
    if let Some(link) = ev["htmlLink"].as_str() {
        parts.push(format!("Link: {link}"));
    }
    if let Some(status) = ev["status"].as_str() {
        parts.push(format!("Status: {status}"));
    }
    parts.join("\n")
}

/// Truncate a string to `max_chars` characters, appending "..." if truncated.
/// Uses char boundaries for UTF-8 safety.
fn truncate_label(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

/// Check whether any attendee in the event has already responded with the
/// given status (e.g. "accepted"). Uses the `self` attendee if present,
/// otherwise returns false.
fn has_self_response(ev: &Value, response_status: &str) -> bool {
    ev["attendees"]
        .as_array()
        .and_then(|attendees| {
            attendees
                .iter()
                .find(|a| a["self"].as_bool().unwrap_or(false))
        })
        .is_some_and(|me| me["responseStatus"].as_str() == Some(response_status))
}

/// Build suggested buttons for calendar events.
///
/// For `list_events` (`detail=false`): up to 5 "RSVP Yes" buttons, skipping
/// events the user has already accepted.
///
/// For `get_event` (`detail=true`): "RSVP Yes", "RSVP No", and "Delete"
/// buttons for the single event, skipping RSVP buttons whose status already
/// matches.
fn build_event_buttons(events: &[Value], calendar_id: &str, detail: bool) -> Vec<Value> {
    let mut buttons = Vec::new();

    for ev in events {
        if buttons.len() >= 5 {
            break;
        }

        let event_id = ev["id"].as_str().unwrap_or_default();
        if event_id.is_empty() {
            continue;
        }

        // Skip cancelled events
        if ev["status"].as_str() == Some("cancelled") {
            continue;
        }

        let summary = ev["summary"].as_str().unwrap_or("(no title)");

        if detail {
            // Single event view: RSVP Yes, RSVP No, Delete
            if !has_self_response(ev, "accepted") {
                let label = format!("RSVP Yes: {}", truncate_label(summary, 20));
                buttons.push(serde_json::json!({
                    "id": format!("rsvp-yes-{event_id}"),
                    "label": truncate_label(&label, 30),
                    "style": "primary",
                    "context": serde_json::json!({
                        "tool": "google_calendar",
                        "params": {
                            "action": "rsvp",
                            "event_id": event_id,
                            "calendar_id": calendar_id,
                            "response": "accepted"
                        }
                    }).to_string()
                }));
            }

            if !has_self_response(ev, "declined") {
                let label = format!("RSVP No: {}", truncate_label(summary, 20));
                buttons.push(serde_json::json!({
                    "id": format!("rsvp-no-{event_id}"),
                    "label": truncate_label(&label, 30),
                    "style": "danger",
                    "context": serde_json::json!({
                        "tool": "google_calendar",
                        "params": {
                            "action": "rsvp",
                            "event_id": event_id,
                            "calendar_id": calendar_id,
                            "response": "declined"
                        }
                    }).to_string()
                }));
            }

            buttons.push(serde_json::json!({
                "id": format!("delete-{event_id}"),
                "label": truncate_label(&format!("Delete: {summary}"), 30),
                "style": "danger",
                "context": serde_json::json!({
                    "tool": "google_calendar",
                    "params": {
                        "action": "delete_event",
                        "event_id": event_id,
                        "calendar_id": calendar_id
                    }
                }).to_string()
            }));
        } else {
            // List view: one RSVP Yes button per event, skip already-accepted
            if has_self_response(ev, "accepted") {
                continue;
            }

            let label = format!("RSVP Yes: {}", truncate_label(summary, 20));
            buttons.push(serde_json::json!({
                "id": format!("rsvp-yes-{event_id}"),
                "label": truncate_label(&label, 30),
                "style": "primary",
                "context": serde_json::json!({
                    "tool": "google_calendar",
                    "params": {
                        "action": "rsvp",
                        "event_id": event_id,
                        "calendar_id": calendar_id,
                        "response": "accepted"
                    }
                }).to_string()
            }));
        }
    }

    buttons
}

#[cfg(test)]
mod tests;
