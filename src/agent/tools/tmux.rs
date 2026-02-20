use crate::agent::tools::base::ExecutionContext;
use crate::agent::tools::{Tool, ToolResult};
use crate::utils::regex::compile_security_patterns;
use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::LazyLock;
use tracing::debug;

/// Regex to validate tmux session names: only allow safe characters.
/// Prevents injection via `:` (session:window separator) or `.` (window.pane separator).
static SAFE_SESSION_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap());

const SOCKET_DIR: &str = "oxicrab-tmux-sockets";
const SOCKET_NAME: &str = "oxicrab.sock";
/// Maximum combined output size before truncation (1 MB).
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

fn get_socket_path() -> PathBuf {
    std::env::temp_dir().join(SOCKET_DIR).join(SOCKET_NAME)
}

pub struct TmuxTool {
    deny_patterns: Vec<Regex>,
}

impl Default for TmuxTool {
    fn default() -> Self {
        Self::new()
    }
}

impl TmuxTool {
    pub fn new() -> Self {
        let deny_patterns = compile_security_patterns().unwrap_or_default();
        Self { deny_patterns }
    }

    async fn run_tmux(&self, args: &[&str]) -> Result<(i32, String, String)> {
        let socket_path = get_socket_path();
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let output = crate::utils::subprocess::scrubbed_command("tmux")
            .arg("-S")
            .arg(socket_path.as_os_str())
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = crate::utils::path_sanitize::sanitize_error_message(
            &String::from_utf8_lossy(&output.stderr),
            None,
        );
        Ok((output.status.code().unwrap_or(1), stdout, stderr))
    }

    fn is_session_missing(stderr: &str) -> bool {
        stderr.contains("No such file or directory")
            || stderr.contains("no server running")
            || stderr.contains("can't find session")
    }

    async fn ensure_session(&self, session_name: &str) -> Result<()> {
        let (code, _, stderr) = self.run_tmux(&["has-session", "-t", session_name]).await?;
        if code != 0 && Self::is_session_missing(&stderr) {
            debug!("Auto-creating missing tmux session '{}'", session_name);
            self.run_tmux(&["new-session", "-d", "-s", session_name])
                .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl Tool for TmuxTool {
    fn name(&self) -> &'static str {
        "tmux"
    }

    fn description(&self) -> &'static str {
        "Manage persistent tmux shell sessions. Create long-running sessions, send commands, and read output."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "send", "read", "list", "kill"],
                    "description": "The tmux action to perform"
                },
                "session_name": {
                    "type": "string",
                    "description": "Session name (required for create/send/read/kill)"
                },
                "command": {
                    "type": "string",
                    "description": "Command to send (required for send action)"
                },
                "lines": {
                    "type": "integer",
                    "description": "Number of lines to capture (default 50, for read action)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        // Check if tmux is available
        if tokio::process::Command::new("tmux")
            .arg("-V")
            .output()
            .await
            .is_err()
        {
            return Ok(ToolResult::error(
                "tmux is not installed or not found on PATH".to_string(),
            ));
        }

        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        match action {
            "create" => {
                let session_name = params["session_name"].as_str().ok_or_else(|| {
                    anyhow::anyhow!("Missing 'session_name' parameter for create")
                })?;
                if !SAFE_SESSION_NAME.is_match(session_name) {
                    return Ok(ToolResult::error(
                        "session name must contain only alphanumeric characters, hyphens, and underscores".to_string(),
                    ));
                }

                let (code, _stdout, stderr) = self
                    .run_tmux(&["new-session", "-d", "-s", session_name])
                    .await?;
                if code != 0 {
                    return Ok(ToolResult::error(format!(
                        "failed to create session '{}': {}",
                        session_name, stderr
                    )));
                }
                debug!(
                    "tmux session '{}' created via socket {}",
                    session_name,
                    get_socket_path().display()
                );
                Ok(ToolResult::new(format!(
                    "Session '{}' created",
                    session_name
                )))
            }
            "send" => {
                let session_name = params["session_name"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_name' parameter for send"))?;
                if !SAFE_SESSION_NAME.is_match(session_name) {
                    return Ok(ToolResult::error(
                        "session name must contain only alphanumeric characters, hyphens, and underscores".to_string(),
                    ));
                }
                let command = params["command"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter for send"))?;

                // Security check: use the same regex blocklist as the exec/shell tool
                for pattern in &self.deny_patterns {
                    if pattern.is_match(command) {
                        return Ok(ToolResult::error(format!(
                            "command blocked by security policy: {}",
                            command
                        )));
                    }
                }

                self.ensure_session(session_name).await?;

                let (code, _, stderr) = self
                    .run_tmux(&["send-keys", "-t", session_name, command, "Enter"])
                    .await?;
                if code != 0 {
                    return Ok(ToolResult::error(format!(
                        "failed to send command to '{}': {}",
                        session_name, stderr
                    )));
                }
                Ok(ToolResult::new(format!(
                    "Command sent to session '{}'",
                    session_name
                )))
            }
            "read" => {
                let session_name = params["session_name"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_name' parameter for read"))?;
                if !SAFE_SESSION_NAME.is_match(session_name) {
                    return Ok(ToolResult::error(
                        "session name must contain only alphanumeric characters, hyphens, and underscores".to_string(),
                    ));
                }
                let lines = params["lines"].as_u64().unwrap_or(50) as i32;

                self.ensure_session(session_name).await?;

                let (code, stdout, stderr) = self
                    .run_tmux(&[
                        "capture-pane",
                        "-t",
                        session_name,
                        "-p",
                        "-S",
                        &format!("-{}", lines),
                    ])
                    .await?;
                if code != 0 {
                    return Ok(ToolResult::error(format!(
                        "failed to read session '{}': {}",
                        session_name, stderr
                    )));
                }
                let output = stdout.trim();
                let output = if output.len() > MAX_OUTPUT_BYTES {
                    let truncated = &output[..output.floor_char_boundary(MAX_OUTPUT_BYTES)];
                    format!("{}\n[output truncated at 1MB]", truncated)
                } else {
                    output.to_string()
                };
                Ok(ToolResult::new(if output.is_empty() {
                    "(no output)".to_string()
                } else {
                    output
                }))
            }
            "list" => {
                let (code, stdout, stderr) = self.run_tmux(&["list-sessions"]).await?;
                if code != 0 {
                    if stderr.contains("no server running") || stderr.contains("no sessions") {
                        return Ok(ToolResult::new("No active sessions".to_string()));
                    }
                    return Ok(ToolResult::error(format!(
                        "failed to list sessions: {}",
                        stderr
                    )));
                }
                let output = stdout.trim();
                Ok(ToolResult::new(if output.is_empty() {
                    "No active sessions".to_string()
                } else {
                    output.to_string()
                }))
            }
            "kill" => {
                let session_name = params["session_name"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_name' parameter for kill"))?;
                if !SAFE_SESSION_NAME.is_match(session_name) {
                    return Ok(ToolResult::error(
                        "session name must contain only alphanumeric characters, hyphens, and underscores".to_string(),
                    ));
                }

                let (code, _, stderr) =
                    self.run_tmux(&["kill-session", "-t", session_name]).await?;
                if code != 0 {
                    return Ok(ToolResult::error(format!(
                        "failed to kill session '{}': {}",
                        session_name, stderr
                    )));
                }
                debug!("tmux session '{}' killed", session_name);
                Ok(ToolResult::new(format!(
                    "Session '{}' killed",
                    session_name
                )))
            }
            _ => Ok(ToolResult::error(format!("unknown action '{}'", action))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmux_available() -> bool {
        std::process::Command::new("tmux")
            .arg("-V")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[test]
    fn test_is_session_missing_no_such_file() {
        assert!(TmuxTool::is_session_missing(
            "error connecting to /tmp/oxicrab-tmux-sockets/oxicrab.sock (No such file or directory)"
        ));
    }

    #[test]
    fn test_is_session_missing_no_server() {
        assert!(TmuxTool::is_session_missing(
            "no server running on /tmp/oxicrab-tmux-sockets/oxicrab.sock"
        ));
    }

    #[test]
    fn test_is_session_missing_cant_find() {
        assert!(TmuxTool::is_session_missing("can't find session: test"));
    }

    #[test]
    fn test_is_session_missing_other_error() {
        assert!(!TmuxTool::is_session_missing("some other error"));
    }

    #[test]
    fn test_socket_path() {
        let path = get_socket_path();
        assert!(path.ends_with("oxicrab-tmux-sockets/oxicrab.sock"));
    }

    #[tokio::test]
    async fn test_missing_action() {
        if !tmux_available() {
            return;
        }
        let tool = TmuxTool::new();
        let result = tool
            .execute(serde_json::json!({}), &ExecutionContext::default())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_unknown_action() {
        if !tmux_available() {
            return;
        }
        let tool = TmuxTool::new();
        let result = tool
            .execute(
                serde_json::json!({"action": "bogus"}),
                &ExecutionContext::default(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("unknown action"));
    }

    #[tokio::test]
    async fn test_send_missing_session_name() {
        if !tmux_available() {
            return;
        }
        let tool = TmuxTool::new();
        let result = tool
            .execute(
                serde_json::json!({"action": "send", "command": "echo hi"}),
                &ExecutionContext::default(),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_send_missing_command() {
        if !tmux_available() {
            return;
        }
        let tool = TmuxTool::new();
        let result = tool
            .execute(
                serde_json::json!({"action": "send", "session_name": "test"}),
                &ExecutionContext::default(),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_missing_session_name() {
        if !tmux_available() {
            return;
        }
        let tool = TmuxTool::new();
        let result = tool
            .execute(
                serde_json::json!({"action": "read"}),
                &ExecutionContext::default(),
            )
            .await;
        assert!(result.is_err());
    }
}
