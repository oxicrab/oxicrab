use nanobot::config::Config;

fn default_config() -> Config {
    serde_json::from_str("{}").unwrap()
}

#[test]
fn test_valid_default_passes() {
    let config = default_config();
    assert!(config.validate().is_ok());
}

#[test]
fn test_zero_max_tokens_rejected() {
    let mut config = default_config();
    config.agents.defaults.max_tokens = 0;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("maxTokens"));
}

#[test]
fn test_huge_max_tokens_rejected() {
    let mut config = default_config();
    config.agents.defaults.max_tokens = 2_000_000;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("maxTokens"));
}

#[test]
fn test_temperature_below_zero_rejected() {
    let mut config = default_config();
    config.agents.defaults.temperature = -0.1;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("temperature"));
}

#[test]
fn test_temperature_above_two_rejected() {
    let mut config = default_config();
    config.agents.defaults.temperature = 2.1;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("temperature"));
}

#[test]
fn test_zero_max_iterations_rejected() {
    let mut config = default_config();
    config.agents.defaults.max_tool_iterations = 0;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("maxToolIterations"));
}

#[test]
fn test_compaction_zero_threshold() {
    let mut config = default_config();
    config.agents.defaults.compaction.enabled = true;
    config.agents.defaults.compaction.threshold_tokens = 0;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("thresholdTokens"));
}

#[test]
fn test_compaction_zero_keep_recent() {
    let mut config = default_config();
    config.agents.defaults.compaction.enabled = true;
    config.agents.defaults.compaction.keep_recent = 0;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("keepRecent"));
}

#[test]
fn test_compaction_disabled_ignores_bad_values() {
    let mut config = default_config();
    config.agents.defaults.compaction.enabled = false;
    config.agents.defaults.compaction.threshold_tokens = 0;
    config.agents.defaults.compaction.keep_recent = 0;
    // Should pass — compaction rules only apply when enabled
    assert!(config.validate().is_ok());
}

#[test]
fn test_daemon_zero_interval() {
    let mut config = default_config();
    config.agents.defaults.daemon.enabled = true;
    config.agents.defaults.daemon.interval = 0;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("interval"));
}

#[test]
fn test_exec_timeout_zero() {
    let mut config = default_config();
    config.tools.exec.timeout = 0;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("timeout"));
}

#[test]
fn test_gateway_port_zero_rejected() {
    let mut config = default_config();
    config.gateway.port = 0;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("port"));
}

#[test]
fn test_huge_max_iterations_rejected() {
    let mut config = default_config();
    config.agents.defaults.max_tool_iterations = 1001;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("maxToolIterations"));
}
