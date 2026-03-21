use crate::utils::path_sanitize::{sanitize_error_message, sanitize_path};
use crate::utils::regex_utils::compile_security_patterns;
use anyhow::{Context, Result};
use async_trait::async_trait;
use oxicrab_core::actions;
use oxicrab_core::config::schema::SandboxConfig;
use oxicrab_core::require_param;
use oxicrab_core::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory};
use oxicrab_core::tools::base::{Tool, ToolResult};
use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::warn;

/// Maximum combined stdout+stderr size before truncation.
const MAX_OUTPUT_BYTES: usize = 1024 * 1024; // 1 MB

pub struct ExecTool {
    timeout: u64,
    working_dir: Option<PathBuf>,
    deny_patterns: Vec<Regex>,
    allowed_commands: Vec<String>,
    restrict_to_workspace: bool,
    sandbox_config: SandboxConfig,
}

impl ExecTool {
    pub fn new(
        timeout: u64,
        working_dir: Option<PathBuf>,
        restrict_to_workspace: bool,
        allowed_commands: Vec<String>,
        sandbox_config: SandboxConfig,
    ) -> Result<Self> {
        let deny_patterns = compile_security_patterns()
            .context("Failed to compile security patterns for exec tool")?;

        let working_dir = working_dir.map(|p| p.canonicalize().unwrap_or(p));

        Ok(Self {
            timeout,
            working_dir,
            deny_patterns,
            allowed_commands,
            restrict_to_workspace,
            sandbox_config,
        })
    }

    const PREFIX_COMMANDS: &'static [&'static str] = &[
        "sudo", "env", "command", "nohup", "nice", "time", "doas", "xargs",
    ];

    pub(crate) fn extract_command_name(token: &str) -> String {
        let token = token.trim();
        let parts = shlex::split(token)
            .unwrap_or_else(|| token.split_whitespace().map(String::from).collect());
        let mut found_prefix = false;
        for part in &parts {
            if part.contains('=') && !part.starts_with('-') {
                continue;
            }
            if found_prefix && part.starts_with('-') {
                continue;
            }
            let name = part.rsplit('/').next().unwrap_or(part);
            if Self::PREFIX_COMMANDS.contains(&name) {
                found_prefix = true;
                continue;
            }
            return name.to_string();
        }
        token.to_string()
    }

    pub(crate) fn extract_all_commands(command: &str) -> Vec<String> {
        let mut commands = Vec::new();
        let bytes = command.as_bytes();
        let len = bytes.len();
        let mut seg_start = 0;
        let mut i = 0;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        while i < len {
            if escaped {
                escaped = false;
                i += 1;
                continue;
            }

            let ch = bytes[i];

            if ch == b'\\' && !in_single {
                escaped = true;
                i += 1;
                continue;
            }

            if ch == b'\'' && !in_double {
                in_single = !in_single;
                i += 1;
                continue;
            }
            if ch == b'"' && !in_single {
                in_double = !in_double;
                i += 1;
                continue;
            }

            if !in_single && !in_double {
                let rest = &command[i..];
                let op_len = if rest.starts_with("&&") || rest.starts_with("||") {
                    Some(2)
                } else if matches!(ch, b'|' | b';' | b'\n') {
                    Some(1)
                } else if ch == b'&' {
                    // Single & (background) — treat as segment separator
                    Some(1)
                } else {
                    None
                };

                if let Some(len) = op_len {
                    let segment = &command[seg_start..i];
                    if !segment.trim().is_empty() {
                        commands.push(Self::extract_command_name(segment));
                    }
                    i += len;
                    seg_start = i;
                    continue;
                }
            }

            i += 1;
        }

        let tail = &command[seg_start..];
        if !tail.trim().is_empty() {
            commands.push(Self::extract_command_name(tail));
        }

        commands
    }

    pub(crate) fn guard_command(&self, command: &str, cwd: &Path) -> Option<String> {
        let command = &command.replace("\\\n", " ");

        let violations = crate::utils::shell_ast::analyze_command(command);
        if let Some(v) = violations.first() {
            return Some(format!(
                "command blocked by structural analysis ({:?}): {}",
                v.kind, v.description
            ));
        }

        if !self.allowed_commands.is_empty() {
            let cmd_names = Self::extract_all_commands(command);
            for name in &cmd_names {
                if !self.allowed_commands.iter().any(|a| a == name) {
                    return Some(format!(
                        "command '{}' is not in the allowed commands list. \
                         Allowed: {}",
                        name,
                        self.allowed_commands.join(", ")
                    ));
                }
            }
        }

        for pattern in &self.deny_patterns {
            if pattern.is_match(command) {
                return Some(format!("command blocked by security policy: {command}"));
            }
        }

        if self.restrict_to_workspace
            && let Some(workspace) = &self.working_dir
        {
            if !cwd.starts_with(workspace) {
                return Some(format!(
                    "working directory '{}' is outside workspace",
                    sanitize_path(cwd, Some(workspace))
                ));
            }

            if let Some(err) = Self::check_paths_in_workspace(command, workspace, cwd) {
                return Some(err);
            }
        }

        None
    }

    pub(crate) fn check_paths_in_workspace(
        command: &str,
        workspace: &Path,
        working_dir: &Path,
    ) -> Option<String> {
        let tokens = shlex::split(command).unwrap_or_default();
        for cleaned in &tokens {
            // Detect glob patterns in absolute paths (shell will expand at runtime,
            // bypassing our canonicalize-based workspace check)
            let has_glob = cleaned.contains('*')
                || cleaned.contains('?')
                || (cleaned.contains('[') && cleaned.contains(']'));
            if has_glob
                && (cleaned.starts_with('/') || cleaned.starts_with('~') || cleaned.contains(".."))
            {
                return Some(format!(
                    "glob pattern in path '{cleaned}' cannot be verified against workspace"
                ));
            }

            if cleaned.starts_with('/') && cleaned != "/" {
                let path = Path::new(cleaned);
                let resolved = path
                    .canonicalize()
                    .unwrap_or_else(|_| lexical_normalize(path));
                if !resolved.starts_with(workspace) {
                    return Some(format!(
                        "path '{}' is outside the workspace",
                        sanitize_path(Path::new(cleaned), Some(workspace),)
                    ));
                }
            } else if cleaned.starts_with('~') {
                let expanded = if cleaned == "~" {
                    dirs::home_dir().unwrap_or_default()
                } else if let Some(rest) = cleaned.strip_prefix("~/") {
                    dirs::home_dir().unwrap_or_default().join(rest)
                } else {
                    return Some(format!(
                        "path '{cleaned}' uses tilde expansion outside workspace"
                    ));
                };
                let resolved = expanded
                    .canonicalize()
                    .unwrap_or_else(|_| lexical_normalize(&expanded));
                if !resolved.starts_with(workspace) {
                    return Some(format!(
                        "path '{}' resolves outside workspace",
                        sanitize_path(Path::new(cleaned), Some(workspace),)
                    ));
                }
            } else if cleaned.contains("..") {
                let resolved = working_dir.join(cleaned);
                let canonical = resolved
                    .canonicalize()
                    .unwrap_or_else(|_| lexical_normalize(&resolved));
                if !canonical.starts_with(workspace) {
                    return Some(format!("path '{cleaned}' resolves outside workspace"));
                }
            }
        }
        None
    }
}

/// Normalize a path lexically (without touching the filesystem).
pub fn lexical_normalize(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                if matches!(components.last(), Some(std::path::Component::Normal(_))) {
                    components.pop();
                }
            }
            std::path::Component::CurDir => {}
            other => components.push(other),
        }
    }
    components.iter().collect()
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

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(self.timeout)
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            subagent_access: SubagentAccess::Full,
            actions: actions![execute],
            category: ToolCategory::System,
            ..Default::default()
        }
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let command = require_param!(params, "command");

        let working_dir = params["working_dir"]
            .as_str()
            .map(PathBuf::from)
            .or_else(|| self.working_dir.clone());

        let cwd = working_dir.unwrap_or_else(|| {
            std::env::current_dir()
                .context("Failed to get current directory")
                .unwrap_or_else(|e| {
                    warn!("Failed to get current directory: {}, using '.'", e);
                    PathBuf::from(".")
                })
        });

        let cwd = cwd.canonicalize().unwrap_or(cwd);

        if !cwd.is_dir() {
            return Ok(ToolResult::error(format!(
                "working directory does not exist: {}",
                cwd.display()
            )));
        }

        if let Some(err) = self.guard_command(command, &cwd) {
            return Ok(ToolResult::error(err));
        }

        let mut cmd = crate::utils::subprocess::scrubbed_command("sh");
        cmd.arg("-c").arg(command);
        cmd.current_dir(&cwd);
        cmd.kill_on_drop(true);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        #[cfg(unix)]
        cmd.process_group(0);

        if self.sandbox_config.enabled {
            let rules = crate::utils::sandbox::SandboxRules::for_shell(&cwd, &self.sandbox_config);
            if let Err(e) = crate::utils::sandbox::apply_to_command(&mut cmd, &rules) {
                return Ok(ToolResult::error(format!(
                    "sandbox is required but failed to apply: {e}"
                )));
            }
        }

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::error(sanitize_error_message(
                    &format!("error executing command: {e}"),
                    self.working_dir.as_deref(),
                )));
            }
        };

        #[cfg(unix)]
        let child_pid = child.id();

        match tokio::time::timeout(Duration::from_secs(self.timeout), child.wait_with_output())
            .await
        {
            Ok(Ok(output)) => Ok(format_output(&output)),
            Ok(Err(e)) => Ok(ToolResult::error(sanitize_error_message(
                &format!("error executing command: {e}"),
                self.working_dir.as_deref(),
            ))),
            Err(_) => {
                #[cfg(unix)]
                if let Some(pid) = child_pid {
                    // SAFETY: killpg sends SIGKILL to all processes in the group.
                    unsafe {
                        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
                    }
                }
                Ok(ToolResult::error(format!(
                    "command timed out after {} seconds",
                    self.timeout
                )))
            }
        }
    }
}

fn format_output(output: &std::process::Output) -> ToolResult {
    let combined_len = output.stdout.len() + output.stderr.len();
    let truncated = combined_len > MAX_OUTPUT_BYTES;

    let stderr_reserve = MAX_OUTPUT_BYTES / 4;
    let stderr_needed = stderr_reserve.min(output.stderr.len());
    let stdout_max = MAX_OUTPUT_BYTES - stderr_needed;
    let stdout_bytes = if output.stdout.len() > stdout_max {
        truncate_at_utf8_boundary(&output.stdout, stdout_max)
    } else {
        &output.stdout
    };
    let remaining = MAX_OUTPUT_BYTES.saturating_sub(stdout_bytes.len());
    let stderr_limit = remaining.max(stderr_needed);
    let stderr_bytes = if output.stderr.len() > stderr_limit {
        truncate_at_utf8_boundary(&output.stderr, stderr_limit)
    } else {
        &output.stderr
    };

    let stdout = String::from_utf8_lossy(stdout_bytes);
    let stderr = String::from_utf8_lossy(stderr_bytes);

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
    if truncated {
        result.push_str("\n[output truncated at 1MB]");
    }

    if output.status.success() {
        ToolResult::new(if result.is_empty() {
            "(no output)".to_string()
        } else {
            result
        })
    } else {
        ToolResult::error(format!("command failed: {result}"))
    }
}

fn truncate_at_utf8_boundary(data: &[u8], max: usize) -> &[u8] {
    if max >= data.len() {
        return data;
    }
    let mut end = max;
    while end > 0 && (data[end] & 0xC0) == 0x80 {
        end -= 1;
    }
    &data[..end]
}

#[cfg(test)]
mod tests;
