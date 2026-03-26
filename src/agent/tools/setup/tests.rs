use super::*;

#[test]
fn test_community_safe_keyword_matching() {
    // Read-only tool names should pass (snake_case, camelCase, PascalCase)
    assert!(is_community_safe("list_users"));
    assert!(is_community_safe("get_document"));
    assert!(is_community_safe("search_records"));
    assert!(is_community_safe("ReadConfig"));
    assert!(is_community_safe("fetchData"));
    assert!(is_community_safe("showStatus"));
    assert!(is_community_safe("count-items"));

    // Mutating tool names should be rejected
    assert!(!is_community_safe("delete_users"));
    assert!(!is_community_safe("create_record"));
    assert!(!is_community_safe("execute_command"));
    assert!(!is_community_safe("send_email"));

    // Substring false positives must be rejected (word-boundary check)
    assert!(!is_community_safe("breadcrumb")); // contains "read" substring
    assert!(!is_community_safe("overwrite")); // contains "view" substring
    assert!(!is_community_safe("altogether")); // contains "get" substring
}

#[test]
fn test_builtin_tools_have_builtin_capability() {
    // Verify that all built-in tool types declare built_in: true
    use oxicrab_tools_system::filesystem::ReadFileTool;
    use oxicrab_tools_system::shell::ExecTool;

    assert!(ReadFileTool::new(None, None).capabilities().built_in);
    assert!(
        ExecTool::new(
            10,
            None,
            false,
            config::AllowedCommands::new(vec![]),
            config::SandboxConfig::default()
        )
        .unwrap()
        .capabilities()
        .built_in
    );
    // WebSearchTool built_in is tested in oxicrab-tools-web crate
}
