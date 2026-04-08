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
mod tests;
