use super::cli_types::{Cli, Commands};
use super::create_workspace_templates;
use clap::Parser;

// --- CLI parsing tests ---

#[test]
fn test_cli_parse_onboard() {
    let cli = Cli::try_parse_from(["oxicrab", "onboard"]).unwrap();
    assert!(matches!(cli.command, Commands::Onboard));
}

#[test]
fn test_cli_parse_gateway() {
    let cli = Cli::try_parse_from(["oxicrab", "gateway"]).unwrap();
    assert!(matches!(
        cli.command,
        Commands::Gateway {
            model: None,
            echo: false
        }
    ));
}

#[test]
fn test_cli_parse_gateway_with_model() {
    let cli = Cli::try_parse_from(["oxicrab", "gateway", "--model", "gpt-4"]).unwrap();
    match cli.command {
        Commands::Gateway { model, echo } => {
            assert_eq!(model.as_deref(), Some("gpt-4"));
            assert!(!echo);
        }
        _ => panic!("expected Gateway"),
    }
}

#[test]
fn test_cli_parse_gateway_echo() {
    let cli = Cli::try_parse_from(["oxicrab", "gateway", "--echo"]).unwrap();
    match cli.command {
        Commands::Gateway { echo, .. } => assert!(echo),
        _ => panic!("expected Gateway"),
    }
}

#[test]
fn test_cli_parse_agent_with_message() {
    let cli = Cli::try_parse_from(["oxicrab", "agent", "-m", "hello"]).unwrap();
    match cli.command {
        Commands::Agent { message, session } => {
            assert_eq!(message.as_deref(), Some("hello"));
            assert_eq!(session, "cli:default");
        }
        _ => panic!("expected Agent"),
    }
}

#[test]
fn test_cli_parse_agent_with_session() {
    let cli = Cli::try_parse_from(["oxicrab", "agent", "--session", "test-session"]).unwrap();
    match cli.command {
        Commands::Agent { session, .. } => {
            assert_eq!(session, "test-session");
        }
        _ => panic!("expected Agent"),
    }
}

#[test]
fn test_cli_parse_agent_default_session() {
    let cli = Cli::try_parse_from(["oxicrab", "agent"]).unwrap();
    match cli.command {
        Commands::Agent { message, session } => {
            assert!(message.is_none());
            assert_eq!(session, "cli:default");
        }
        _ => panic!("expected Agent"),
    }
}

#[test]
fn test_cli_parse_doctor() {
    let cli = Cli::try_parse_from(["oxicrab", "doctor"]).unwrap();
    assert!(matches!(cli.command, Commands::Doctor));
}

#[test]
fn test_cli_parse_status() {
    let cli = Cli::try_parse_from(["oxicrab", "status"]).unwrap();
    assert!(matches!(cli.command, Commands::Status));
}

#[test]
fn test_cli_parse_invalid_command() {
    assert!(Cli::try_parse_from(["oxicrab", "nonexistent"]).is_err());
}

#[test]
fn test_cli_parse_stats_tokens() {
    let cli = Cli::try_parse_from(["oxicrab", "stats", "tokens"]).unwrap();
    match cli.command {
        Commands::Stats { cmd } => {
            assert!(matches!(
                cmd,
                super::cli_types::StatsCommands::Tokens { days: 7 }
            ));
        }
        _ => panic!("expected Stats"),
    }
}

#[test]
fn test_cli_parse_stats_tokens_custom_days() {
    let cli = Cli::try_parse_from(["oxicrab", "stats", "tokens", "--days", "30"]).unwrap();
    match cli.command {
        Commands::Stats { cmd } => {
            assert!(matches!(
                cmd,
                super::cli_types::StatsCommands::Tokens { days: 30 }
            ));
        }
        _ => panic!("expected Stats"),
    }
}

#[test]
fn test_cli_parse_credentials_list() {
    let cli = Cli::try_parse_from(["oxicrab", "credentials", "list"]).unwrap();
    match cli.command {
        Commands::Credentials { cmd } => {
            assert!(matches!(cmd, super::cli_types::CredentialCommands::List));
        }
        _ => panic!("expected Credentials"),
    }
}

#[test]
fn test_cli_parse_credentials_set() {
    let cli = Cli::try_parse_from([
        "oxicrab",
        "credentials",
        "set",
        "anthropic-api-key",
        "sk-test",
    ])
    .unwrap();
    match cli.command {
        Commands::Credentials { cmd } => match cmd {
            super::cli_types::CredentialCommands::Set { name, value } => {
                assert_eq!(name, "anthropic-api-key");
                assert_eq!(value.as_deref(), Some("sk-test"));
            }
            _ => panic!("expected Set"),
        },
        _ => panic!("expected Credentials"),
    }
}

#[test]
fn test_cli_parse_pairing_list() {
    let cli = Cli::try_parse_from(["oxicrab", "pairing", "list"]).unwrap();
    match cli.command {
        Commands::Pairing { cmd } => {
            assert!(matches!(cmd, super::cli_types::PairingCommands::List));
        }
        _ => panic!("expected Pairing"),
    }
}

#[test]
fn test_cli_parse_pairing_approve() {
    let cli = Cli::try_parse_from(["oxicrab", "pairing", "approve", "ABC12345"]).unwrap();
    match cli.command {
        Commands::Pairing { cmd } => match cmd {
            super::cli_types::PairingCommands::Approve { code } => {
                assert_eq!(code, "ABC12345");
            }
            _ => panic!("expected Approve"),
        },
        _ => panic!("expected Pairing"),
    }
}

#[test]
fn test_cli_parse_cron_list() {
    let cli = Cli::try_parse_from(["oxicrab", "cron", "list"]).unwrap();
    match cli.command {
        Commands::Cron { cmd } => {
            assert!(matches!(
                cmd,
                super::cli_types::CronCommands::List { all: false }
            ));
        }
        _ => panic!("expected Cron"),
    }
}

#[test]
fn test_cli_parse_cron_remove() {
    let cli = Cli::try_parse_from(["oxicrab", "cron", "remove", "--id", "job-123"]).unwrap();
    match cli.command {
        Commands::Cron { cmd } => match cmd {
            super::cli_types::CronCommands::Remove { id } => {
                assert_eq!(id, "job-123");
            }
            _ => panic!("expected Remove"),
        },
        _ => panic!("expected Cron"),
    }
}

#[test]
fn test_cli_parse_channels_status() {
    let cli = Cli::try_parse_from(["oxicrab", "channels", "status"]).unwrap();
    match cli.command {
        Commands::Channels { cmd } => {
            assert!(matches!(cmd, super::cli_types::ChannelCommands::Status));
        }
        _ => panic!("expected Channels"),
    }
}

#[test]
fn test_cli_parse_no_args_fails() {
    // Running with no subcommand should fail
    assert!(Cli::try_parse_from(["oxicrab"]).is_err());
}

// --- Workspace template tests ---

#[test]
fn test_create_workspace_templates_in_nonexistent_dir() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("nested").join("workspace");
    // Directory doesn't exist yet
    assert!(!workspace.exists());
    // Should fail since parent directories don't exist
    assert!(create_workspace_templates(&workspace).is_err());
}

#[test]
fn test_create_workspace_templates() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    create_workspace_templates(&workspace).unwrap();

    // Core template files should exist
    assert!(workspace.join("USER.md").exists());
    assert!(workspace.join("AGENTS.md").exists());
    assert!(workspace.join("TOOLS.md").exists());
    // Memory directory should exist (DB lives there)
    assert!(workspace.join("memory").is_dir());
}

#[test]
fn test_create_workspace_templates_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    create_workspace_templates(&workspace).unwrap();

    // Write custom content to USER.md
    let user_path = workspace.join("USER.md");
    std::fs::write(&user_path, "custom content").unwrap();

    // Second run should not overwrite
    create_workspace_templates(&workspace).unwrap();

    let content = std::fs::read_to_string(&user_path).unwrap();
    assert_eq!(content, "custom content");
}

#[test]
fn test_create_workspace_templates_content() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    create_workspace_templates(&workspace).unwrap();

    let agents = std::fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
    assert!(agents.contains("oxicrab"));
    assert!(agents.contains("Personality"));

    let tools = std::fs::read_to_string(workspace.join("TOOLS.md")).unwrap();
    assert!(tools.contains("Tool Notes"));
}
