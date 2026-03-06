use super::MemoryDB;
use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct SubagentLogEntry {
    pub id: i64,
    pub task_id: String,
    pub timestamp: String,
    pub event_type: String,
    pub content: String,
    pub metadata: Option<String>,
}

impl MemoryDB {
    /// Insert a subagent log entry.
    pub fn insert_subagent_log(
        &self,
        task_id: &str,
        event_type: &str,
        content: &str,
        metadata: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT INTO subagent_logs (task_id, event_type, content, metadata)
             VALUES (?1, ?2, ?3, ?4)",
            params![task_id, event_type, content, metadata],
        )?;
        Ok(())
    }

    /// List all log entries for a task, ordered by id.
    pub fn list_subagent_logs(&self, task_id: &str) -> Result<Vec<SubagentLogEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, timestamp, event_type, content, metadata
             FROM subagent_logs WHERE task_id = ?1 ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![task_id], |row| {
                Ok(SubagentLogEntry {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    timestamp: row.get(2)?,
                    event_type: row.get(3)?,
                    content: row.get(4)?,
                    metadata: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// List distinct `task_ids`, most recent first.
    pub fn list_recent_subagent_tasks(&self, limit: usize) -> Result<Vec<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT task_id FROM subagent_logs
             GROUP BY task_id ORDER BY MAX(id) DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Delete logs for tasks beyond the most recent N. Returns count deleted.
    pub fn purge_old_subagent_logs(&self, keep_tasks: usize) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let deleted = conn.execute(
            "DELETE FROM subagent_logs WHERE task_id NOT IN (
                SELECT task_id FROM subagent_logs
                GROUP BY task_id ORDER BY MAX(id) DESC LIMIT ?1
            )",
            params![keep_tasks as i64],
        )?;
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::super::MemoryDB;

    #[test]
    fn test_insert_and_list_subagent_logs() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.insert_subagent_log("task-1", "start", "SUBAGENT START task_id=task-1", None)
            .unwrap();
        db.insert_subagent_log("task-1", "tools", "TOOLS REGISTERED: exec, read_file", None)
            .unwrap();
        db.insert_subagent_log(
            "task-1",
            "end",
            "SUBAGENT END task_id=task-1 status=ok duration=1.2s",
            Some(r#"{"status":"ok","duration_secs":1.2}"#),
        )
        .unwrap();

        let entries = db.list_subagent_logs("task-1").unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].event_type, "start");
        assert_eq!(entries[1].event_type, "tools");
        assert_eq!(entries[2].event_type, "end");
        assert!(entries[2].metadata.is_some());
    }

    #[test]
    fn test_list_subagent_logs_empty() {
        let db = MemoryDB::new(":memory:").unwrap();
        let entries = db.list_subagent_logs("nonexistent").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_list_recent_subagent_tasks() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.insert_subagent_log("task-a", "start", "start a", None)
            .unwrap();
        db.insert_subagent_log("task-b", "start", "start b", None)
            .unwrap();
        db.insert_subagent_log("task-c", "start", "start c", None)
            .unwrap();

        let tasks = db.list_recent_subagent_tasks(2).unwrap();
        assert_eq!(tasks.len(), 2);
        // Most recent first
        assert_eq!(tasks[0], "task-c");
        assert_eq!(tasks[1], "task-b");
    }

    #[test]
    fn test_purge_old_subagent_logs() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.insert_subagent_log("task-old", "start", "old start", None)
            .unwrap();
        db.insert_subagent_log("task-old", "end", "old end", None)
            .unwrap();
        db.insert_subagent_log("task-mid", "start", "mid start", None)
            .unwrap();
        db.insert_subagent_log("task-new", "start", "new start", None)
            .unwrap();

        let deleted = db.purge_old_subagent_logs(2).unwrap();
        assert_eq!(deleted, 2); // task-old had 2 entries

        let remaining = db.list_recent_subagent_tasks(10).unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.contains(&"task-new".to_string()));
        assert!(remaining.contains(&"task-mid".to_string()));
        assert!(!remaining.contains(&"task-old".to_string()));
    }

    #[test]
    fn test_purge_keeps_all_when_under_limit() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.insert_subagent_log("task-1", "start", "s1", None)
            .unwrap();
        db.insert_subagent_log("task-2", "start", "s2", None)
            .unwrap();

        let deleted = db.purge_old_subagent_logs(50).unwrap();
        assert_eq!(deleted, 0);

        let tasks = db.list_recent_subagent_tasks(50).unwrap();
        assert_eq!(tasks.len(), 2);
    }

    #[test]
    fn test_metadata_is_optional() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.insert_subagent_log("task-1", "iteration", "iter 1", None)
            .unwrap();
        db.insert_subagent_log(
            "task-1",
            "tool_call",
            "TOOL CALL: exec",
            Some(r#"{"tool":"exec"}"#),
        )
        .unwrap();

        let entries = db.list_subagent_logs("task-1").unwrap();
        assert!(entries[0].metadata.is_none());
        assert!(entries[1].metadata.is_some());
    }
}
