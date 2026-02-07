use crate::agent::tools::{Tool, ToolResult};
use crate::auth::google::GoogleCredentials;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::sync::Arc;

pub struct GoogleCalendarTool {
    credentials: Arc<tokio::sync::Mutex<GoogleCredentials>>,
}

impl GoogleCalendarTool {
    pub fn new(credentials: GoogleCredentials) -> Self {
        Self {
            credentials: Arc::new(tokio::sync::Mutex::new(credentials)),
        }
    }

    async fn get_access_token(&self) -> Result<String> {
        let mut creds = self.credentials.lock().await;
        if !creds.is_valid() {
            creds.refresh().await?;
        }
        Ok(creds.get_access_token().to_string())
    }

    async fn call_api(&self, endpoint: &str, method: &str, body: Option<Value>) -> Result<Value> {
        let token = self.get_access_token().await?;
        let client = reqwest::Client::new();
        let url = format!("https://www.googleapis.com/calendar/v3/{}", endpoint);

        let mut request = match method {
            "GET" => client.get(&url),
            "POST" => client.post(&url),
            "PUT" => client.put(&url),
            "DELETE" => client.delete(&url),
            _ => return Err(anyhow::anyhow!("Unsupported method: {}", method)),
        };

        request = request.header("Authorization", format!("Bearer {}", token));

        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request.send().await?;
        let data: serde_json::Value = response.error_for_status()?.json().await?;
        Ok(data)
    }
}

#[async_trait]
impl Tool for GoogleCalendarTool {
    fn name(&self) -> &str {
        "google_calendar"
    }

    fn description(&self) -> &str {
        "Interact with Google Calendar. Actions: list_events, get_event, create_event, update_event, delete_event, list_calendars."
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

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        let cal_id = params["calendar_id"]
            .as_str()
            .unwrap_or("primary");

        match action {
            "list_events" => {
                let now = Utc::now();
                let time_min = params["time_min"]
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| now.to_rfc3339());
                let time_max = params["time_max"]
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        (now + chrono::Duration::days(7)).to_rfc3339()
                    });
                let max_results = params["max_results"].as_u64().unwrap_or(20) as u32;

                let endpoint = format!(
                    "calendars/{}/events?timeMin={}&timeMax={}&maxResults={}&singleEvents=true&orderBy=startTime",
                    urlencoding::encode(cal_id), urlencoding::encode(&time_min), urlencoding::encode(&time_max), max_results
                );
                let result = self.call_api(&endpoint, "GET", None).await?;
                let empty_vec: Vec<serde_json::Value> = vec![];
                let events = result["items"].as_array().unwrap_or(&empty_vec);

                if events.is_empty() {
                    return Ok(ToolResult::new("No upcoming events found.".to_string()));
                }

                let mut lines = vec![format!("Found {} event(s):\n", events.len())];
                for ev in events {
                    let start = ev["start"]["dateTime"].as_str()
                        .or_else(|| ev["start"]["date"].as_str())
                        .unwrap_or("?");
                    let end = ev["end"]["dateTime"].as_str()
                        .or_else(|| ev["end"]["date"].as_str())
                        .unwrap_or("?");
                    let summary = ev["summary"].as_str().unwrap_or("(no title)");
                    let location = ev["location"].as_str().unwrap_or("");
                    let loc_str = if !location.is_empty() {
                        format!("\n  Location: {}", location)
                    } else {
                        String::new()
                    };
                    let empty_attendees: Vec<serde_json::Value> = vec![];
                    let attendees = ev["attendees"].as_array().unwrap_or(&empty_attendees);
                    let att_str = if !attendees.is_empty() {
                        let names: Vec<String> = attendees.iter()
                            .take(5)
                            .filter_map(|a| a["email"].as_str().map(|s| s.to_string()))
                            .collect();
                        let mut s = format!("\n  Attendees: {}", names.join(", "));
                        if attendees.len() > 5 {
                            s.push_str(&format!(" (+{} more)", attendees.len() - 5));
                        }
                        s
                    } else {
                        String::new()
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

                let endpoint = format!("calendars/{}/events/{}", urlencoding::encode(cal_id), urlencoding::encode(event_id));
                let ev = self.call_api(&endpoint, "GET", None).await?;
                Ok(ToolResult::new(format_event_detail(&ev)))
            }
            "create_event" => {
                let summary = params["summary"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'summary' parameter"))?;
                let start = params["start"]
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
                    body["start"] = serde_json::json!({"date": &start[..10.min(start.len())]});
                    let end = params["end"].as_str().unwrap_or(start);
                    body["end"] = serde_json::json!({"date": &end[..10.min(end.len())]});
                } else {
                    body["start"] = serde_json::json!({"dateTime": start, "timeZone": tz});
                    let end = params["end"].as_str().map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            DateTime::parse_from_rfc3339(start)
                                .map(|dt| (dt + chrono::Duration::hours(1)).to_rfc3339())
                                .unwrap_or_else(|_| start.to_string())
                        });
                    body["end"] = serde_json::json!({"dateTime": end, "timeZone": tz});
                }

                if let Some(attendees) = params["attendees"].as_array() {
                    body["attendees"] = Value::Array(
                        attendees.iter()
                            .filter_map(|a| a.as_str())
                            .map(|email| serde_json::json!({"email": email}))
                            .collect()
                    );
                }

                let endpoint = format!("calendars/{}/events", urlencoding::encode(cal_id));
                let ev = self.call_api(&endpoint, "POST", Some(body)).await?;
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

                let endpoint = format!("calendars/{}/events/{}", urlencoding::encode(cal_id), urlencoding::encode(event_id));
                let mut ev = self.call_api(&endpoint, "GET", None).await?;

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
                        attendees.iter()
                            .filter_map(|a| a.as_str())
                            .map(|email| serde_json::json!({"email": email}))
                            .collect()
                    );
                }

                let updated = self.call_api(&endpoint, "PUT", Some(ev)).await?;
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

                let endpoint = format!("calendars/{}/events/{}", urlencoding::encode(cal_id), urlencoding::encode(event_id));
                self.call_api(&endpoint, "DELETE", None).await?;
                Ok(ToolResult::new(format!("Event {} deleted.", event_id)))
            }
            "list_calendars" => {
                let result = self.call_api("users/me/calendarList", "GET", None).await?;
                let empty_cals: Vec<serde_json::Value> = vec![];
                let cals = result["items"].as_array().unwrap_or(&empty_cals);
                if cals.is_empty() {
                    return Ok(ToolResult::new("No calendars found.".to_string()));
                }
                let mut lines = vec!["Your calendars:\n".to_string()];
                for cal in cals {
                    let primary = if cal["primary"].as_bool().unwrap_or(false) { " (primary)" } else { "" };
                    lines.push(format!(
                        "- {}{}\n  ID: {}",
                        cal["summary"].as_str().unwrap_or("?"),
                        primary,
                        cal["id"].as_str().unwrap_or("?")
                    ));
                }
                Ok(ToolResult::new(lines.join("\n")))
            }
            _ => Ok(ToolResult::error(format!("Unknown action: {}", action))),
        }
    }
}

fn format_event_detail(ev: &Value) -> String {
    let start = ev["start"]["dateTime"].as_str()
        .or_else(|| ev["start"]["date"].as_str())
        .unwrap_or("?");
    let end = ev["end"]["dateTime"].as_str()
        .or_else(|| ev["end"]["date"].as_str())
        .unwrap_or("?");
    let mut parts = vec![
        format!("Summary: {}", ev["summary"].as_str().unwrap_or("(no title)")),
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
        let att: Vec<String> = attendees.iter()
            .filter_map(|a| a["email"].as_str().map(|s| s.to_string()))
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
