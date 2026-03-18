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
fn test_allowed_simple() {
    let t = tool(allowed());
    assert!(t.guard_command("ls -la", Path::new("/tmp")).is_none());
}

#[test]
fn test_blocked_not_in_list() {
    let t = tool(allowed());
    let result = t.guard_command("rm -rf /", Path::new("/tmp"));
    assert!(result.is_some());
    assert!(result.unwrap().contains("rm"));
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
fn test_blocklist_still_applies() {
    let mut cmds = allowed();
    cmds.push("rm".to_string());
    let t = tool(cmds);
    let result = t.guard_command("rm -rf /", Path::new("/tmp"));
    assert!(result.is_some());
    assert!(result.unwrap().contains("security policy"));
}

#[test]
fn test_paths_inside_workspace_allowed() {
    let workspace = Path::new("/tmp");
    let result = ExecTool::check_paths_in_workspace("cat /tmp/file.txt", workspace, workspace);
    assert!(result.is_none());
}

#[test]
fn test_paths_outside_workspace_blocked() {
    let workspace = Path::new("/tmp/workspace");
    let result = ExecTool::check_paths_in_workspace("cat /etc/passwd", workspace, workspace);
    assert!(result.is_some());
    assert!(result.unwrap().contains("outside the workspace"));
}

#[test]
fn test_max_output_bytes_is_1mb() {
    assert_eq!(MAX_OUTPUT_BYTES, 1024 * 1024);
}

#[test]
fn test_truncate_at_utf8_boundary_ascii() {
    let data = b"hello world";
    assert_eq!(truncate_at_utf8_boundary(data, 5), b"hello");
}

#[test]
fn test_truncate_at_utf8_boundary_multibyte() {
    let data = "héllo".as_bytes();
    assert_eq!(data.len(), 6);
    let truncated = truncate_at_utf8_boundary(data, 2);
    assert_eq!(truncated, b"h");
    let truncated = truncate_at_utf8_boundary(data, 3);
    assert_eq!(truncated, "hé".as_bytes());
}

#[test]
fn test_extract_all_commands_quoted_pipe_single() {
    let cmds = ExecTool::extract_all_commands("jq '.[] | .name' file.json");
    assert_eq!(cmds, vec!["jq"]);
}

#[test]
fn test_jq_filter_allowed() {
    let mut cmds = allowed();
    cmds.push("jq".to_string());
    let t = tool(cmds);
    assert!(
        t.guard_command("jq '.[] | .name' /tmp/data.json", Path::new("/tmp"))
            .is_none(),
        "jq with quoted pipe filter should be allowed"
    );
}

#[test]
fn test_line_continuation_blocked() {
    let t = tool(vec![]);
    let result = t.guard_command("rm \\\n-rf /", Path::new("/tmp"));
    assert!(
        result.is_some(),
        "line continuation should be normalized before security check"
    );
    assert!(result.unwrap().contains("security policy"));
}

#[test]
fn test_exec_capabilities() {
    let tool = tool(vec![]);
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(!caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::Full);
    assert!(caps.actions.is_empty());
}
