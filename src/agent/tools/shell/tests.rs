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
    ExecTool::new(
        60,
        Some(PathBuf::from("/tmp")),
        false,
        cmds,
        SandboxConfig {
            enabled: false,
            ..SandboxConfig::default()
        },
    )
    .unwrap()
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
    // (now caught by AST structural analysis before regex)
    let t = tool(vec![]);
    let result = t.guard_command("$(echo rm) -rf /", Path::new("/tmp"));
    assert!(result.is_some());
    let msg = result.unwrap();
    assert!(
        msg.contains("structural analysis") || msg.contains("security policy"),
        "expected block message, got: {}",
        msg
    );
}

#[test]
fn test_blocklist_blocks_backtick_substitution() {
    // (now caught by AST structural analysis before regex)
    let t = tool(vec![]);
    let result = t.guard_command("echo `cat /etc/passwd`", Path::new("/tmp"));
    assert!(result.is_some());
    let msg = result.unwrap();
    assert!(
        msg.contains("structural analysis") || msg.contains("security policy"),
        "expected block message, got: {}",
        msg
    );
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
        SandboxConfig {
            enabled: false,
            ..SandboxConfig::default()
        },
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
        SandboxConfig {
            enabled: false,
            ..SandboxConfig::default()
        },
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
