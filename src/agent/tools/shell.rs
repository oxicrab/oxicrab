use crate::agent::tools::base::ExecutionContext;
use crate::agent::tools::{Tool, ToolResult, ToolVersion};
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
}

impl ExecTool {
    pub fn new(
        timeout: u64,
        working_dir: Option<PathBuf>,
        restrict_to_workspace: bool,
        allowed_commands: Vec<String>,
    ) -> Result<Self> {
        let deny_patterns = compile_security_patterns()
            .context("Failed to compile security patterns for exec tool")?;

        Ok(Self {
            timeout,
            working_dir,
            deny_patterns,
            allowed_commands,
            restrict_to_workspace,
        })
    }

    /// Known prefix commands that wrap another command.
    const PREFIX_COMMANDS: &'static [&'static str] = &[
        "sudo", "env", "command", "nohup", "nice", "time", "doas", "xargs",
    ];

    /// Extract the base command name from a shell command string.
    /// Handles leading env vars (FOO=bar cmd), sudo/command prefixes,
    /// and returns the first actual executable token.
    fn extract_command_name(token: &str) -> &str {
        let token = token.trim();
        let parts: Vec<&str> = token.split_whitespace().collect();
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
            return name;
        }
        token
    }

    /// Extract all command names from a shell pipeline/chain.
    /// Splits on |, &&, ||, ;, and newlines to find each command.
    fn extract_all_commands(command: &str) -> Vec<&str> {
        // Split on shell operators: |, &&, ||, ;, \n
        // We need to handle these carefully to extract command names
        let mut commands = Vec::new();
        let mut remaining = command;

        while !remaining.is_empty() {
            // Find the next shell operator
            let next_split = remaining
                .find("&&")
                .map(|i| (i, 2))
                .into_iter()
                .chain(remaining.find("||").map(|i| (i, 2)))
                .chain(remaining.find('|').map(|i| {
                    // Make sure this isn't part of || (already handled)
                    if remaining.get(i + 1..i + 2) == Some("|") {
                        (usize::MAX, 1) // Skip, will be handled by ||
                    } else {
                        (i, 1)
                    }
                }))
                .chain(remaining.find(';').map(|i| (i, 1)))
                .chain(remaining.find('\n').map(|i| (i, 1)))
                .filter(|(pos, _)| *pos != usize::MAX)
                .min_by_key(|(pos, _)| *pos);

            if let Some((pos, len)) = next_split {
                let segment = &remaining[..pos];
                if !segment.trim().is_empty() {
                    commands.push(Self::extract_command_name(segment));
                }
                remaining = &remaining[pos + len..];
            } else {
                if !remaining.trim().is_empty() {
                    commands.push(Self::extract_command_name(remaining));
                }
                break;
            }
        }

        commands
    }

    fn guard_command(&self, command: &str, cwd: &Path) -> Option<String> {
        // Normalize shell line continuations before security checks so that
        // "rm \\\n-rf /" is treated as "rm -rf /" by the patterns below.
        let command = &command.replace("\\\n", " ");

        // Allowlist check: verify all commands in the pipeline are allowed
        if !self.allowed_commands.is_empty() {
            let cmd_names = Self::extract_all_commands(command);
            for name in &cmd_names {
                if !self.allowed_commands.iter().any(|a| a == name) {
                    return Some(format!(
                        "Error: Command '{}' is not in the allowed commands list. \
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
                return Some(format!(
                    "Error: Command blocked by security policy: {}",
                    command
                ));
            }
        }

        if self.restrict_to_workspace
            && let Some(workspace) = &self.working_dir
        {
            if !cwd.starts_with(workspace) {
                return Some(format!(
                    "Error: Working directory '{}' is outside workspace",
                    cwd.display()
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
        for token in command.split_whitespace() {
            // Strip shell quoting characters BEFORE checking for absolute path
            let cleaned = token.trim_matches(|c| c == '\'' || c == '"');
            if !cleaned.starts_with('/') {
                continue;
            }
            if cleaned.is_empty() || cleaned == "/" {
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
                    "Error: path '{}' is outside the workspace",
                    cleaned
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

        match tokio::time::timeout(Duration::from_secs(self.timeout), cmd.output()).await {
            Ok(Ok(output)) => {
                let combined_len = output.stdout.len() + output.stderr.len();
                let truncated = combined_len > MAX_OUTPUT_BYTES;

                // Truncate raw bytes before UTF-8 conversion to bound memory
                let stdout_bytes = if output.stdout.len() > MAX_OUTPUT_BYTES {
                    &output.stdout[..MAX_OUTPUT_BYTES]
                } else {
                    &output.stdout
                };
                let remaining = MAX_OUTPUT_BYTES.saturating_sub(stdout_bytes.len());
                let stderr_bytes = if output.stderr.len() > remaining {
                    &output.stderr[..remaining]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed() -> Vec<String> {
        [
            "ls", "cat", "grep", "git", "echo", "curl", "python3", "cargo",
        ]
        .iter()
        .map(ToString::to_string)
        .collect()
    }

    fn tool(cmds: Vec<String>) -> ExecTool {
        ExecTool::new(60, Some(PathBuf::from("/tmp")), false, cmds).unwrap()
    }

    #[test]
    fn test_extract_simple_command() {
        assert_eq!(ExecTool::extract_command_name("ls -la"), "ls");
    }

    #[test]
    fn test_extract_full_path() {
        assert_eq!(ExecTool::extract_command_name("/usr/bin/ls -la"), "ls");
    }

    #[test]
    fn test_extract_with_env_vars() {
        assert_eq!(
            ExecTool::extract_command_name("FOO=bar BAZ=1 cargo test"),
            "cargo"
        );
    }

    #[test]
    fn test_extract_sudo_prefix() {
        assert_eq!(ExecTool::extract_command_name("sudo rm -rf /"), "rm");
    }

    #[test]
    fn test_extract_env_prefix() {
        assert_eq!(
            ExecTool::extract_command_name("env -i PATH=/usr/bin ls"),
            "ls"
        );
    }

    #[test]
    fn test_extract_nohup_prefix() {
        assert_eq!(
            ExecTool::extract_command_name("nohup python3 app.py"),
            "python3"
        );
    }

    #[test]
    fn test_extract_sudo_with_simple_flags() {
        // sudo -n doesn't take an argument, so cat is correctly found
        assert_eq!(
            ExecTool::extract_command_name("sudo -n cat /etc/shadow"),
            "cat"
        );
    }

    #[test]
    fn test_extract_chained_prefixes() {
        assert_eq!(
            ExecTool::extract_command_name("sudo env FOO=bar python3 script.py"),
            "python3"
        );
    }

    #[test]
    fn test_extract_all_pipe() {
        let cmds = ExecTool::extract_all_commands("cat file.txt | grep foo | sort");
        assert_eq!(cmds, vec!["cat", "grep", "sort"]);
    }

    #[test]
    fn test_extract_all_and_chain() {
        let cmds = ExecTool::extract_all_commands("mkdir -p dir && cd dir && ls");
        assert_eq!(cmds, vec!["mkdir", "cd", "ls"]);
    }

    #[test]
    fn test_extract_all_semicolons() {
        let cmds = ExecTool::extract_all_commands("echo hello; echo world");
        assert_eq!(cmds, vec!["echo", "echo"]);
    }

    #[test]
    fn test_extract_all_or_chain() {
        let cmds = ExecTool::extract_all_commands("ls /missing || echo fallback");
        assert_eq!(cmds, vec!["ls", "echo"]);
    }

    #[test]
    fn test_allowed_simple() {
        let t = tool(allowed());
        assert!(t.guard_command("ls -la", Path::new("/tmp")).is_none());
    }

    #[test]
    fn test_allowed_pipe() {
        let t = tool(allowed());
        assert!(
            t.guard_command("cat file | grep foo", Path::new("/tmp"))
                .is_none()
        );
    }

    #[test]
    fn test_blocked_not_in_list() {
        let t = tool(allowed());
        let result = t.guard_command("rm -rf /", Path::new("/tmp"));
        assert!(result.is_some());
        assert!(result.unwrap().contains("rm"));
    }

    #[test]
    fn test_blocked_pipe_with_disallowed() {
        let t = tool(allowed());
        let result = t.guard_command("cat file | perl -e 'system(\"bad\")'", Path::new("/tmp"));
        assert!(result.is_some());
        assert!(result.unwrap().contains("perl"));
    }

    #[test]
    fn test_empty_allowlist_permits_all() {
        let t = tool(vec![]);
        assert!(
            t.guard_command("anything_goes", Path::new("/tmp"))
                .is_none()
        );
    }

    #[test]
    fn test_sudo_prefix_blocked_by_allowlist() {
        // "sudo rm" should extract "rm", which is not in the allowlist
        let t = tool(allowed());
        let result = t.guard_command("sudo rm -rf /tmp/data", Path::new("/tmp"));
        assert!(result.is_some());
        assert!(result.unwrap().contains("rm"));
    }

    #[test]
    fn test_blocklist_still_applies() {
        // Even if command is on the allowlist, the blocklist catches dangerous usage
        let mut cmds = allowed();
        cmds.push("rm".to_string());
        let t = tool(cmds);
        let result = t.guard_command("rm -rf /", Path::new("/tmp"));
        assert!(result.is_some());
        assert!(result.unwrap().contains("security policy"));
    }

    #[test]
    fn test_full_path_resolved() {
        let t = tool(allowed());
        // /usr/bin/ls should resolve to "ls" which is allowed
        assert!(
            t.guard_command("/usr/bin/ls -la", Path::new("/tmp"))
                .is_none()
        );
    }

    #[test]
    fn test_relative_path_not_rejected() {
        let t = tool(allowed());
        // Relative paths like .venv/bin/python should work if python3 is allowed
        // Here "python3" is in the allowlist but "python" is not
        let result = t.guard_command(".venv/bin/python3 scripts/run.py", Path::new("/tmp"));
        // extract_command_name strips the path prefix, leaving "python3"
        assert!(result.is_none());
    }

    #[test]
    fn test_blocklist_blocks_command_substitution() {
        // Even with an empty allowlist, command substitution is blocked
        let t = tool(vec![]);
        let result = t.guard_command("$(echo rm) -rf /", Path::new("/tmp"));
        assert!(result.is_some());
        assert!(result.unwrap().contains("security policy"));
    }

    #[test]
    fn test_blocklist_blocks_backtick_substitution() {
        let t = tool(vec![]);
        let result = t.guard_command("echo `cat /etc/passwd`", Path::new("/tmp"));
        assert!(result.is_some());
        assert!(result.unwrap().contains("security policy"));
    }

    #[test]
    fn test_blocklist_blocks_variable_expansion() {
        let t = tool(vec![]);
        let result = t.guard_command("echo ${HOME}", Path::new("/tmp"));
        assert!(result.is_some());
        assert!(result.unwrap().contains("security policy"));
    }

    #[test]
    fn test_blocklist_blocks_rm_long_options() {
        let t = tool(vec![]);
        let result = t.guard_command("rm --recursive --force /tmp", Path::new("/tmp"));
        assert!(result.is_some());
        assert!(result.unwrap().contains("security policy"));
    }

    // --- check_paths_in_workspace tests ---

    #[test]
    fn test_paths_inside_workspace_allowed() {
        let workspace = Path::new("/tmp");
        let result = ExecTool::check_paths_in_workspace("cat /tmp/file.txt", workspace);
        assert!(result.is_none());
    }

    #[test]
    fn test_paths_outside_workspace_blocked() {
        let workspace = Path::new("/tmp/workspace");
        let result = ExecTool::check_paths_in_workspace("cat /etc/passwd", workspace);
        assert!(result.is_some());
        assert!(result.unwrap().contains("outside the workspace"));
    }

    #[test]
    fn test_paths_relative_paths_ignored() {
        let workspace = Path::new("/tmp/workspace");
        let result = ExecTool::check_paths_in_workspace("cat relative/path.txt", workspace);
        assert!(result.is_none());
    }

    #[test]
    fn test_paths_root_slash_ignored() {
        let workspace = Path::new("/tmp/workspace");
        // Single "/" should be skipped
        let result = ExecTool::check_paths_in_workspace("ls /", workspace);
        assert!(result.is_none());
    }

    #[test]
    fn test_paths_quoted_paths_stripped() {
        let workspace = Path::new("/tmp");
        let result = ExecTool::check_paths_in_workspace("cat '/tmp/file.txt'", workspace);
        assert!(result.is_none());
    }

    #[test]
    fn test_paths_workspace_enforced_via_guard() {
        let t = ExecTool::new(
            60,
            Some(PathBuf::from("/tmp/workspace")),
            true,
            vec!["cat".to_string()],
        )
        .unwrap();
        let result = t.guard_command("cat /etc/shadow", Path::new("/tmp/workspace"));
        assert!(result.is_some());
        assert!(result.unwrap().contains("outside the workspace"));
    }

    #[test]
    fn test_paths_workspace_disabled_allows_all() {
        let t = ExecTool::new(
            60,
            Some(PathBuf::from("/tmp/workspace")),
            false,
            vec!["cat".to_string()],
        )
        .unwrap();
        // With restrict_to_workspace=false, paths outside workspace are allowed
        let result = t.guard_command("cat /etc/hostname", Path::new("/tmp/workspace"));
        assert!(result.is_none());
    }

    // --- line continuation normalization ---

    #[test]
    fn test_line_continuation_blocked() {
        // "rm \<newline>-rf /" should be caught after normalization
        let t = tool(vec![]);
        let result = t.guard_command("rm \\\n-rf /", Path::new("/tmp"));
        assert!(
            result.is_some(),
            "line continuation should be normalized before security check"
        );
        assert!(result.unwrap().contains("security policy"));
    }

    #[test]
    fn test_line_continuation_allowlist() {
        // "r\<newline>m" joined = "r m" which won't match "rm" as a command,
        // but "rm \\\n-rf" joined = "rm  -rf" should be caught
        let t = tool(allowed());
        let result = t.guard_command("rm \\\n-rf /tmp/data", Path::new("/tmp"));
        assert!(result.is_some());
    }

    // --- output truncation constants ---

    #[test]
    fn test_max_output_bytes_is_1mb() {
        assert_eq!(MAX_OUTPUT_BYTES, 1024 * 1024);
    }
}
