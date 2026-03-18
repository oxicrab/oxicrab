use super::*;

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

#[test]
fn test_tmux_capabilities() {
    let tool = TmuxTool::new();
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(!caps.network_outbound);
    assert_eq!(caps.actions.len(), 5);
}

#[test]
fn test_tmux_actions_match_schema() {
    let tool = TmuxTool::new();
    let caps = tool.capabilities();
    let params = tool.parameters();
    let schema_actions: Vec<String> = params["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let cap_actions: Vec<String> = caps.actions.iter().map(|a| a.name.to_string()).collect();
    for action in &schema_actions {
        assert!(
            cap_actions.contains(action),
            "action '{action}' in schema but not in capabilities()"
        );
    }
    for action in &cap_actions {
        assert!(
            schema_actions.contains(action),
            "action '{action}' in capabilities() but not in schema"
        );
    }
}
