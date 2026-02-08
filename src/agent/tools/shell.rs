use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use crate::utils::regex::compile_security_patterns;
use anyhow::{Context, Result};
use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

pub struct ExecTool {
    timeout: u64,
    working_dir: Option<PathBuf>,
    deny_patterns: Vec<Regex>,
    restrict_to_workspace: bool,
}

impl ExecTool {
    pub fn new(timeout: u64, working_dir: Option<PathBuf>, restrict_to_workspace: bool) -> Self {
        // Compile security patterns with proper error handling
        // If compilation fails, we'll log a warning but continue with empty patterns
        // This is a safety measure - in production, this should never fail
        let deny_patterns = compile_security_patterns()
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to compile security patterns: {}. Tool will have reduced security.", e);
                Vec::new()
            });

        Self {
            timeout,
            working_dir,
            deny_patterns,
            restrict_to_workspace,
        }
    }

    fn guard_command(&self, command: &str, cwd: &Path) -> Option<String> {
        for pattern in &self.deny_patterns {
            if pattern.is_match(command) {
                return Some(format!(
                    "Error: Command blocked by security policy: {}",
                    command
                ));
            }
        }

        if self.restrict_to_workspace {
            if let Some(workspace) = &self.working_dir {
                if !cwd.starts_with(workspace) {
                    return Some(format!(
                        "Error: Working directory '{}' is outside workspace",
                        cwd.display()
                    ));
                }
            }
        }

        None
    }
}

#[async_trait]
impl Tool for ExecTool {
    fn name(&self) -> &str {
        "exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output. Use with caution."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Optional working directory for the command"
                }
            },
            "required": ["command"]
        })
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 1, 0) // Version 1.1.0 - includes security improvements
    }

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let command = params["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter"))?;

        let working_dir = params["working_dir"]
            .as_str()
            .map(PathBuf::from)
            .or_else(|| self.working_dir.clone());

        let cwd = working_dir.unwrap_or_else(|| {
            std::env::current_dir()
                .context("Failed to get current directory")
                .unwrap_or_else(|e| {
                    tracing::warn!("Failed to get current directory: {}, using '.'", e);
                    PathBuf::from(".")
                })
        });

        if let Some(err) = self.guard_command(command, &cwd) {
            return Ok(ToolResult::error(err));
        }

        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd.current_dir(&cwd);
        cmd.kill_on_drop(true);

        match tokio::time::timeout(Duration::from_secs(self.timeout), cmd.output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                let mut result = String::new();
                if !stdout.is_empty() {
                    result.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !result.is_empty() {
                        result.push_str("\n--- stderr ---\n");
                    }
                    result.push_str(&stderr);
                }

                if output.status.success() {
                    Ok(ToolResult::new(if result.is_empty() {
                        "(no output)".to_string()
                    } else {
                        result
                    }))
                } else {
                    Ok(ToolResult::error(format!("Command failed: {}", result)))
                }
            }
            Ok(Err(e)) => Ok(ToolResult::error(format!("Error executing command: {}", e))),
            Err(_) => Ok(ToolResult::error(format!(
                "Command timed out after {} seconds",
                self.timeout
            ))),
        }
    }
}
