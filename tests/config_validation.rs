use oxicrab::config::{
    Config, ExfiltrationGuardConfig, McpConfig, PromptGuardAction, PromptGuardConfig,
};

fn default_config() -> Config {
    serde_json::from_str("{}").expect("parse default config")
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

#[test]
fn test_mcp_config_defaults_empty() {
    let config: McpConfig = serde_json::from_str("{}").expect("parse json");
    assert!(config.servers.is_empty());
}

#[test]
fn test_mcp_config_parses_servers() {
    let json = serde_json::json!({
        "servers": {
            "filesystem": {
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
                "enabled": true
            },
            "git": {
                "command": "python",
                "args": ["-m", "mcp_server_git"],
                "env": {"GIT_DIR": "/repo"},
                "enabled": false
            }
        }
    });

    let config: McpConfig = serde_json::from_value(json).expect("parse mcp config");
    assert_eq!(config.servers.len(), 2);

    let fs = &config.servers["filesystem"];
    assert_eq!(fs.command, "npx");
    assert_eq!(
        fs.args,
        vec!["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    );
    assert!(fs.enabled);
    assert!(fs.env.is_empty());

    let git = &config.servers["git"];
    assert_eq!(git.command, "python");
    assert!(!git.enabled);
    assert_eq!(git.env.get("GIT_DIR").unwrap(), "/repo");
}

#[test]
fn test_mcp_config_enabled_defaults_true() {
    let json = serde_json::json!({
        "servers": {
            "test": {
                "command": "echo"
            }
        }
    });

    let config: McpConfig = serde_json::from_value(json).expect("parse mcp config");
    assert!(config.servers["test"].enabled);
    assert!(config.servers["test"].args.is_empty());
    assert!(config.servers["test"].env.is_empty());
}

#[test]
fn test_mcp_config_in_full_config() {
    let json = serde_json::json!({
        "tools": {
            "mcp": {
                "servers": {
                    "test_server": {
                        "command": "/usr/bin/test-mcp",
                        "args": ["--port", "3000"]
                    }
                }
            }
        }
    });

    let config: Config = serde_json::from_value(json).expect("parse config");
    assert_eq!(config.tools.mcp.servers.len(), 1);
    assert_eq!(
        config.tools.mcp.servers["test_server"].command,
        "/usr/bin/test-mcp"
    );
}

// --- Prompt Guard config validation ---

#[test]
fn test_prompt_guard_warn_action_valid() {
    let mut config = default_config();
    config.agents.defaults.prompt_guard.enabled = true;
    config.agents.defaults.prompt_guard.action = PromptGuardAction::Warn;
    assert!(config.validate().is_ok());
}

#[test]
fn test_prompt_guard_block_action_valid() {
    let mut config = default_config();
    config.agents.defaults.prompt_guard.enabled = true;
    config.agents.defaults.prompt_guard.action = PromptGuardAction::Block;
    assert!(config.validate().is_ok());
}

#[test]
fn test_prompt_guard_invalid_action_rejected_by_serde() {
    let json = serde_json::json!({
        "agents": {
            "defaults": {
                "promptGuard": {
                    "enabled": true,
                    "action": "invalid"
                }
            }
        }
    });
    assert!(serde_json::from_value::<Config>(json).is_err());
}

#[test]
fn test_prompt_guard_default_config_valid() {
    let config: PromptGuardConfig = serde_json::from_str("{}").expect("parse json");
    assert!(!config.enabled);
    assert_eq!(config.action, PromptGuardAction::Warn);
}

#[test]
fn test_prompt_guard_parses_from_json() {
    let json = serde_json::json!({
        "agents": {
            "defaults": {
                "promptGuard": {
                    "enabled": true,
                    "action": "block"
                }
            }
        }
    });
    let config: Config = serde_json::from_value(json).expect("parse config");
    assert!(config.agents.defaults.prompt_guard.enabled);
    assert_eq!(
        config.agents.defaults.prompt_guard.action,
        PromptGuardAction::Block
    );
}

// --- Exfiltration Guard config ---

#[test]
fn test_exfiltration_guard_default_config() {
    let config: ExfiltrationGuardConfig = serde_json::from_str("{}").expect("parse json");
    assert!(!config.enabled);
    assert_eq!(config.blocked_tools, vec!["http", "web_fetch", "browser"]);
}

#[test]
fn test_exfiltration_guard_custom_blocked_tools() {
    let json = serde_json::json!({
        "enabled": true,
        "blockedTools": ["http", "exec", "custom_tool"]
    });
    let config: ExfiltrationGuardConfig = serde_json::from_value(json).expect("parse json");
    assert!(config.enabled);
    assert_eq!(config.blocked_tools, vec!["http", "exec", "custom_tool"]);
}

#[test]
fn test_exfiltration_guard_parses_from_full_config() {
    let json = serde_json::json!({
        "tools": {
            "exfiltrationGuard": {
                "enabled": true,
                "blockedTools": ["http", "browser"]
            }
        }
    });
    let config: Config = serde_json::from_value(json).expect("parse config");
    assert!(config.tools.exfiltration_guard.enabled);
    assert_eq!(
        config.tools.exfiltration_guard.blocked_tools,
        vec!["http", "browser"]
    );
}
