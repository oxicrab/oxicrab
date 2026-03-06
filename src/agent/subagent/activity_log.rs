use crate::agent::memory::memory_db::MemoryDB;
use std::sync::Arc;
use tracing::warn;

/// Per-subagent activity log backed by `MemoryDB`.
///
/// Captures the full execution trace: task, registered tools, each LLM
/// iteration, tool calls with arguments and results, and the final outcome.
pub struct ActivityLog {
    db: Arc<MemoryDB>,
    task_id: String,
    start: std::time::Instant,
}

impl ActivityLog {
    /// Create a new activity log. Returns `None` if the initial start marker
    /// cannot be written (non-fatal — subagent proceeds without logging).
    pub fn new(task_id: &str, db: Arc<MemoryDB>) -> Option<Self> {
        // Verify the DB is usable by writing a sentinel; if this fails,
        // return None so the subagent continues without logging.
        if let Err(e) = db.insert_subagent_log(task_id, "start", "activity log initialized", None) {
            warn!("failed to initialize subagent activity log: {e}");
            return None;
        }
        Some(Self {
            db,
            task_id: task_id.to_string(),
            start: std::time::Instant::now(),
        })
    }

    fn write(&self, event_type: &str, content: &str, metadata: Option<&str>) {
        if let Err(e) = self
            .db
            .insert_subagent_log(&self.task_id, event_type, content, metadata)
        {
            warn!("subagent [{}] failed to write log entry: {e}", self.task_id);
        }
    }

    pub fn log_start(&mut self, task: &str) {
        self.write(
            "start",
            &format!("SUBAGENT START task_id={}", self.task_id),
            None,
        );
        self.write("start", &format!("TASK: {task}"), None);
    }

    pub fn log_tools(&mut self, registered: &[String]) {
        self.write(
            "tools",
            &format!("TOOLS REGISTERED: {}", registered.join(", ")),
            None,
        );
    }

    pub fn log_iteration_tool_calls(&mut self, iteration: usize, count: usize) {
        self.write(
            "iteration",
            &format!("ITERATION {iteration}: LLM responded with {count} tool call(s)"),
            Some(&format!(
                r#"{{"iteration":{iteration},"tool_calls":{count}}}"#
            )),
        );
    }

    pub fn log_iteration_text(&mut self, iteration: usize, content_len: usize) {
        self.write(
            "iteration",
            &format!(
                "ITERATION {iteration}: LLM responded with text ({content_len} chars) — final result"
            ),
            Some(&format!(
                r#"{{"iteration":{iteration},"content_len":{content_len}}}"#
            )),
        );
    }

    pub fn log_iteration_empty(&mut self, iteration: usize, retries_left: usize) {
        self.write(
            "iteration",
            &format!(
                "ITERATION {iteration}: LLM returned empty response (retries left: {retries_left})"
            ),
            Some(&format!(
                r#"{{"iteration":{iteration},"retries_left":{retries_left}}}"#
            )),
        );
    }

    pub fn log_tool_call(&mut self, name: &str, args: &serde_json::Value) {
        let args_str = serde_json::to_string(args).unwrap_or_default();
        let preview: String = args_str.chars().take(500).collect();
        self.write(
            "tool_call",
            &format!("  TOOL CALL: {name} {preview}"),
            Some(&format!(r#"{{"tool":"{name}"}}"#)),
        );
    }

    pub fn log_tool_result(&mut self, name: &str, content: &str, is_error: bool) {
        let prefix = if is_error {
            "  TOOL ERROR"
        } else {
            "  TOOL RESULT"
        };
        let preview: String = content.chars().take(500).collect();
        let suffix = if content.chars().count() > 500 {
            "..."
        } else {
            ""
        };
        self.write(
            "tool_result",
            &format!(
                "{}: {} ({} chars): {}{}",
                prefix,
                name,
                content.len(),
                preview,
                suffix
            ),
            Some(&format!(
                r#"{{"tool":"{name}","is_error":{is_error},"content_len":{}}}"#,
                content.len()
            )),
        );
    }

    pub fn log_max_iterations(&mut self, max: usize) {
        self.write(
            "max_iterations",
            &format!("MAX ITERATIONS REACHED ({max}) — exiting without final response"),
            Some(&format!(r#"{{"max_iterations":{max}}}"#)),
        );
    }

    pub fn log_end(&mut self, status: &str) {
        let elapsed = self.start.elapsed();
        self.write(
            "end",
            &format!(
                "SUBAGENT END task_id={} status={} duration={:.1}s",
                self.task_id,
                status,
                elapsed.as_secs_f64()
            ),
            Some(&format!(
                r#"{{"status":"{status}","duration_secs":{:.1}}}"#,
                elapsed.as_secs_f64()
            )),
        );
        // Auto-purge old task logs, keeping the most recent 50
        if let Err(e) = self.db.purge_old_subagent_logs(50) {
            warn!("subagent [{}] failed to purge old logs: {e}", self.task_id);
        }
    }
}
