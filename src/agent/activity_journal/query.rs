//! Time-windowed read over an [`ActivityJournal`].

use super::ActivityRecord;
use chrono::{DateTime, Duration, Utc};

/// One interval over the journal: `[anchor - half_window, anchor + half_window]`.
#[derive(Debug, Clone)]
pub struct ActivityWindow {
    pub anchor: DateTime<Utc>,
    pub half_window_minutes: u32,
}

impl ActivityWindow {
    pub fn start(&self) -> DateTime<Utc> {
        self.anchor - Duration::minutes(i64::from(self.half_window_minutes))
    }
    pub fn end(&self) -> DateTime<Utc> {
        self.anchor + Duration::minutes(i64::from(self.half_window_minutes))
    }
}

/// Filter `records` to those within `window`. Returns chronologically
/// ordered results (the journal is append-only so filtering preserves
/// order). `session_filter` restricts to one session when `Some`.
pub fn query_window<'a>(
    records: &'a [ActivityRecord],
    window: &ActivityWindow,
    session_filter: Option<&str>,
) -> Vec<&'a ActivityRecord> {
    let start = window.start();
    let end = window.end();
    records
        .iter()
        .filter(|r| r.timestamp >= start && r.timestamp <= end)
        .filter(|r| match session_filter {
            Some(s) => r.session_key == s,
            None => true,
        })
        .collect()
}

/// Render a slice of records as a markdown-like transcript suitable for
/// returning to the LLM as a tool result.
pub fn render_records(records: &[&ActivityRecord]) -> String {
    if records.is_empty() {
        return "No activity recorded in the requested window.".to_string();
    }
    let mut out = String::new();
    for r in records {
        let ts = r.timestamp.format("%Y-%m-%d %H:%M:%S UTC");
        let role = match r.role.as_str() {
            "user" => "USER",
            "agent" => "AGENT",
            other => other,
        };
        out.push_str(&format!("[{ts}] {role}: {}\n", r.content));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn rec(ts: DateTime<Utc>, role: &str, body: &str) -> ActivityRecord {
        ActivityRecord {
            timestamp: ts,
            session_key: "demo".to_string(),
            role: role.to_string(),
            content: body.to_string(),
        }
    }

    #[test]
    fn window_filters_inclusive_bounds() {
        let now = Utc.with_ymd_and_hms(2026, 4, 25, 14, 0, 0).unwrap();
        let recs = vec![
            rec(now - Duration::minutes(31), "user", "outside-before"),
            rec(now - Duration::minutes(15), "user", "inside-early"),
            rec(now, "agent", "anchor"),
            rec(now + Duration::minutes(30), "user", "inside-late"),
            rec(now + Duration::minutes(31), "user", "outside-after"),
        ];
        let win = ActivityWindow {
            anchor: now,
            half_window_minutes: 30,
        };
        let hits = query_window(&recs, &win, None);
        let texts: Vec<_> = hits.iter().map(|r| r.content.as_str()).collect();
        assert_eq!(texts, vec!["inside-early", "anchor", "inside-late"]);
    }

    #[test]
    fn session_filter_works() {
        let now = Utc.with_ymd_and_hms(2026, 4, 25, 14, 0, 0).unwrap();
        let mut a = rec(now, "user", "alice");
        a.session_key = "a".to_string();
        let mut b = rec(now, "user", "bob");
        b.session_key = "b".to_string();
        let recs = vec![a, b];
        let win = ActivityWindow {
            anchor: now,
            half_window_minutes: 30,
        };
        let hits = query_window(&recs, &win, Some("a"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].content, "alice");
    }

    #[test]
    fn render_empty() {
        let out = render_records(&[]);
        assert!(out.contains("No activity"));
    }

    #[test]
    fn render_includes_role_and_time() {
        let now = Utc.with_ymd_and_hms(2026, 4, 25, 14, 0, 0).unwrap();
        let r = rec(now, "agent", "hello");
        let out = render_records(&[&r]);
        assert!(out.contains("AGENT: hello"));
        assert!(out.contains("2026-04-25 14:00:00 UTC"));
    }
}
