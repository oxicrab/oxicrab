use super::MemoryDB;
use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronTrace {
    pub id: String,
    pub job_id: String,
    pub job_name: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub events: Vec<TraceEvent>,
    pub summary: Option<String>,
    pub token_count: u32,
    pub tool_call_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TraceEvent {
    Started {
        message: String,
    },
    ToolCall {
        name: String,
        params_summary: String,
    },
    ToolResult {
        name: String,
        duration_ms: u64,
        is_error: bool,
        summary: String,
    },
    LlmRequest {
        message_count: usize,
    },
    LlmResponse {
        has_tool_calls: bool,
        token_count: u32,
    },
    Error {
        message: String,
    },
    Completed {
        response: String,
    },
}

impl CronTrace {
    pub fn new(id: String, job_id: String, job_name: String) -> Self {
        Self {
            id,
            job_id,
            job_name,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            status: "running".to_string(),
            events: Vec::new(),
            summary: None,
            token_count: 0,
            tool_call_count: 0,
        }
    }

    pub fn add_event(&mut self, event: TraceEvent) {
        match &event {
            TraceEvent::ToolCall { .. } => self.tool_call_count += 1,
            TraceEvent::LlmResponse { token_count, .. } => {
                self.token_count += token_count;
            }
            _ => {}
        }
        self.events.push(event);
    }

    pub fn complete(&mut self, summary: Option<String>) {
        self.status = "completed".to_string();
        self.completed_at = Some(chrono::Utc::now().to_rfc3339());
        self.summary = summary;
    }

    pub fn fail(&mut self, error: &str) {
        self.status = "failed".to_string();
        self.completed_at = Some(chrono::Utc::now().to_rfc3339());
        self.add_event(TraceEvent::Error {
            message: truncate_str(error, 500),
        });
    }
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

/// Truncate a tool params value to a concise summary string.
pub fn summarize_params(params: &serde_json::Value) -> String {
    let s = params.to_string();
    truncate_str(&s, 200)
}

/// Truncate a tool result to a concise summary string.
pub fn summarize_result(content: &str) -> String {
    truncate_str(content, 500)
}

impl MemoryDB {
    pub fn insert_cron_trace(&self, trace: &CronTrace) -> Result<()> {
        let conn = self.lock_conn()?;
        let events_json = serde_json::to_string(&trace.events)?;
        conn.execute(
            "INSERT INTO cron_execution_traces \
             (id, job_id, job_name, started_at, completed_at, status, events, \
              summary, token_count, tool_call_count) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                trace.id,
                trace.job_id,
                trace.job_name,
                trace.started_at,
                trace.completed_at,
                trace.status,
                events_json,
                trace.summary,
                trace.token_count,
                trace.tool_call_count,
            ],
        )?;
        Ok(())
    }

    pub fn update_cron_trace(&self, trace: &CronTrace) -> Result<()> {
        let conn = self.lock_conn()?;
        let events_json = serde_json::to_string(&trace.events)?;
        conn.execute(
            "UPDATE cron_execution_traces SET \
             completed_at = ?1, status = ?2, events = ?3, summary = ?4, \
             token_count = ?5, tool_call_count = ?6 \
             WHERE id = ?7",
            params![
                trace.completed_at,
                trace.status,
                events_json,
                trace.summary,
                trace.token_count,
                trace.tool_call_count,
                trace.id,
            ],
        )?;
        Ok(())
    }

    pub fn get_cron_trace(&self, id: &str) -> Result<Option<CronTrace>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, job_id, job_name, started_at, completed_at, status, \
             events, summary, token_count, tool_call_count \
             FROM cron_execution_traces WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_trace(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_cron_traces(&self, job_id: Option<&str>, limit: usize) -> Result<Vec<CronTrace>> {
        let conn = self.lock_conn()?;
        let (sql, filter_params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) =
            if let Some(jid) = job_id {
                (
                    "SELECT id, job_id, job_name, started_at, completed_at, status, \
                     events, summary, token_count, tool_call_count \
                     FROM cron_execution_traces WHERE job_id = ?1 \
                     ORDER BY started_at DESC LIMIT ?2",
                    vec![Box::new(jid.to_string()), Box::new(limit as i64)],
                )
            } else {
                (
                    "SELECT id, job_id, job_name, started_at, completed_at, status, \
                     events, summary, token_count, tool_call_count \
                     FROM cron_execution_traces \
                     ORDER BY started_at DESC LIMIT ?1",
                    vec![Box::new(limit as i64)],
                )
            };

        let mut stmt = conn.prepare(sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            filter_params.iter().map(AsRef::as_ref).collect();
        let mut traces = Vec::new();
        let mut rows = stmt.query(params_ref.as_slice())?;
        while let Some(row) = rows.next()? {
            traces.push(row_to_trace(row)?);
        }
        Ok(traces)
    }

    pub fn purge_old_cron_traces(&self, keep_count: usize) -> Result<usize> {
        let conn = self.lock_conn()?;
        let deleted = conn.execute(
            "DELETE FROM cron_execution_traces WHERE id NOT IN (
                SELECT id FROM cron_execution_traces ORDER BY started_at DESC LIMIT ?1
            )",
            params![keep_count as i64],
        )?;
        Ok(deleted)
    }
}

fn row_to_trace(row: &rusqlite::Row<'_>) -> Result<CronTrace> {
    let events_json: String = row.get(6)?;
    let events: Vec<TraceEvent> = serde_json::from_str(&events_json)?;
    Ok(CronTrace {
        id: row.get(0)?,
        job_id: row.get(1)?,
        job_name: row.get(2)?,
        started_at: row.get(3)?,
        completed_at: row.get(4)?,
        status: row.get(5)?,
        events,
        summary: row.get(7)?,
        token_count: row.get(8)?,
        tool_call_count: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> MemoryDB {
        MemoryDB::new(":memory:").expect("in-memory DB")
    }

    #[test]
    fn insert_and_get_trace() {
        let db = test_db();
        let mut trace = CronTrace::new("trace-1".into(), "job-1".into(), "test job".into());
        trace.add_event(TraceEvent::Started {
            message: "hello".into(),
        });
        trace.add_event(TraceEvent::ToolCall {
            name: "cron".into(),
            params_summary: "{}".into(),
        });
        db.insert_cron_trace(&trace).unwrap();

        let loaded = db.get_cron_trace("trace-1").unwrap().unwrap();
        assert_eq!(loaded.id, "trace-1");
        assert_eq!(loaded.job_id, "job-1");
        assert_eq!(loaded.status, "running");
        assert_eq!(loaded.events.len(), 2);
        assert_eq!(loaded.tool_call_count, 1);
    }

    #[test]
    fn update_trace() {
        let db = test_db();
        let mut trace = CronTrace::new("trace-2".into(), "job-2".into(), "test job 2".into());
        db.insert_cron_trace(&trace).unwrap();

        trace.complete(Some("done".into()));
        db.update_cron_trace(&trace).unwrap();

        let loaded = db.get_cron_trace("trace-2").unwrap().unwrap();
        assert_eq!(loaded.status, "completed");
        assert!(loaded.completed_at.is_some());
        assert_eq!(loaded.summary.as_deref(), Some("done"));
    }

    #[test]
    fn list_traces_with_filter() {
        let db = test_db();
        for i in 0..5 {
            let job_id = if i < 3 { "job-a" } else { "job-b" };
            let trace = CronTrace::new(format!("trace-{i}"), job_id.into(), "test".into());
            db.insert_cron_trace(&trace).unwrap();
        }

        let all = db.list_cron_traces(None, 100).unwrap();
        assert_eq!(all.len(), 5);

        let filtered = db.list_cron_traces(Some("job-a"), 100).unwrap();
        assert_eq!(filtered.len(), 3);

        let limited = db.list_cron_traces(None, 2).unwrap();
        assert_eq!(limited.len(), 2);
    }

    #[test]
    fn purge_old_traces() {
        let db = test_db();
        for i in 0..10 {
            let trace = CronTrace::new(format!("trace-{i}"), "job-1".into(), "test".into());
            db.insert_cron_trace(&trace).unwrap();
        }

        let purged = db.purge_old_cron_traces(3).unwrap();
        assert_eq!(purged, 7);

        let remaining = db.list_cron_traces(None, 100).unwrap();
        assert_eq!(remaining.len(), 3);
    }

    #[test]
    fn trace_fail() {
        let db = test_db();
        let mut trace = CronTrace::new("trace-f".into(), "job-f".into(), "failing job".into());
        trace.fail("something went wrong");
        db.insert_cron_trace(&trace).unwrap();

        let loaded = db.get_cron_trace("trace-f").unwrap().unwrap();
        assert_eq!(loaded.status, "failed");
        assert!(loaded.completed_at.is_some());
        assert!(matches!(
            loaded.events.last(),
            Some(TraceEvent::Error { .. })
        ));
    }
}
