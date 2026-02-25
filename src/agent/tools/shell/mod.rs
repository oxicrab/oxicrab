use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use crate::config::SandboxConfig;
use crate::utils::regex::compile_security_patterns;
use anyhow::{Context, Result};
use async_trait::async_trait;
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

        // Canonicalize working_dir to resolve symlinks (e.g. /var → /private/var on macOS).
        // This ensures the workspace comparison in guard_command() uses resolved paths
        // on both sides, since cwd is also canonicalized at execution time.
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

    /// Known prefix commands that wrap another command.
    const PREFIX_COMMANDS: &'static [&'static str] = &[
        "sudo", "env", "command", "nohup", "nice", "time", "doas", "xargs",
    ];

    /// Extract the base command name from a shell command string.
    /// Handles leading env vars (FOO=bar cmd), sudo/command prefixes,
    /// and returns the first actual executable token.
    fn extract_command_name(token: &str) -> String {
        let token = token.trim();
        // Use shlex for proper shell-aware tokenization (handles quoting/escaping).
        // Fall back to whitespace splitting when shlex cannot parse the input
        // (e.g. truly malformed quoting).
        let parts = shlex::split(token)
            .unwrap_or_else(|| token.split_whitespace().map(String::from).collect());
        let mut found_prefix = false;
        for part in &parts {
            // Skip env var assignments (KEY=value)
            if part.contains('=') && !part.starts_with('-') {
                continue;
            }
            // Skip flags (e.g., sudo -u root, env -i, nice -n 10)
            if found_prefix && part.starts_with('-') {
                continue;
            }
            // Get basename in case of full path like /usr/bin/ls
            let name = part.rsplit('/').next().unwrap_or(part);
            // Skip known prefix commands to find the actual command
            if Self::PREFIX_COMMANDS.contains(&name) {
                found_prefix = true;
                continue;
            }
            return name.to_string();
        }
        token.to_string()
    }

    /// Extract all command names from a shell pipeline/chain.
    /// Splits on |, &&, ||, ;, and newlines to find each command,
    /// respecting single and double quoting so that operators inside
    /// quoted strings (e.g. `jq '.[] | .name'`) are not treated as
    /// pipeline separators.
    fn extract_all_commands(command: &str) -> Vec<String> {
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

            // Backslash escapes outside single quotes
            if ch == b'\\' && !in_single {
                escaped = true;
                i += 1;
                continue;
            }

            // Quote tracking
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

            // Only split on operators outside quotes
            if !in_single && !in_double {
                let rest = &command[i..];
                let op_len = if rest.starts_with("&&") || rest.starts_with("||") {
                    Some(2)
                } else if matches!(ch, b'|' | b';' | b'\n') {
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

        // Remaining tail
        let tail = &command[seg_start..];
        if !tail.trim().is_empty() {
            commands.push(Self::extract_command_name(tail));
        }

        commands
    }

    fn guard_command(&self, command: &str, cwd: &Path) -> Option<String> {
        // Normalize shell line continuations before security checks so that
        // "rm \\\n-rf /" is treated as "rm -rf /" by the patterns below.
        let command = &command.replace("\\\n", " ");

        // AST structural analysis: catches patterns that regex can't reliably
        // detect (interpreter inline exec, pipe targets, function definitions,
        // subshells, process substitution). If parsing fails, falls through
        // silently to the regex layer.
        let violations = crate::utils::shell_ast::analyze_command(command);
        if let Some(v) = violations.first() {
            return Some(format!(
                "command blocked by structural analysis ({:?}): {}",
                v.kind, v.description
            ));
        }

        // Allowlist check: verify all commands in the pipeline are allowed.
        // Empty allowlist = unrestricted mode (all commands permitted).
        // Non-empty allowlist = only listed commands are allowed.
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

        // Blocklist check (secondary safety layer)
        for pattern in &self.deny_patterns {
            if pattern.is_match(command) {
                return Some(format!("command blocked by security policy: {}", command));
            }
        }

        if self.restrict_to_workspace
            && let Some(workspace) = &self.working_dir
        {
            if !cwd.starts_with(workspace) {
                return Some(format!(
                    "working directory '{}' is outside workspace",
                    crate::utils::path_sanitize::sanitize_path(cwd, Some(workspace))
                ));
            }

            // Check path-like tokens in the command for workspace escape
            if let Some(err) = Self::check_paths_in_workspace(command, workspace) {
                return Some(err);
            }
        }

        None
    }

    /// Extract absolute path tokens from a command and verify they resolve
    /// inside the workspace. Returns an error message if any path escapes.
    fn check_paths_in_workspace(command: &str, workspace: &Path) -> Option<String> {
        // Use shlex for proper shell-aware tokenization (handles quoting/escaping)
        let tokens = shlex::split(command).unwrap_or_default();
        for cleaned in &tokens {
            if !cleaned.starts_with('/') || cleaned == "/" {
                continue;
            }
            let path = Path::new(cleaned);
            // Use canonicalize if the path exists (resolves symlinks).
            // For non-existent paths, use lexical normalization to prevent
            // symlink-based workspace escapes (canonicalize fails on non-existent
            // paths, returning the raw path which could contain `..` components).
            let resolved = path
                .canonicalize()
                .unwrap_or_else(|_| lexical_normalize(path));
            if !resolved.starts_with(workspace) {
                return Some(format!(
                    "path '{}' is outside the workspace",
                    crate::utils::path_sanitize::sanitize_path(Path::new(cleaned), Some(workspace))
                ));
            }
        }
        None
    }
}

/// Normalize a path lexically (without touching the filesystem).
/// Resolves `.` and `..` components so that `/workspace/../etc/passwd`
/// correctly normalizes to `/etc/passwd` rather than passing through
/// as if it starts with `/workspace`.
pub(crate) fn lexical_normalize(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                // Pop the last normal component (but never pop past root)
                if matches!(components.last(), Some(std::path::Component::Normal(_))) {
                    components.pop();
                }
            }
            std::path::Component::CurDir => {} // skip "."
            other => components.push(other),
        }
    }
    components.iter().collect()
}

#[async_trait]
impl Tool for ExecTool {
    fn name(&self) -> &'static str {
        "exec"
    }

    fn description(&self) -> &'static str {
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

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(self.timeout)
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: false,
            subagent_access: SubagentAccess::Full,
            actions: vec![],
        }
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
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
                    warn!("Failed to get current directory: {}, using '.'", e);
                    PathBuf::from(".")
                })
        });

        // Canonicalize cwd so symlinks and ".." are resolved before workspace check
        let cwd = cwd.canonicalize().unwrap_or(cwd);

        if let Some(err) = self.guard_command(command, &cwd) {
            return Ok(ToolResult::error(err));
        }

        let mut cmd = crate::utils::subprocess::scrubbed_command("sh");
        cmd.arg("-c").arg(command);
        cmd.current_dir(&cwd);
        cmd.kill_on_drop(true);

        if self.sandbox_config.enabled {
            let rules = crate::utils::sandbox::SandboxRules::for_shell(&cwd, &self.sandbox_config);
            if let Err(e) = crate::utils::sandbox::apply_to_command(&mut cmd, &rules) {
                warn!("failed to apply sandbox: {}, continuing without", e);
            }
        }

        match tokio::time::timeout(Duration::from_secs(self.timeout), cmd.output()).await {
            Ok(Ok(output)) => {
                let combined_len = output.stdout.len() + output.stderr.len();
                let truncated = combined_len > MAX_OUTPUT_BYTES;

                // Truncate raw bytes before UTF-8 conversion to bound memory.
                // Reserve at least 25% for stderr so error messages aren't lost.
                let stderr_reserve = MAX_OUTPUT_BYTES / 4;
                let stdout_max = MAX_OUTPUT_BYTES - stderr_reserve.min(output.stderr.len());
                let stdout_bytes = if output.stdout.len() > stdout_max {
                    truncate_at_utf8_boundary(&output.stdout, stdout_max)
                } else {
                    &output.stdout
                };
                let remaining = MAX_OUTPUT_BYTES.saturating_sub(stdout_bytes.len());
                let stderr_bytes = if output.stderr.len() > remaining {
                    truncate_at_utf8_boundary(&output.stderr, remaining)
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
                    Ok(ToolResult::new(if result.is_empty() {
                        "(no output)".to_string()
                    } else {
                        result
                    }))
                } else {
                    Ok(ToolResult::error(format!("command failed: {}", result)))
                }
            }
            Ok(Err(e)) => Ok(ToolResult::error(
                crate::utils::path_sanitize::sanitize_error_message(
                    &format!("error executing command: {}", e),
                    self.working_dir.as_deref(),
                ),
            )),
            Err(_) => Ok(ToolResult::error(format!(
                "command timed out after {} seconds",
                self.timeout
            ))),
        }
    }
}

/// Truncate a byte slice at a UTF-8 character boundary, never splitting
/// a multi-byte character.
fn truncate_at_utf8_boundary(data: &[u8], max: usize) -> &[u8] {
    if max >= data.len() {
        return data;
    }
    // Walk backwards from max to find a valid UTF-8 start byte
    let mut end = max;
    while end > 0 && (data[end] & 0xC0) == 0x80 {
        end -= 1;
    }
    &data[..end]
}

#[cfg(test)]
mod tests;
