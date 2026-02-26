use super::*;

#[test]
fn test_default_capabilities_are_deny_all() {
    let caps = ToolCapabilities::default();
    assert!(!caps.built_in);
    assert!(!caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::Denied);
    assert!(caps.actions.is_empty());
}

#[test]
fn test_subagent_access_equality() {
    assert_eq!(SubagentAccess::Full, SubagentAccess::Full);
    assert_ne!(SubagentAccess::Full, SubagentAccess::ReadOnly);
    assert_ne!(SubagentAccess::ReadOnly, SubagentAccess::Denied);
}
