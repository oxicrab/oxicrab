use crate::utils::regex_utils::compile_security_patterns;
use anyhow::Result;
use async_trait::async_trait;
use oxicrab_core::actions;
use oxicrab_core::config::schema::SandboxConfig;
use oxicrab_core::require_param;
use oxicrab_core::tools::base::{
    ExecutionContext, ToolCapabilities, ToolCategory, ToolConcurrency,
};
use oxicrab_core::tools::base::{Tool, ToolResult};
use regex::Regex;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::LazyLock;
use tracing::debug;

/// Regex to validate tmux session names: only allow safe characters.
static SAFE_SESSION_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap());

const SOCKET_DIR: &str = "oxicrab-tmux-sockets";
const SOCKET_NAME: &str = "oxicrab.sock";
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

fn get_socket_path() -> PathBuf {
    std::env::temp_dir().join(SOCKET_DIR).join(SOCKET_NAME)
}

pub struct TmuxTool {
    deny_patterns: Vec<Regex>,
    sandbox_config: SandboxConfig,
}

impl TmuxTool {
    pub fn new(sandbox_config: SandboxConfig) -> Self {
        let deny_patterns = compile_security_patterns().unwrap_or_default();
        Self {
            deny_patterns,
            sandbox_config,
        }
    }

    async fn run_tmux(&self, args: &[&str]) -> Result<(i32, String, String)> {
        let socket_path = get_socket_path();
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut cmd = crate::utils::subprocess::scrubbed_command("tmux");
        cmd.arg("-S")
            .arg(socket_path.as_os_str())
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if self.sandbox_config.enabled {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/tmp"));
            let rules = crate::utils::sandbox::SandboxRules::for_shell(&cwd, &self.sandbox_config);
            crate::utils::sandbox::apply_to_command(&mut cmd, &rules)
                .map_err(|e| anyhow::anyhow!("sandbox is required but failed to apply: {e}"))?;
        }

        let output = cmd.output().await?;

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
        let target = exact_target(session_name);
        let (code, _, stderr) = self.run_tmux(&["has-session", "-t", &target]).await?;
        if code != 0 && Self::is_session_missing(&stderr) {
            debug!("auto-creating missing tmux session '{}'", session_name);
            self.run_tmux(&["new-session", "-d", "-s", session_name])
                .await?;
        }
        Ok(())
    }
}

/// Format a tmux target string that forces exact-match semantics.
///
/// Tmux's `-t` target accepts prefix / fnmatch matches by default, so
/// `has-session -t foo` would also match a session named `foo-prod`. The
/// `=` prefix forces exact name comparison. Reference:
/// <https://man.openbsd.org/tmux#TARGET-SESSION>
fn exact_target(session_name: &str) -> String {
    format!("={session_name}")
}

#[async_trait]
impl Tool for TmuxTool {
    fn name(&self) -> &str {
        "tmux"
    }

    fn description(&self) -> &str {
        "Manage persistent tmux shell sessions. Create long-running sessions, send commands, and read output."
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            category: ToolCategory::System,
            actions: actions![create, send, read: ro, list: ro, kill],
            concurrency: ToolConcurrency::Exclusive,
            ..Default::default()
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "send", "read", "list", "kill"],
                    "description": "The tmux action to perform. 'create' starts a new \
                     session. 'send' sends a command/text to a session. 'read' captures \
                     recent output lines. 'list' shows active sessions. 'kill' terminates a \
                     session."
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
        if crate::utils::subprocess::scrubbed_command("tmux")
            .arg("-V")
            .output()
            .await
            .is_err()
        {
            return Ok(ToolResult::error(
                "tmux is not installed or not found on PATH".to_string(),
            ));
        }

        let action = require_param!(params, "action");

        match action {
            "create" => {
                let session_name = require_param!(params, "session_name");
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
                        "failed to create session '{session_name}': {stderr}"
                    )));
                }
                debug!(
                    "tmux session '{}' created via socket {}",
                    session_name,
                    get_socket_path().display()
                );
                Ok(ToolResult::new(format!("Session '{session_name}' created")))
            }
            "send" => {
                let session_name = require_param!(params, "session_name");
                if !SAFE_SESSION_NAME.is_match(session_name) {
                    return Ok(ToolResult::error(
                        "session name must contain only alphanumeric characters, hyphens, and underscores".to_string(),
                    ));
                }
                let command = require_param!(params, "command");

                let violations = crate::utils::shell_ast::analyze_command(command);
                if let Some(v) = violations.first() {
                    return Ok(ToolResult::error(format!(
                        "command blocked by structural analysis ({:?}): {}",
                        v.kind, v.description
                    )));
                }

                for pattern in &self.deny_patterns {
                    if pattern.is_match(command) {
                        return Ok(ToolResult::error(format!(
                            "command blocked by security policy: {command}"
                        )));
                    }
                }

                self.ensure_session(session_name).await?;

                let target = exact_target(session_name);
                let (code, _, stderr) = self
                    .run_tmux(&["send-keys", "-t", &target, command, "Enter"])
                    .await?;
                if code != 0 {
                    return Ok(ToolResult::error(format!(
                        "failed to send command to '{session_name}': {stderr}"
                    )));
                }
                Ok(ToolResult::new(format!(
                    "Command sent to session '{session_name}'"
                )))
            }
            "read" => {
                let session_name = require_param!(params, "session_name");
                if !SAFE_SESSION_NAME.is_match(session_name) {
                    return Ok(ToolResult::error(
                        "session name must contain only alphanumeric characters, hyphens, and underscores".to_string(),
                    ));
                }
                let lines = params["lines"].as_u64().unwrap_or(50).min(10000) as i32;

                self.ensure_session(session_name).await?;

                let target = exact_target(session_name);
                let (code, stdout, stderr) = self
                    .run_tmux(&[
                        "capture-pane",
                        "-t",
                        &target,
                        "-p",
                        "-S",
                        &format!("-{lines}"),
                    ])
                    .await?;
                if code != 0 {
                    return Ok(ToolResult::error(format!(
                        "failed to read session '{session_name}': {stderr}"
                    )));
                }
                let output = stdout.trim();
                let output = if output.len() > MAX_OUTPUT_BYTES {
                    let truncated = &output[..output.floor_char_boundary(MAX_OUTPUT_BYTES)];
                    format!("{truncated}\n[output truncated at 1MB]")
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
                        "failed to list sessions: {stderr}"
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
                let session_name = require_param!(params, "session_name");
                if !SAFE_SESSION_NAME.is_match(session_name) {
                    return Ok(ToolResult::error(
                        "session name must contain only alphanumeric characters, hyphens, and underscores".to_string(),
                    ));
                }

                let target = exact_target(session_name);
                let (code, _, stderr) = self.run_tmux(&["kill-session", "-t", &target]).await?;
                if code != 0 {
                    return Ok(ToolResult::error(format!(
                        "failed to kill session '{session_name}': {stderr}"
                    )));
                }
                debug!("tmux session '{}' killed", session_name);
                Ok(ToolResult::new(format!("Session '{session_name}' killed")))
            }
            _ => Ok(ToolResult::error(format!("unknown action '{action}'"))),
        }
    }
}

#[cfg(test)]
mod tests;
