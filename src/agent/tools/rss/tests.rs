use super::*;

#[test]
fn test_tool_name() {
    let tool = RssTool::new_for_test();
    assert_eq!(tool.name(), "rss");
}

#[test]
fn test_tool_capabilities() {
    let tool = RssTool::new_for_test();
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(caps.network_outbound);
    assert_eq!(caps.category, ToolCategory::Web);
    assert_eq!(caps.actions.len(), 11);
}

#[test]
fn test_execution_timeout() {
    let tool = RssTool::new_for_test();
    assert_eq!(tool.execution_timeout(), Duration::from_mins(5));
}

#[test]
fn test_parameters_schema_has_all_actions() {
    let tool = RssTool::new_for_test();
    let params = tool.parameters();
    let actions = params["properties"]["action"]["enum"]
        .as_array()
        .expect("action enum should exist");
    let action_names: Vec<&str> = actions.iter().filter_map(|v| v.as_str()).collect();
    for expected in [
        "onboard",
        "set_profile",
        "add_feed",
        "remove_feed",
        "list_feeds",
        "scan",
        "get_articles",
        "accept",
        "reject",
        "get_article_detail",
        "feed_stats",
    ] {
        assert!(
            action_names.contains(&expected),
            "missing action: {expected}"
        );
    }
}
