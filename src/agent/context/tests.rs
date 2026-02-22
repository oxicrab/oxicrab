use super::*;

fn create_test_context(workspace: &Path) -> ContextBuilder {
    std::fs::create_dir_all(workspace.join("memory")).unwrap();
    ContextBuilder::new(workspace).unwrap()
}

#[test]
fn test_default_identity_contains_required_sections() {
    let tmp = tempfile::TempDir::new().unwrap();
    let _ctx = create_test_context(tmp.path());

    let identity =
        ContextBuilder::get_default_identity("2026-02-09", "EST", "Rust 0.1.3", "/workspace");

    assert!(identity.contains("# oxicrab"), "missing heading");
    assert!(identity.contains("## Capabilities"), "missing capabilities");
    assert!(identity.contains("## Current Context"), "missing context");
    assert!(
        !identity.contains("## Behavioral Rules"),
        "fallback should not contain behavioral rules — AGENTS.md is the single source"
    );
    assert!(
        identity.contains("**Date**: 2026-02-09"),
        "missing date injection"
    );
    assert!(
        identity.contains("**Runtime**: Rust 0.1.3"),
        "missing runtime injection"
    );
    assert!(
        identity.contains("**Timezone**: EST"),
        "missing timezone injection"
    );
    assert!(
        identity.contains("**Workspace**: /workspace"),
        "missing workspace injection"
    );
    assert!(
        identity.contains("/workspace/memory/MEMORY.md"),
        "missing memory path"
    );
}

#[test]
fn test_default_identity_has_no_behavioral_rules() {
    let tmp = tempfile::TempDir::new().unwrap();
    let _ctx = create_test_context(tmp.path());

    let identity = ContextBuilder::get_default_identity("now", "UTC", "Rust 0.1.3", "/ws");

    assert!(
        !identity.contains("## Behavioral Rules"),
        "fallback should not contain behavioral rules — AGENTS.md is the single source"
    );
}

#[test]
fn test_build_identity_with_context_appends_context() {
    let tmp = tempfile::TempDir::new().unwrap();
    let _ctx = create_test_context(tmp.path());

    let result = ContextBuilder::build_identity_with_context(
        "# Custom Bot\n\nI am a custom bot.",
        "2026-02-09",
        "EST",
        "Rust 0.1.3",
        "/my/workspace",
    );

    assert!(result.starts_with("# Custom Bot"));
    assert!(result.contains("I am a custom bot."));
    assert!(result.contains("## Current Context"));
    assert!(result.contains("**Date**: 2026-02-09"));
    assert!(result.contains("/my/workspace/memory/MEMORY.md"));
    // Should NOT contain hardcoded behavioral rules
    assert!(
        !result.contains("## Behavioral Rules"),
        "should not append behavioral rules when AGENTS.md provides them"
    );
}

#[test]
fn test_identity_uses_file_when_present() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = create_test_context(tmp.path());

    std::fs::write(
        tmp.path().join("AGENTS.md"),
        "# My Bot\n\nCustom identity content.",
    )
    .unwrap();

    let identity = ctx.get_identity();

    assert!(identity.contains("# My Bot"));
    assert!(identity.contains("Custom identity content."));
    assert!(identity.contains("## Current Context"));
}

#[test]
fn test_identity_falls_back_when_no_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = create_test_context(tmp.path());
    // No AGENTS.md written

    let identity = ctx.get_identity();

    assert!(identity.contains("# oxicrab"));
    assert!(
        !identity.contains("## Behavioral Rules"),
        "fallback should not contain behavioral rules — AGENTS.md is the single source"
    );
}

#[test]
fn test_bootstrap_loads_user_md() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut ctx = create_test_context(tmp.path());

    std::fs::write(tmp.path().join("USER.md"), "# User\nTimezone: ET").unwrap();

    let bootstrap = ctx.load_bootstrap_files();

    assert!(bootstrap.contains("## USER.md"));
    assert!(bootstrap.contains("Timezone: ET"));
}

#[test]
fn test_bootstrap_loads_tools_md() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut ctx = create_test_context(tmp.path());

    std::fs::write(tmp.path().join("TOOLS.md"), "# Tools\nUse bash for shell.").unwrap();

    let bootstrap = ctx.load_bootstrap_files();

    assert!(bootstrap.contains("## TOOLS.md"));
    assert!(bootstrap.contains("Use bash for shell."));
}

#[test]
fn test_bootstrap_skips_agents_md() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut ctx = create_test_context(tmp.path());

    std::fs::write(
        tmp.path().join("AGENTS.md"),
        "# Identity\nShould not appear.",
    )
    .unwrap();

    let bootstrap = ctx.load_bootstrap_files();

    assert!(
        !bootstrap.contains("## AGENTS.md"),
        "AGENTS.md should be handled separately, not in bootstrap"
    );
}

#[test]
fn test_bootstrap_skips_missing_files() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut ctx = create_test_context(tmp.path());
    // No files created

    let bootstrap = ctx.load_bootstrap_files();

    assert!(bootstrap.is_empty());
}

#[test]
fn test_bootstrap_caching() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut ctx = create_test_context(tmp.path());

    std::fs::write(tmp.path().join("USER.md"), "# User\nv1").unwrap();

    let first = ctx.load_bootstrap_files();
    assert!(first.contains("v1"));
    assert!(ctx.bootstrap_cache.is_some());

    // Second call should return cached version (same mtime)
    let second = ctx.load_bootstrap_files();
    assert_eq!(first, second);
}

#[tokio::test]
async fn test_build_messages_includes_sender_id() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut ctx = create_test_context(tmp.path());

    let messages = ctx
        .build_messages(
            &[],
            "hello",
            Some("telegram"),
            Some("123"),
            Some("user42"),
            vec![],
            false,
        )
        .unwrap();

    let system_msg = &messages[0];
    assert!(
        system_msg.content.contains("Sender: user42"),
        "system prompt should include sender ID"
    );
}

#[tokio::test]
async fn test_build_messages_no_sender_id() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut ctx = create_test_context(tmp.path());

    let messages = ctx
        .build_messages(
            &[],
            "hello",
            Some("telegram"),
            Some("123"),
            None,
            vec![],
            false,
        )
        .unwrap();

    let system_msg = &messages[0];
    assert!(
        !system_msg.content.contains("Sender:"),
        "system prompt should not include sender line when None"
    );
}

#[tokio::test]
async fn test_build_messages_with_images() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut ctx = create_test_context(tmp.path());

    let images = vec![crate::providers::base::ImageData {
        media_type: "image/jpeg".to_string(),
        data: "base64encodeddata".to_string(),
    }];
    let messages = ctx
        .build_messages(
            &[],
            "what is this",
            Some("telegram"),
            Some("123"),
            Some("user42"),
            images,
            false,
        )
        .unwrap();

    // Last message should be user with images
    let user_msg = messages.last().unwrap();
    assert_eq!(user_msg.role, "user");
    assert_eq!(user_msg.images.len(), 1);
    assert_eq!(user_msg.images[0].media_type, "image/jpeg");
    assert_eq!(user_msg.images[0].data, "base64encodeddata");
    assert!(user_msg.content.contains("what is this"));
}

#[tokio::test]
async fn test_build_messages_without_images() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut ctx = create_test_context(tmp.path());

    let messages = ctx
        .build_messages(
            &[],
            "hello",
            Some("telegram"),
            Some("123"),
            None,
            vec![],
            false,
        )
        .unwrap();

    let user_msg = messages.last().unwrap();
    assert_eq!(user_msg.role, "user");
    assert!(user_msg.images.is_empty());
}

#[test]
fn test_channel_formatting_hint_discord() {
    let hint = ContextBuilder::channel_formatting_hint("discord");
    assert!(hint.is_some());
    assert!(hint.unwrap().contains("NOT tables"));
}

#[test]
fn test_channel_formatting_hint_unknown() {
    assert!(ContextBuilder::channel_formatting_hint("cli").is_none());
    assert!(ContextBuilder::channel_formatting_hint("unknown").is_none());
}

#[tokio::test]
async fn test_build_messages_includes_channel_hint() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut ctx = create_test_context(tmp.path());

    let messages = ctx
        .build_messages(
            &[],
            "hello",
            Some("discord"),
            Some("123"),
            Some("user42"),
            vec![],
            false,
        )
        .unwrap();

    let system_msg = &messages[0];
    assert!(
        system_msg.content.contains("NOT tables"),
        "system prompt should include discord formatting hint"
    );
}

#[test]
fn test_default_identity_has_tool_directness_rule() {
    let identity =
        ContextBuilder::get_default_identity("2026-02-21", "UTC", "Rust 0.1.3", "/workspace");
    assert!(
        identity.contains("call them directly"),
        "default identity should include tool directness rule"
    );
}

#[test]
fn test_bootstrap_files_constant() {
    assert!(BOOTSTRAP_FILES.contains(&"USER.md"));
    assert!(BOOTSTRAP_FILES.contains(&"TOOLS.md"));
    assert!(BOOTSTRAP_FILES.contains(&"AGENTS.md"));
    assert!(
        !BOOTSTRAP_FILES.contains(&"SOUL.md"),
        "SOUL.md was consolidated"
    );
    assert!(
        !BOOTSTRAP_FILES.contains(&"IDENTITY.md"),
        "IDENTITY.md was renamed to AGENTS.md"
    );
}

#[tokio::test]
async fn test_build_messages_group_excludes_personal_memory() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut ctx = create_test_context(tmp.path());

    // Write personal memory
    let memory_dir = tmp.path().join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(memory_dir.join("MEMORY.md"), "# Personal\nMy secret notes.").unwrap();

    // DM mode: should include personal memory
    let dm_msgs = ctx
        .build_messages(
            &[],
            "hello",
            Some("telegram"),
            Some("123"),
            None,
            vec![],
            false,
        )
        .unwrap();
    let dm_system = &dm_msgs[0].content;
    assert!(
        dm_system.contains("My secret notes"),
        "DM should include personal memory"
    );

    // Group mode: should NOT include personal memory
    let group_msgs = ctx
        .build_messages(
            &[],
            "hello",
            Some("telegram"),
            Some("-123"),
            None,
            vec![],
            true,
        )
        .unwrap();
    let group_system = &group_msgs[0].content;
    assert!(
        !group_system.contains("My secret notes"),
        "group chat should NOT include personal memory"
    );
}
