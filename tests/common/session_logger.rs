//! Opt-in session logger for integration tests.
//!
//! Records tool calls, tool results, LLM requests, and LLM responses
//! during a test run, then writes a human-readable log to a temp file.

use serde_json::Value;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum TestEvent {
    ToolCall {
        name: String,
        params: Value,
        timestamp: Instant,
    },
    ToolResult {
        name: String,
        content: String,
        is_error: bool,
        duration: Duration,
    },
    LlmRequest {
        message_count: usize,
    },
    LlmResponse {
        content: String,
        tool_calls: usize,
    },
}

/// Records events during a test run for debugging and post-mortem analysis.
/// Thread-safe via interior `Mutex`. Create one per test, pass it around.
pub struct TestSessionLogger {
    events: Mutex<Vec<TestEvent>>,
    test_name: String,
    start: Instant,
}

impl TestSessionLogger {
    pub fn new(test_name: &str) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            test_name: test_name.to_string(),
            start: Instant::now(),
        }
    }

    pub fn log_tool_call(&self, name: &str, params: &Value) {
        self.events
            .lock()
            .expect("lock events")
            .push(TestEvent::ToolCall {
                name: name.to_string(),
                params: params.clone(),
                timestamp: Instant::now(),
            });
    }

    pub fn log_tool_result(&self, name: &str, content: &str, is_error: bool, duration: Duration) {
        self.events
            .lock()
            .expect("lock events")
            .push(TestEvent::ToolResult {
                name: name.to_string(),
                content: content.to_string(),
                is_error,
                duration,
            });
    }

    pub fn log_llm_request(&self, message_count: usize) {
        self.events
            .lock()
            .expect("lock events")
            .push(TestEvent::LlmRequest { message_count });
    }

    pub fn log_llm_response(&self, content: &str, tool_calls: usize) {
        self.events
            .lock()
            .expect("lock events")
            .push(TestEvent::LlmResponse {
                content: content.to_string(),
                tool_calls,
            });
    }

    /// One-line summary of the session: counts of each event type.
    pub fn summary(&self) -> String {
        let events = self.events.lock().expect("lock events");
        let mut tool_calls = 0usize;
        let mut llm_requests = 0usize;
        let mut errors = 0usize;

        for event in events.iter() {
            match event {
                TestEvent::ToolCall { .. } => tool_calls += 1,
                TestEvent::ToolResult { is_error, .. } if *is_error => errors += 1,
                TestEvent::LlmRequest { .. } => llm_requests += 1,
                _ => {}
            }
        }

        format!(
            "{} tool calls, {} LLM requests, {} errors",
            tool_calls, llm_requests, errors
        )
    }

    /// Write a human-readable log file under the system temp directory.
    /// Returns the path to the written file.
    pub fn write_log(&self) -> std::io::Result<std::path::PathBuf> {
        let log_dir = std::env::temp_dir().join("oxicrab-test-logs");
        std::fs::create_dir_all(&log_dir)?;

        let path = log_dir.join(format!("{}.log", self.test_name));
        let events = self.events.lock().expect("lock events");

        let mut lines = Vec::with_capacity(events.len() + 4);
        lines.push(format!("# Test: {}", self.test_name));
        lines.push(format!("# Events: {}", events.len()));
        lines.push(format!("# Summary: {}", self.summary_inner(&events)));
        lines.push(String::new());

        for (i, event) in events.iter().enumerate() {
            let elapsed = match event {
                TestEvent::ToolCall { timestamp, .. } => timestamp.duration_since(self.start),
                _ => Duration::ZERO,
            };

            let line = match event {
                TestEvent::ToolCall { name, params, .. } => {
                    let params_str =
                        serde_json::to_string(params).unwrap_or_else(|_| "???".to_string());
                    let truncated = if params_str.len() > 200 {
                        format!("{}...", &params_str[..200])
                    } else {
                        params_str
                    };
                    format!(
                        "[{:>6}ms] #{:>3} TOOL_CALL  {} {}",
                        elapsed.as_millis(),
                        i + 1,
                        name,
                        truncated
                    )
                }
                TestEvent::ToolResult {
                    name,
                    content,
                    is_error,
                    duration,
                } => {
                    let status = if *is_error { "ERR" } else { "OK" };
                    let truncated = if content.len() > 100 {
                        format!("{}...", &content[..100])
                    } else {
                        content.clone()
                    };
                    format!(
                        "          #{:>3} TOOL_RESULT {} [{}] ({}ms) {}",
                        i + 1,
                        name,
                        status,
                        duration.as_millis(),
                        truncated.replace('\n', " ")
                    )
                }
                TestEvent::LlmRequest { message_count } => {
                    format!(
                        "          #{:>3} LLM_REQ    messages={}",
                        i + 1,
                        message_count
                    )
                }
                TestEvent::LlmResponse {
                    content,
                    tool_calls,
                } => {
                    let truncated = if content.len() > 100 {
                        format!("{}...", &content[..100])
                    } else {
                        content.clone()
                    };
                    format!(
                        "          #{:>3} LLM_RESP   tool_calls={} \"{}\"",
                        i + 1,
                        tool_calls,
                        truncated.replace('\n', " ")
                    )
                }
            };
            lines.push(line);
        }

        std::fs::write(&path, lines.join("\n"))?;
        Ok(path)
    }

    fn summary_inner(&self, events: &[TestEvent]) -> String {
        let mut tool_calls = 0usize;
        let mut llm_requests = 0usize;
        let mut errors = 0usize;

        for event in events {
            match event {
                TestEvent::ToolCall { .. } => tool_calls += 1,
                TestEvent::ToolResult { is_error, .. } if *is_error => errors += 1,
                TestEvent::LlmRequest { .. } => llm_requests += 1,
                _ => {}
            }
        }

        format!(
            "{} tool calls, {} LLM requests, {} errors",
            tool_calls, llm_requests, errors
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summary_counts_correctly() {
        let logger = TestSessionLogger::new("test_summary");

        logger.log_tool_call("echo", &json!({"text": "hi"}));
        logger.log_tool_result("echo", "Echo: hi", false, Duration::from_millis(5));
        logger.log_llm_request(3);
        logger.log_llm_response("hello", 0);
        logger.log_tool_call("fail_tool", &json!({}));
        logger.log_tool_result(
            "fail_tool",
            "error occurred",
            true,
            Duration::from_millis(10),
        );

        assert_eq!(logger.summary(), "2 tool calls, 1 LLM requests, 1 errors");
    }

    #[test]
    fn write_log_creates_file() {
        let logger = TestSessionLogger::new("test_write_log");

        logger.log_llm_request(1);
        logger.log_llm_response("hi there", 0);
        logger.log_tool_call("echo", &json!({"text": "hello"}));
        logger.log_tool_result("echo", "Echo: hello", false, Duration::from_millis(2));

        let path = logger.write_log().expect("write log");
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).expect("read log");
        assert!(content.contains("# Test: test_write_log"));
        assert!(content.contains("LLM_REQ"));
        assert!(content.contains("TOOL_CALL"));
        assert!(content.contains("TOOL_RESULT"));

        // Cleanup
        let _ = std::fs::remove_file(&path);
    }
}
