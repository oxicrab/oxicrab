use chrono::Utc;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use tracing::warn;

/// Per-subagent activity log written to `~/.oxicrab/logs/`.
///
/// Captures the full execution trace: task, registered tools, each LLM
/// iteration, tool calls with arguments and results, and the final outcome.
pub struct ActivityLog {
    writer: BufWriter<File>,
    task_id: String,
    start: std::time::Instant,
    path: PathBuf,
}

impl ActivityLog {
    /// Create a new activity log. Returns `None` if the log directory or file
    /// cannot be created (non-fatal — subagent proceeds without logging).
    pub fn new(task_id: &str) -> Option<Self> {
        let home = dirs::home_dir()?;
        let log_dir = home.join(".oxicrab/logs");
        if let Err(e) = fs::create_dir_all(&log_dir) {
            warn!(
                "failed to create subagent log directory {:?}: {}",
                log_dir, e
            );
            return None;
        }
        let date = Utc::now().format("%Y%m%d-%H%M%S");
        let file_path = log_dir.join(format!("subagent-{}-{}.log", task_id, date));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .map_err(|e| {
                warn!("failed to open subagent log {:?}: {}", file_path, e);
                e
            })
            .ok()?;
        Some(Self {
            writer: BufWriter::new(file),
            task_id: task_id.to_string(),
            start: std::time::Instant::now(),
            path: file_path,
        })
    }

    /// Path to the log file (for reporting to users).
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn write_line(&mut self, msg: &str) {
        let ts = Utc::now().format("%H:%M:%S%.3fZ");
        let _ = writeln!(self.writer, "[{}] {}", ts, msg);
        let _ = self.writer.flush();
    }

    pub fn log_start(&mut self, task: &str) {
        self.write_line(&format!("SUBAGENT START task_id={}", self.task_id));
        self.write_line(&format!("TASK: {}", task));
    }

    pub fn log_tools(&mut self, registered: &[String], blocked: &[String]) {
        self.write_line(&format!("TOOLS REGISTERED: {}", registered.join(", ")));
        if blocked.is_empty() {
            self.write_line("TOOLS BLOCKED: (none)");
        } else {
            self.write_line(&format!("TOOLS BLOCKED: {}", blocked.join(", ")));
        }
    }

    pub fn log_iteration_tool_calls(&mut self, iteration: usize, count: usize) {
        self.write_line(&format!(
            "ITERATION {}: LLM responded with {} tool call(s)",
            iteration, count
        ));
    }

    pub fn log_iteration_text(&mut self, iteration: usize, content_len: usize) {
        self.write_line(&format!(
            "ITERATION {}: LLM responded with text ({} chars) — final result",
            iteration, content_len
        ));
    }

    pub fn log_iteration_empty(&mut self, iteration: usize, retries_left: usize) {
        self.write_line(&format!(
            "ITERATION {}: LLM returned empty response (retries left: {})",
            iteration, retries_left
        ));
    }

    pub fn log_tool_call(&mut self, name: &str, args: &serde_json::Value) {
        let args_str = serde_json::to_string(args).unwrap_or_default();
        let preview: String = args_str.chars().take(500).collect();
        self.write_line(&format!("  TOOL CALL: {} {}", name, preview));
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
        self.write_line(&format!(
            "{}: {} ({} chars): {}{}",
            prefix,
            name,
            content.len(),
            preview,
            suffix
        ));
    }

    pub fn log_cost_blocked(&mut self, msg: &str) {
        self.write_line(&format!("COST GUARD BLOCKED: {}", msg));
    }

    pub fn log_max_iterations(&mut self, max: usize) {
        self.write_line(&format!(
            "MAX ITERATIONS REACHED ({}) — exiting without final response",
            max
        ));
    }

    pub fn log_end(&mut self, status: &str) {
        let elapsed = self.start.elapsed();
        self.write_line(&format!(
            "SUBAGENT END task_id={} status={} duration={:.1}s",
            self.task_id,
            status,
            elapsed.as_secs_f64()
        ));
    }
}
