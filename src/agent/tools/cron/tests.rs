use super::*;
use crate::agent::tools::Tool;
use crate::config::{
    ChannelsConfig, DiscordConfig, SlackConfig, TelegramConfig, TwilioConfig, WhatsAppConfig,
};
use serde_json::json;

fn make_test_channels_config() -> ChannelsConfig {
    ChannelsConfig {
        slack: SlackConfig {
            enabled: true,
            allow_from: vec!["U08G6HBC89X".to_string()],
            ..Default::default()
        },
        discord: DiscordConfig {
            enabled: true,
            allow_from: vec!["123456789".to_string()],
            ..Default::default()
        },
        telegram: TelegramConfig {
            enabled: true,
            allow_from: vec!["987654321".to_string()],
            ..Default::default()
        },
        whatsapp: WhatsAppConfig {
            enabled: true,
            allow_from: vec!["+15551234567".to_string()],
            ..Default::default()
        },
        twilio: TwilioConfig::default(),
    }
}

#[test]
fn test_resolve_all_channels() {
    let cfg = make_test_channels_config();
    let targets = resolve_all_channel_targets_from_config(Some(&cfg));
    assert_eq!(targets.len(), 4);
    assert!(
        targets
            .iter()
            .any(|t| t.channel == "slack" && t.to == "U08G6HBC89X")
    );
    assert!(
        targets
            .iter()
            .any(|t| t.channel == "discord" && t.to == "123456789")
    );
    assert!(
        targets
            .iter()
            .any(|t| t.channel == "telegram" && t.to == "987654321")
    );
    assert!(
        targets
            .iter()
            .any(|t| t.channel == "whatsapp" && t.to == "15551234567@s.whatsapp.net")
    );
}

#[test]
fn test_resolve_disabled_channels_excluded() {
    let mut cfg = make_test_channels_config();
    cfg.discord.enabled = false;
    cfg.whatsapp.enabled = false;
    let targets = resolve_all_channel_targets_from_config(Some(&cfg));
    assert_eq!(targets.len(), 2);
    assert!(targets.iter().any(|t| t.channel == "slack"));
    assert!(targets.iter().any(|t| t.channel == "telegram"));
    assert!(!targets.iter().any(|t| t.channel == "discord"));
    assert!(!targets.iter().any(|t| t.channel == "whatsapp"));
}

#[test]
fn test_resolve_whatsapp_format() {
    assert_eq!(
        format_whatsapp_target("+15551234567"),
        "15551234567@s.whatsapp.net"
    );
    assert_eq!(
        format_whatsapp_target("15551234567"),
        "15551234567@s.whatsapp.net"
    );
    assert_eq!(
        format_whatsapp_target("15551234567@s.whatsapp.net"),
        "15551234567@s.whatsapp.net"
    );
}

#[test]
fn test_resolve_no_config() {
    let targets = resolve_all_channel_targets_from_config(None);
    assert!(targets.is_empty());
}

#[test]
fn test_first_concrete_target_skips_wildcard() {
    let list = vec!["*".to_string(), "user123".to_string()];
    assert_eq!(first_concrete_target(&list), "user123");
}

#[test]
fn test_first_concrete_target_empty_list() {
    let list: Vec<String> = vec![];
    assert_eq!(first_concrete_target(&list), "");
}

#[test]
fn test_first_concrete_target_only_wildcard() {
    let list = vec!["*".to_string()];
    assert_eq!(first_concrete_target(&list), "");
}

#[test]
fn test_first_concrete_target_no_wildcard() {
    let list = vec!["alice".to_string(), "bob".to_string()];
    assert_eq!(first_concrete_target(&list), "alice");
}

#[test]
fn test_format_whatsapp_target_with_plus() {
    assert_eq!(
        format_whatsapp_target("+441234567890"),
        "441234567890@s.whatsapp.net"
    );
}

#[test]
fn test_resolve_empty_allow_from_excluded() {
    let mut cfg = make_test_channels_config();
    cfg.slack.allow_from = vec![];
    let targets = resolve_all_channel_targets_from_config(Some(&cfg));
    // Slack should be excluded (empty allow_from -> first_concrete_target returns "")
    assert!(!targets.iter().any(|t| t.channel == "slack"));
}

#[test]
fn test_resolve_wildcard_only_excluded() {
    let mut cfg = make_test_channels_config();
    cfg.telegram.allow_from = vec!["*".to_string()];
    let targets = resolve_all_channel_targets_from_config(Some(&cfg));
    // Telegram wildcard has no concrete target -> excluded
    assert!(!targets.iter().any(|t| t.channel == "telegram"));
}

// --- Capabilities tests ---

#[test]
fn test_cron_capabilities() {
    use crate::agent::tools::base::SubagentAccess;
    let db = std::sync::Arc::new(
        crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"),
    );
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(cron_service, None, None);
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::ReadOnly);
    let read_only: Vec<&str> = caps
        .actions
        .iter()
        .filter(|a| a.read_only)
        .map(|a| a.name)
        .collect();
    let mutating: Vec<&str> = caps
        .actions
        .iter()
        .filter(|a| !a.read_only)
        .map(|a| a.name)
        .collect();
    assert!(read_only.contains(&"list"));
    assert!(read_only.contains(&"dlq_list"));
    assert!(mutating.contains(&"add"));
    assert!(mutating.contains(&"remove"));
    assert!(mutating.contains(&"run"));
    assert!(mutating.contains(&"dlq_replay"));
    assert!(mutating.contains(&"dlq_clear"));
}

#[test]
fn test_cron_actions_match_schema() {
    let db = std::sync::Arc::new(
        crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"),
    );
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(cron_service, None, None);
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

#[test]
fn test_parse_schedule_delay_seconds() {
    let params = json!({"delay_seconds": 300});
    let schedule = CronTool::parse_schedule(&params).unwrap();
    match schedule {
        CronSchedule::At { at_ms: Some(ms) } => {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64);
            // Should be ~300 seconds from now (allow 5s tolerance)
            let diff = ms - now_ms;
            assert!(
                diff > 295_000 && diff < 305_000,
                "expected ~300s delay, got {diff}ms"
            );
        }
        other => panic!("expected At schedule, got {other:?}"),
    }
}

#[test]
fn test_parse_schedule_delay_seconds_too_small() {
    let params = json!({"delay_seconds": 0});
    let result = CronTool::parse_schedule(&params);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.content.contains("at least 1"));
}

#[test]
fn test_parse_schedule_delay_seconds_too_large() {
    let params = json!({"delay_seconds": 99_999_999});
    let result = CronTool::parse_schedule(&params);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.content.contains("cannot exceed 1 year"));
}

#[test]
fn test_every_seconds_rejects_sub_minute() {
    // 1 second should be rejected
    let result = CronTool::parse_schedule(&json!({"every_seconds": 1}));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.content.contains("between 60"));

    // 59 seconds should be rejected
    let result = CronTool::parse_schedule(&json!({"every_seconds": 59}));
    assert!(result.is_err());

    // 60 seconds should be accepted
    let result = CronTool::parse_schedule(&json!({"every_seconds": 60}));
    assert!(result.is_ok());

    // 0 should be rejected
    let result = CronTool::parse_schedule(&json!({"every_seconds": 0}));
    assert!(result.is_err());
}

#[test]
fn test_every_seconds_rejects_over_one_year() {
    let result = CronTool::parse_schedule(&json!({"every_seconds": 31_536_001}));
    assert!(result.is_err());

    // Exactly one year should be accepted
    let result = CronTool::parse_schedule(&json!({"every_seconds": 31_536_000}));
    assert!(result.is_ok());
}

// --- Self-scheduling guard tests ---

#[tokio::test]
async fn test_cron_self_scheduling_guard_blocks_add() {
    let db = Arc::new(crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"));
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(cron_service, Some(make_test_channels_config()), None);

    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        crate::bus::meta::IS_CRON_JOB.to_string(),
        serde_json::Value::Bool(true),
    );
    let ctx = ExecutionContext {
        channel: "slack".to_string(),
        chat_id: "U123".to_string(),
        metadata,
        ..Default::default()
    };

    let params = json!({
        "action": "add",
        "message": "schedule another job",
        "every_seconds": 300
    });
    let result = tool.execute(params, &ctx).await.unwrap();
    assert!(
        result.is_error,
        "cron add should be blocked during cron execution"
    );
    assert!(result.content.contains("cannot schedule"));
}

#[tokio::test]
async fn test_cron_self_scheduling_guard_allows_list() {
    let db = Arc::new(crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"));
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(cron_service, Some(make_test_channels_config()), None);

    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        crate::bus::meta::IS_CRON_JOB.to_string(),
        serde_json::Value::Bool(true),
    );
    let ctx = ExecutionContext {
        channel: "slack".to_string(),
        chat_id: "U123".to_string(),
        metadata,
        ..Default::default()
    };

    // list should still work during cron execution (it's read-only)
    let params = json!({"action": "list"});
    let result = tool.execute(params, &ctx).await.unwrap();
    assert!(
        !result.is_error,
        "cron list should be allowed during cron execution"
    );
}

// --- Add action validation tests ---

#[tokio::test]
async fn test_cron_add_requires_message() {
    let db = Arc::new(crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"));
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(cron_service, Some(make_test_channels_config()), None);
    let ctx = ExecutionContext {
        channel: "slack".to_string(),
        chat_id: "U123".to_string(),
        ..Default::default()
    };

    let params = json!({"action": "add", "every_seconds": 300});
    let result = tool.execute(params, &ctx).await;
    // Should error (missing message param)
    assert!(result.is_err() || result.as_ref().unwrap().is_error);
}

#[tokio::test]
async fn test_cron_add_rejects_empty_message() {
    let db = Arc::new(crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"));
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(cron_service, Some(make_test_channels_config()), None);
    let ctx = ExecutionContext {
        channel: "slack".to_string(),
        chat_id: "U123".to_string(),
        ..Default::default()
    };

    let params = json!({"action": "add", "message": "   ", "every_seconds": 300});
    let result = tool.execute(params, &ctx).await.unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn test_cron_add_rejects_invalid_type() {
    let db = Arc::new(crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"));
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(cron_service, Some(make_test_channels_config()), None);
    let ctx = ExecutionContext {
        channel: "slack".to_string(),
        chat_id: "U123".to_string(),
        ..Default::default()
    };

    let params = json!({"action": "add", "message": "test", "type": "bogus", "every_seconds": 300});
    let result = tool.execute(params, &ctx).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid type"));
}

#[tokio::test]
async fn test_cron_add_echo_type() {
    let db = Arc::new(crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"));
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(
        cron_service.clone(),
        Some(make_test_channels_config()),
        None,
    );
    let ctx = ExecutionContext {
        channel: "slack".to_string(),
        chat_id: "U08G6HBC89X".to_string(),
        ..Default::default()
    };

    let params = json!({
        "action": "add",
        "message": "Standup in 5 minutes!",
        "type": "echo",
        "delay_seconds": 300
    });
    let result = tool.execute(params, &ctx).await.unwrap();
    assert!(
        !result.is_error,
        "echo type job should succeed: {}",
        result.content
    );
    assert!(result.content.contains("Created job"));
}

#[tokio::test]
async fn test_cron_add_and_list_and_remove() {
    let db = Arc::new(crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"));
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(
        cron_service.clone(),
        Some(make_test_channels_config()),
        None,
    );
    let ctx = ExecutionContext {
        channel: "slack".to_string(),
        chat_id: "U08G6HBC89X".to_string(),
        ..Default::default()
    };

    // Add a recurring job
    let params = json!({
        "action": "add",
        "message": "Check email and summarize",
        "every_seconds": 3600
    });
    let result = tool.execute(params, &ctx).await.unwrap();
    assert!(!result.is_error, "add failed: {}", result.content);
    // Extract job ID from "Created job '...' (id: XXXX, targets: ...)"
    let id = result
        .content
        .split("id: ")
        .nth(1)
        .and_then(|s| s.split(',').next())
        .unwrap();

    // List should show the job
    let list_params = json!({"action": "list"});
    let list_result = tool.execute(list_params, &ctx).await.unwrap();
    assert!(!list_result.is_error);
    assert!(
        list_result.content.contains(id),
        "list should contain job id: {}",
        list_result.content
    );
    assert!(list_result.content.contains("Check email"));

    // Remove the job
    let remove_params = json!({"action": "remove", "job_id": id});
    let remove_result = tool.execute(remove_params, &ctx).await.unwrap();
    assert!(!remove_result.is_error);
    assert!(remove_result.content.contains("Removed"));

    // List should be empty now
    let list_result = tool.execute(json!({"action": "list"}), &ctx).await.unwrap();
    assert!(list_result.content.contains("No scheduled jobs"));
}

#[tokio::test]
async fn test_cron_remove_nonexistent_job() {
    let db = Arc::new(crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"));
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(cron_service, None, None);
    let ctx = ExecutionContext {
        channel: "slack".to_string(),
        chat_id: "U123".to_string(),
        ..Default::default()
    };

    let params = json!({"action": "remove", "job_id": "nonexistent"});
    let result = tool.execute(params, &ctx).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("not found"));
}

#[tokio::test]
async fn test_cron_add_with_channels_all() {
    let db = Arc::new(crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"));
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(cron_service, Some(make_test_channels_config()), None);
    let ctx = ExecutionContext {
        channel: "slack".to_string(),
        chat_id: "U08G6HBC89X".to_string(),
        ..Default::default()
    };

    let params = json!({
        "action": "add",
        "message": "Good morning briefing",
        "cron_expr": "0 9 * * *",
        "channels": ["all"]
    });
    let result = tool.execute(params, &ctx).await.unwrap();
    assert!(
        !result.is_error,
        "add with all channels failed: {}",
        result.content
    );
    // Should target all 4 enabled channels
    assert!(result.content.contains("slack"));
    assert!(result.content.contains("discord"));
    assert!(result.content.contains("telegram"));
    assert!(result.content.contains("whatsapp"));
}

#[tokio::test]
async fn test_cron_add_no_context_rejects() {
    let db = Arc::new(crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"));
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(cron_service, Some(make_test_channels_config()), None);
    let ctx = ExecutionContext::default(); // empty channel/chat_id

    let params = json!({
        "action": "add",
        "message": "test",
        "every_seconds": 300
    });
    let result = tool.execute(params, &ctx).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("no session context"));
}

// --- Schedule parsing edge cases ---

#[test]
fn test_parse_schedule_cron_expr() {
    let params = json!({"cron_expr": "0 9 * * 1-5"});
    let schedule = CronTool::parse_schedule(&params).unwrap();
    match schedule {
        CronSchedule::Cron { expr, .. } => {
            assert_eq!(expr, Some("0 9 * * 1-5".to_string()));
        }
        other => panic!("expected Cron schedule, got {other:?}"),
    }
}

#[test]
fn test_parse_schedule_at_time() {
    let params = json!({"at_time": "2099-01-15T09:00:00-05:00"});
    let schedule = CronTool::parse_schedule(&params).unwrap();
    match schedule {
        CronSchedule::At { at_ms: Some(ms) } => {
            assert!(ms > 0, "at_ms should be positive");
        }
        other => panic!("expected At schedule, got {other:?}"),
    }
}

#[test]
fn test_parse_schedule_no_schedule_rejects() {
    let params = json!({"message": "test"});
    let result = CronTool::parse_schedule(&params);
    assert!(result.is_err());
}

#[test]
fn test_parse_schedule_event_pattern() {
    let params = json!({"event_pattern": "(?i)deploy.*prod"});
    let schedule = CronTool::parse_schedule(&params).unwrap();
    match schedule {
        CronSchedule::Event { pattern, .. } => {
            assert_eq!(pattern, Some("(?i)deploy.*prod".to_string()));
        }
        other => panic!("expected Event schedule, got {other:?}"),
    }
}

#[test]
fn test_parse_schedule_invalid_event_pattern() {
    // Invalid regex should be rejected
    let params = json!({"event_pattern": "[invalid"});
    let result = CronTool::parse_schedule(&params);
    assert!(result.is_err());
}

// --- Button builder tests ---

fn make_test_job(id: &str, name: &str, enabled: bool) -> CronJob {
    CronJob {
        id: id.to_string(),
        name: name.to_string(),
        enabled,
        schedule: CronSchedule::Every {
            every_ms: Some(3_600_000),
        },
        payload: CronPayload {
            kind: "agent_turn".to_string(),
            message: "test".to_string(),
            agent_echo: true,
            targets: vec![],
        },
        state: CronJobState::default(),
        created_at_ms: 0,
        updated_at_ms: 0,
        delete_after_run: false,
        expires_at_ms: None,
        max_runs: None,
        cooldown_secs: None,
        max_concurrent: None,
    }
}

#[test]
fn test_build_job_buttons_enabled_job() {
    let jobs = vec![make_test_job("abc123", "Daily report", true)];
    let buttons = build_job_buttons(&jobs);
    assert_eq!(buttons.len(), 2);
    // First button: Pause
    assert_eq!(buttons[0]["id"], "pause-job-abc123");
    assert_eq!(buttons[0]["label"], "Pause: Daily report");
    assert_eq!(buttons[0]["style"], "primary");
    let ctx: serde_json::Value =
        serde_json::from_str(buttons[0]["context"].as_str().unwrap()).unwrap();
    assert_eq!(ctx["tool"], "cron");
    assert_eq!(ctx["job_id"], "abc123");
    assert_eq!(ctx["action"], "pause");
    // Second button: Remove
    assert_eq!(buttons[1]["id"], "remove-job-abc123");
    assert_eq!(buttons[1]["label"], "Remove: Daily report");
    assert_eq!(buttons[1]["style"], "danger");
}

#[test]
fn test_build_job_buttons_disabled_job() {
    let jobs = vec![make_test_job("xyz789", "Nightly sync", false)];
    let buttons = build_job_buttons(&jobs);
    assert_eq!(buttons.len(), 2);
    // First button: Resume (not Pause)
    assert_eq!(buttons[0]["id"], "resume-job-xyz789");
    assert_eq!(buttons[0]["label"], "Resume: Nightly sync");
    assert_eq!(buttons[0]["style"], "success");
    let ctx: serde_json::Value =
        serde_json::from_str(buttons[0]["context"].as_str().unwrap()).unwrap();
    assert_eq!(ctx["action"], "resume");
}

#[test]
fn test_build_job_buttons_max_5() {
    let jobs: Vec<CronJob> = (0..10)
        .map(|i| make_test_job(&format!("id{i}"), &format!("Job {i}"), true))
        .collect();
    let buttons = build_job_buttons(&jobs);
    assert_eq!(buttons.len(), 5);
}

#[test]
fn test_build_job_buttons_empty() {
    let buttons = build_job_buttons(&[]);
    assert!(buttons.is_empty());
}

#[test]
fn test_build_job_buttons_long_name_truncated() {
    let jobs = vec![make_test_job(
        "id1",
        "A very long job name that exceeds limits",
        true,
    )];
    let buttons = build_job_buttons(&jobs);
    // "Pause: " prefix + 20 char max → truncated with "..."
    let label = buttons[0]["label"].as_str().unwrap();
    assert!(label.starts_with("Pause: "));
    assert!(label.ends_with("..."));
    assert!(label.chars().count() <= 27); // "Pause: " (7) + 20 max
}

#[test]
fn test_truncate_label_short() {
    assert_eq!(truncate_label("Pause: ", "Daily", 20), "Pause: Daily");
}

#[test]
fn test_truncate_label_exact_boundary() {
    let name = "A".repeat(20);
    assert_eq!(truncate_label("X: ", &name, 20), format!("X: {name}"));
}

#[test]
fn test_truncate_label_over_limit() {
    let name = "A".repeat(25);
    let result = truncate_label("X: ", &name, 20);
    assert!(result.ends_with("..."));
    // prefix (3) + 17 chars + "..." = 23 chars total
    assert_eq!(result.chars().count(), 23);
}

#[test]
fn test_truncate_label_unicode() {
    // Each emoji is one char
    let name = "\u{1f600}".repeat(25); // 25 emoji chars
    let result = truncate_label("R: ", &name, 20);
    assert!(result.ends_with("..."));
    // Should not panic on multi-byte boundaries
}

#[test]
fn test_with_buttons_empty() {
    let result = ToolResult::new("test");
    let result = with_buttons(result, vec![]);
    assert!(result.metadata.is_none());
}

#[test]
fn test_with_buttons_attaches_metadata() {
    let result = ToolResult::new("test");
    let buttons = vec![json!({"id": "b1", "label": "Click"})];
    let result = with_buttons(result, buttons);
    let meta = result.metadata.as_ref().unwrap();
    let btns = meta["suggested_buttons"].as_array().unwrap();
    assert_eq!(btns.len(), 1);
    assert_eq!(btns[0]["id"], "b1");
}

#[tokio::test]
async fn test_list_returns_suggested_buttons() {
    let db = Arc::new(crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"));
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(
        cron_service.clone(),
        Some(make_test_channels_config()),
        None,
    );
    let ctx = ExecutionContext {
        channel: "slack".to_string(),
        chat_id: "U08G6HBC89X".to_string(),
        ..Default::default()
    };

    // Add a job first
    let params = json!({
        "action": "add",
        "message": "Check email",
        "every_seconds": 3600
    });
    let add_result = tool.execute(params, &ctx).await.unwrap();
    assert!(!add_result.is_error, "add failed: {}", add_result.content);

    // List should return buttons
    let list_result = tool.execute(json!({"action": "list"}), &ctx).await.unwrap();
    assert!(!list_result.is_error);
    let meta = list_result
        .metadata
        .as_ref()
        .expect("list should return metadata with buttons");
    let buttons = meta["suggested_buttons"].as_array().unwrap();
    // 1 job → 2 buttons (Pause + Remove)
    assert_eq!(buttons.len(), 2);
    assert!(buttons[0]["label"].as_str().unwrap().starts_with("Pause: "));
    assert!(
        buttons[1]["label"]
            .as_str()
            .unwrap()
            .starts_with("Remove: ")
    );
}

#[tokio::test]
async fn test_list_empty_has_no_buttons() {
    let db = Arc::new(crate::agent::memory::memory_db::MemoryDB::new(":memory:").expect("test db"));
    let cron_service = Arc::new(CronService::new(db));
    let tool = CronTool::new(cron_service, None, None);
    let ctx = ExecutionContext {
        channel: "slack".to_string(),
        chat_id: "U123".to_string(),
        ..Default::default()
    };

    let result = tool.execute(json!({"action": "list"}), &ctx).await.unwrap();
    assert!(result.content.contains("No scheduled jobs"));
    assert!(result.metadata.is_none());
}
