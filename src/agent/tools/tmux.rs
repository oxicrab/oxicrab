use crate::agent::tools::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tracing::debug;

const SOCKET_DIR: &str = "nanobot-tmux-sockets";
const SOCKET_NAME: &str = "nanobot.sock";

fn get_socket_path() -> PathBuf {
    std::env::temp_dir().join(SOCKET_DIR).join(SOCKET_NAME)
}

pub struct TmuxTool;

impl TmuxTool {
    pub fn new() -> Self {
        Self
    }

    async fn run_tmux(&self, args: &[&str]) -> Result<(i32, String, String)> {
        let socket_path = get_socket_path();
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let output = Command::new("tmux")
            .arg("-S")
            .arg(socket_path.as_os_str())
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Ok((output.status.code().unwrap_or(1), stdout, stderr))
    }
}

#[async_trait]
impl Tool for TmuxTool {
    fn name(&self) -> &str {
        "tmux"
    }

    fn description(&self) -> &str {
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

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        // Check if tmux is available
        if Command::new("tmux").arg("-V").output().await.is_err() {
            return Ok(ToolResult::error("Error: tmux is not installed or not found on PATH".to_string()));
        }

        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        match action {
            "create" => {
                let session_name = params["session_name"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_name' parameter for create"))?;

                let (code, _stdout, stderr) = self.run_tmux(&["new-session", "-d", "-s", session_name]).await?;
                if code != 0 {
                    return Ok(ToolResult::error(format!("Error: Failed to create session '{}': {}", session_name, stderr)));
                }
                debug!("tmux session '{}' created via socket {}", session_name, get_socket_path().display());
                Ok(ToolResult::new(format!("Session '{}' created", session_name)))
            }
            "send" => {
                let session_name = params["session_name"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_name' parameter for send"))?;
                let command = params["command"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter for send"))?;

                let (code, _, stderr) = self.run_tmux(&["send-keys", "-t", session_name, command, "Enter"]).await?;
                if code != 0 {
                    return Ok(ToolResult::error(format!("Error: Failed to send command to '{}': {}", session_name, stderr)));
                }
                Ok(ToolResult::new(format!("Command sent to session '{}'", session_name)))
            }
            "read" => {
                let session_name = params["session_name"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_name' parameter for read"))?;
                let lines = params["lines"].as_u64().unwrap_or(50) as i32;

                let (code, stdout, stderr) = self.run_tmux(&["capture-pane", "-t", session_name, "-p", "-S", &format!("-{}", lines)]).await?;
                if code != 0 {
                    return Ok(ToolResult::error(format!("Error: Failed to read session '{}': {}", session_name, stderr)));
                }
                let output = stdout.trim();
                Ok(ToolResult::new(if output.is_empty() { "(no output)".to_string() } else { output.to_string() }))
            }
            "list" => {
                let (code, stdout, stderr) = self.run_tmux(&["list-sessions"]).await?;
                if code != 0 {
                    if stderr.contains("no server running") || stderr.contains("no sessions") {
                        return Ok(ToolResult::new("No active sessions".to_string()));
                    }
                    return Ok(ToolResult::error(format!("Error: Failed to list sessions: {}", stderr)));
                }
                let output = stdout.trim();
                Ok(ToolResult::new(if output.is_empty() { "No active sessions".to_string() } else { output.to_string() }))
            }
            "kill" => {
                let session_name = params["session_name"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_name' parameter for kill"))?;

                let (code, _, stderr) = self.run_tmux(&["kill-session", "-t", session_name]).await?;
                if code != 0 {
                    return Ok(ToolResult::error(format!("Error: Failed to kill session '{}': {}", session_name, stderr)));
                }
                debug!("tmux session '{}' killed", session_name);
                Ok(ToolResult::new(format!("Session '{}' killed", session_name)))
            }
            _ => Ok(ToolResult::error(format!("Error: Unknown action '{}'", action))),
        }
    }
}
