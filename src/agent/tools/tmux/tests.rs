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

#[test]
fn test_tmux_capabilities() {
    use crate::agent::tools::base::SubagentAccess;
    let tool = TmuxTool::new();
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(!caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::Denied);
    assert!(caps.actions.is_empty());
}
