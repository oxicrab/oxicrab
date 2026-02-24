use super::*;

#[test]
fn test_action_claims_positive() {
    let cases = [
        "I've updated the configuration file.",
        "I have created the new module for you.",
        "I wrote the function as requested.",
        "I deleted the old config.",
        "I've configured the settings.",
        "I've saved the changes to disk.",
        "I've scheduled the cron job.",
        "Changes have been applied to the project.",
        "File has been updated successfully.",
        "Config was modified as requested.",
        "I'VE UPDATED THE FILE.",
        "i've written the code.",
        "Sure, here's what I did:\n\nI've updated the configuration to use the new API endpoint.\nLet me know if you need anything else.",
        "I enabled the feature flag.",
        "I've deployed the changes.",
        "Updates were made to the database schema.",
        "I tested all the tools.",
        "I've executed the commands.",
        "I've fetched the latest data.",
        "I verified all the results.",
        "I searched for the information.",
        "I listed all the directory contents.",
        "All Tools Working:",
        "All tools are fully functional!",
        "All tests passed successfully.",
        "All tests were successful.",
        "Successfully executed the command.",
        "Successfully tested all endpoints.",
        "Already completed the migration.",
        // Terse-format action claims (no first-person pronoun)
        "Created: Send out the form — due tomorrow at 10:00 AM.",
        "Updated: config.json with the new API key.",
        "Deleted: old-backup.tar.gz",
        "Done! The task has been set up.",
        "Sent: the email to the team.",
        "Scheduled: deployment for tomorrow at 9am.",
        "Completed: all items on the checklist.",
        "Saved: your preferences.",
        "Added! The new entry is in the database.",
        "Marked as complete: Call Sun Logistics",
        "\nCreated: a new issue in the tracker.",
        // Prefix-word evasion patterns (LLM adds word before action verb)
        "Both created:\n• Feed the cat\n• Feed the dog",
        "Task created: Click the box — due tomorrow at 10:00 AM.",
        "Job scheduled: one-shot at 4pm today.",
        "All done! Everything is configured.",
    ];
    for text in cases {
        assert!(contains_action_claims(text), "should match: {}", text);
    }
}

#[test]
fn test_action_claims_negative() {
    let cases = [
        "Here's how you can update the file.",
        "Would you like me to create a new file?",
        "The function returns a string value.",
        "To update the config, you need to edit settings.json.",
        "Hello! How can I help you today?",
        "You updated the file yesterday.",
        // "Created" in descriptive context (not terse action claim)
        "Tasks created before Monday will be archived.",
        "Created tasks can be viewed in the dashboard.",
    ];
    for text in cases {
        assert!(!contains_action_claims(text), "should NOT match: {}", text);
    }
}

#[test]
fn test_tool_name_mentions() {
    let tools = vec![
        "web_search".to_string(),
        "weather".to_string(),
        "cron".to_string(),
        "reddit".to_string(),
        "exec".to_string(),
    ];
    // 4+ tool names -> triggers
    assert!(mentions_multiple_tools(
        "## Tool Test Results\n- web_search - Found news\n- weather - 45°F\n- cron - 5 jobs\n- reddit - 10 posts",
        &tools
    ));
    // 1 tool name -> doesn't trigger
    let tools3 = vec![
        "web_search".to_string(),
        "weather".to_string(),
        "cron".to_string(),
    ];
    assert!(!mentions_multiple_tools(
        "I can help you with web_search if you'd like.",
        &tools3
    ));
    // 2 tool names -> doesn't trigger
    assert!(!mentions_multiple_tools(
        "The web_search and weather tools are available.",
        &tools3
    ));
}

#[test]
fn test_silent_response_prefix() {
    assert!("[SILENT] Internal note recorded.".starts_with("[SILENT]"));
    assert!(!"[silent] This should pass through.".starts_with("[SILENT]"));
    assert!(!"Here is a normal response.".starts_with("[SILENT]"));
}

#[test]
fn test_false_no_tools_claims() {
    let positives = [
        "I don't have access to tools to help with that.",
        "I cannot have access to any tools.",
        "I'm unable to use tools directly.",
        "No tools are available to me.",
    ];
    for text in positives {
        assert!(is_false_no_tools_claim(text), "should match: {}", text);
    }
    let negatives = [
        "Here's how to use the tools in this project.",
        "I'll use the exec tool to run that command.",
    ];
    for text in negatives {
        assert!(!is_false_no_tools_claim(text), "should NOT match: {}", text);
    }
}

// --- Parallel tool execution tests ---

use crate::agent::tools::base::{Tool, ToolResult};
use crate::providers::base::ToolCallRequest;
use async_trait::async_trait;
use std::sync::Arc;

/// A mock tool that sleeps for a duration then returns a result.
struct MockTool {
    tool_name: String,
    delay_ms: u64,
    response: String,
}

#[async_trait]
impl Tool for MockTool {
    fn name(&self) -> &str {
        &self.tool_name
    }
    fn description(&self) -> &'static str {
        "mock"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &ExecutionContext,
    ) -> anyhow::Result<ToolResult> {
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        Ok(ToolResult::new(self.response.clone()))
    }
}

/// A mock tool that returns an error.
struct ErrorTool;

#[async_trait]
impl Tool for ErrorTool {
    fn name(&self) -> &'static str {
        "error_tool"
    }
    fn description(&self) -> &'static str {
        "mock"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &ExecutionContext,
    ) -> anyhow::Result<ToolResult> {
        Err(anyhow::anyhow!("intentional error"))
    }
}

/// A mock tool that panics.
struct PanicTool;

#[async_trait]
impl Tool for PanicTool {
    fn name(&self) -> &'static str {
        "panic_tool"
    }
    fn description(&self) -> &'static str {
        "mock"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &ExecutionContext,
    ) -> anyhow::Result<ToolResult> {
        panic!("intentional panic");
    }
}

fn make_tool_call(id: &str, name: &str) -> ToolCallRequest {
    ToolCallRequest {
        id: id.to_string(),
        name: name.to_string(),
        arguments: serde_json::json!({}),
    }
}

fn empty_tools() -> Vec<String> {
    vec![]
}

fn make_registry_with(tools_list: Vec<Arc<dyn Tool>>) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    for tool in tools_list {
        registry.register(tool);
    }
    registry
}

#[tokio::test]
async fn test_parallel_tool_execution_ordering() {
    // 3 tools with different delays — results must come back in call order
    let registry = make_registry_with(vec![
        Arc::new(MockTool {
            tool_name: "slow".into(),
            delay_ms: 80,
            response: "slow_result".into(),
        }),
        Arc::new(MockTool {
            tool_name: "fast".into(),
            delay_ms: 10,
            response: "fast_result".into(),
        }),
        Arc::new(MockTool {
            tool_name: "medium".into(),
            delay_ms: 40,
            response: "medium_result".into(),
        }),
    ]);
    let registry = Arc::new(registry);

    let calls = [
        make_tool_call("1", "slow"),
        make_tool_call("2", "fast"),
        make_tool_call("3", "medium"),
    ];

    // Spawn in parallel (same pattern as the production code)
    let handles: Vec<_> = calls
        .iter()
        .map(|tc| {
            let reg = registry.clone();
            let tc_name = tc.name.clone();
            let tc_args = tc.arguments.clone();
            let available = empty_tools();
            tokio::task::spawn(async move {
                execute_tool_call(
                    &reg,
                    &tc_name,
                    &tc_args,
                    &available,
                    &ExecutionContext::default(),
                    &[],
                    None,
                )
                .await
            })
        })
        .collect();

    let results: Vec<_> = futures_util::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // Results must be in the same order as the calls, not execution completion order
    assert_eq!(results[0].0, "slow_result");
    assert_eq!(results[1].0, "fast_result");
    assert_eq!(results[2].0, "medium_result");
    assert!(!results[0].1);
    assert!(!results[1].1);
    assert!(!results[2].1);
}

#[tokio::test]
async fn test_single_tool_no_parallel_overhead() {
    let registry = make_registry_with(vec![Arc::new(MockTool {
        tool_name: "only".into(),
        delay_ms: 0,
        response: "only_result".into(),
    })]);

    let (result, is_error) = execute_tool_call(
        &registry,
        "only",
        &serde_json::json!({}),
        &empty_tools(),
        &ExecutionContext::default(),
        &[],
        None,
    )
    .await;

    assert_eq!(result, "only_result");
    assert!(!is_error);
}

#[tokio::test]
async fn test_parallel_tool_one_panics() {
    let registry = make_registry_with(vec![
        Arc::new(MockTool {
            tool_name: "good1".into(),
            delay_ms: 0,
            response: "result1".into(),
        }),
        Arc::new(PanicTool),
        Arc::new(MockTool {
            tool_name: "good2".into(),
            delay_ms: 0,
            response: "result2".into(),
        }),
    ]);
    let registry = Arc::new(registry);

    let calls = [
        make_tool_call("1", "good1"),
        make_tool_call("2", "panic_tool"),
        make_tool_call("3", "good2"),
    ];

    let handles: Vec<_> = calls
        .iter()
        .map(|tc| {
            let reg = registry.clone();
            let tc_name = tc.name.clone();
            let tc_args = tc.arguments.clone();
            let available = empty_tools();
            tokio::task::spawn(async move {
                execute_tool_call(
                    &reg,
                    &tc_name,
                    &tc_args,
                    &available,
                    &ExecutionContext::default(),
                    &[],
                    None,
                )
                .await
            })
        })
        .collect();

    let results: Vec<_> = futures_util::future::join_all(handles)
        .await
        .into_iter()
        .map(|join_result| match join_result {
            Ok(result) => result,
            Err(_) => ("Tool crashed unexpectedly".to_string(), true),
        })
        .collect();

    // Good tools succeed
    assert_eq!(results[0].0, "result1");
    assert!(!results[0].1);
    assert_eq!(results[2].0, "result2");
    assert!(!results[2].1);
    // Panicked tool gets error
    assert!(results[1].0.contains("crashed"));
    assert!(results[1].1);
}

#[tokio::test]
async fn test_parallel_tool_one_errors() {
    let registry = make_registry_with(vec![
        Arc::new(MockTool {
            tool_name: "good".into(),
            delay_ms: 0,
            response: "good_result".into(),
        }),
        Arc::new(ErrorTool),
        Arc::new(MockTool {
            tool_name: "also_good".into(),
            delay_ms: 0,
            response: "also_good_result".into(),
        }),
    ]);
    let registry = Arc::new(registry);

    let calls = [
        make_tool_call("1", "good"),
        make_tool_call("2", "error_tool"),
        make_tool_call("3", "also_good"),
    ];

    let handles: Vec<_> = calls
        .iter()
        .map(|tc| {
            let reg = registry.clone();
            let tc_name = tc.name.clone();
            let tc_args = tc.arguments.clone();
            let available = empty_tools();
            tokio::task::spawn(async move {
                execute_tool_call(
                    &reg,
                    &tc_name,
                    &tc_args,
                    &available,
                    &ExecutionContext::default(),
                    &[],
                    None,
                )
                .await
            })
        })
        .collect();

    let results: Vec<_> = futures_util::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // Good tools unaffected
    assert_eq!(results[0].0, "good_result");
    assert!(!results[0].1);
    assert_eq!(results[2].0, "also_good_result");
    assert!(!results[2].1);
    // Error tool marked as error
    assert!(results[1].0.contains("Tool execution failed"));
    assert!(results[1].1);
}

// --- Unknown tool error improvement tests ---

#[tokio::test]
async fn test_unknown_tool_lists_available() {
    let registry = make_registry_with(vec![
        Arc::new(MockTool {
            tool_name: "read_file".into(),
            delay_ms: 0,
            response: "ok".into(),
        }),
        Arc::new(MockTool {
            tool_name: "write_file".into(),
            delay_ms: 0,
            response: "ok".into(),
        }),
        Arc::new(MockTool {
            tool_name: "exec".into(),
            delay_ms: 0,
            response: "ok".into(),
        }),
    ]);
    let available = vec![
        "read_file".to_string(),
        "write_file".to_string(),
        "exec".to_string(),
    ];
    let (result, is_error) = execute_tool_call(
        &registry,
        "nonexistent_tool",
        &serde_json::json!({}),
        &available,
        &ExecutionContext::default(),
        &[],
        None,
    )
    .await;
    assert!(is_error);
    assert!(result.contains("does not exist"));
    assert!(result.contains("read_file"));
    assert!(result.contains("write_file"));
    assert!(result.contains("exec"));
}

// --- Schema validation tests ---

/// A mock tool with a defined parameter schema for validation tests.
struct SchemaTestTool;

#[async_trait]
impl Tool for SchemaTestTool {
    fn name(&self) -> &'static str {
        "schema_test"
    }
    fn description(&self) -> &'static str {
        "test tool with schema"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "count": { "type": "integer" },
                "verbose": { "type": "boolean" },
                "tags": { "type": "array" },
                "options": { "type": "object" }
            },
            "required": ["query"]
        })
    }
    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &ExecutionContext,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::new("ok".to_string()))
    }
}

#[test]
fn test_validate_params_missing_required() {
    let tool = SchemaTestTool;
    let params = serde_json::json!({});
    let result = validate_tool_params(&tool, &params);
    assert!(result.is_some());
    let msg = result.unwrap();
    assert!(msg.contains("missing required parameter 'query'"));
}

#[test]
fn test_validate_params_wrong_type() {
    let tool = SchemaTestTool;
    // query should be string, but we pass a number
    let params = serde_json::json!({"query": 42});
    let result = validate_tool_params(&tool, &params);
    assert!(result.is_some());
    let msg = result.unwrap();
    assert!(msg.contains("parameter 'query' should be string but got number"));
}

#[test]
fn test_validate_params_valid() {
    let tool = SchemaTestTool;
    let params = serde_json::json!({"query": "hello", "count": 5, "verbose": true});
    let result = validate_tool_params(&tool, &params);
    assert!(result.is_none());
}

#[test]
fn test_validate_params_no_required() {
    // MockTool has empty schema (no required array) — should always pass
    let tool = MockTool {
        tool_name: "no_schema".into(),
        delay_ms: 0,
        response: "ok".into(),
    };
    let params = serde_json::json!({});
    let result = validate_tool_params(&tool, &params);
    assert!(result.is_none());
}

#[test]
fn test_validate_params_optional_missing_ok() {
    let tool = SchemaTestTool;
    // Only required field "query" is provided; optional fields omitted — should pass
    let params = serde_json::json!({"query": "test"});
    let result = validate_tool_params(&tool, &params);
    assert!(result.is_none());
}

#[tokio::test]
async fn test_validation_rejects_before_execution() {
    // Tool with required param "query" — call without it, should get validation error
    let registry = make_registry_with(vec![Arc::new(SchemaTestTool)]);
    let (result, is_error) = execute_tool_call(
        &registry,
        "schema_test",
        &serde_json::json!({}),
        &empty_tools(),
        &ExecutionContext::default(),
        &[],
        None,
    )
    .await;
    assert!(is_error);
    assert!(result.contains("missing required parameter 'query'"));
}

// --- Approval gate tests ---

/// A mock tool that requires approval (simulates an `AttenuatedMcpTool`).
struct ApprovalRequiredTool;

#[async_trait]
impl Tool for ApprovalRequiredTool {
    fn name(&self) -> &'static str {
        "untrusted_mcp_tool"
    }
    fn description(&self) -> &'static str {
        "mock untrusted tool"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &ExecutionContext,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::new("should not reach here"))
    }
    fn requires_approval(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn test_requires_approval_blocks_execution() {
    let registry = make_registry_with(vec![Arc::new(ApprovalRequiredTool)]);
    let (result, is_error) = execute_tool_call(
        &registry,
        "untrusted_mcp_tool",
        &serde_json::json!({}),
        &empty_tools(),
        &ExecutionContext::default(),
        &[],
        None,
    )
    .await;
    assert!(is_error);
    assert!(result.contains("requires approval"));
    assert!(result.contains("untrusted MCP server"));
}

#[tokio::test]
async fn test_normal_tool_not_blocked_by_approval() {
    let registry = make_registry_with(vec![Arc::new(MockTool {
        tool_name: "safe_tool".into(),
        delay_ms: 0,
        response: "safe_result".into(),
    })]);
    let (result, is_error) = execute_tool_call(
        &registry,
        "safe_tool",
        &serde_json::json!({}),
        &empty_tools(),
        &ExecutionContext::default(),
        &[],
        None,
    )
    .await;
    assert!(!is_error);
    assert_eq!(result, "safe_result");
}

// --- Image loading tests ---

// Minimal valid magic bytes for each format
const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
const PNG_MAGIC: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
const GIF_MAGIC: &[u8] = b"GIF89a";
fn webp_magic() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&[0x00; 4]); // file size placeholder
    data.extend_from_slice(b"WEBP");
    data
}

#[test]
fn test_load_and_encode_images_valid_jpg() {
    let tmp = tempfile::TempDir::new().unwrap();
    let img_path = tmp.path().join("test.jpg");
    std::fs::write(&img_path, JPEG_MAGIC).unwrap();

    let paths = vec![img_path.to_string_lossy().to_string()];
    let images = load_and_encode_images(&paths);

    assert_eq!(images.len(), 1);
    assert_eq!(images[0].media_type, "image/jpeg");
    assert!(!images[0].data.is_empty());
}

#[test]
fn test_load_and_encode_images_multiple_formats() {
    let tmp = tempfile::TempDir::new().unwrap();

    std::fs::write(tmp.path().join("test.jpg"), JPEG_MAGIC).unwrap();
    std::fs::write(tmp.path().join("test.png"), PNG_MAGIC).unwrap();
    std::fs::write(tmp.path().join("test.gif"), GIF_MAGIC).unwrap();
    std::fs::write(tmp.path().join("test.webp"), webp_magic()).unwrap();

    let paths: Vec<String> = ["jpg", "png", "gif", "webp"]
        .iter()
        .map(|ext| {
            tmp.path()
                .join(format!("test.{}", ext))
                .to_string_lossy()
                .to_string()
        })
        .collect();
    let images = load_and_encode_images(&paths);

    assert_eq!(images.len(), 4);
    assert_eq!(images[0].media_type, "image/jpeg");
    assert_eq!(images[1].media_type, "image/png");
    assert_eq!(images[2].media_type, "image/gif");
    assert_eq!(images[3].media_type, "image/webp");
}

#[test]
fn test_load_and_encode_images_skips_missing() {
    let images = load_and_encode_images(&["/nonexistent/path/image.jpg".to_string()]);
    assert!(images.is_empty());
}

#[test]
fn test_load_and_encode_images_skips_unsupported_format() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("test.bmp");
    std::fs::write(&path, b"bmp data").unwrap();

    let images = load_and_encode_images(&[path.to_string_lossy().to_string()]);
    assert!(images.is_empty());
}

#[test]
fn test_load_and_encode_images_rejects_bad_magic_bytes() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Write a .png file with JPEG content
    let path = tmp.path().join("fake.png");
    std::fs::write(&path, JPEG_MAGIC).unwrap();

    let images = load_and_encode_images(&[path.to_string_lossy().to_string()]);
    assert!(images.is_empty(), "should reject mismatched magic bytes");
}

#[test]
fn test_load_and_encode_images_rejects_html_as_image() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Simulate Slack returning HTML instead of image
    let path = tmp.path().join("download.png");
    std::fs::write(&path, b"<html><body>Error</body></html>").unwrap();

    let images = load_and_encode_images(&[path.to_string_lossy().to_string()]);
    assert!(images.is_empty(), "should reject HTML content");
}

#[test]
fn test_load_and_encode_images_max_limit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut paths = Vec::new();
    for i in 0..8 {
        let path = tmp.path().join(format!("img{}.png", i));
        std::fs::write(&path, PNG_MAGIC).unwrap();
        paths.push(path.to_string_lossy().to_string());
    }

    let images = load_and_encode_images(&paths);
    assert_eq!(images.len(), MAX_IMAGES); // Capped at 5
}

#[test]
fn test_load_and_encode_images_empty_input() {
    let images = load_and_encode_images(&[]);
    assert!(images.is_empty());
}

#[test]
fn test_load_and_encode_images_base64_roundtrip() {
    use base64::Engine;
    let tmp = tempfile::TempDir::new().unwrap();
    let img_path = tmp.path().join("test.png");
    // Use valid PNG magic + extra data
    let mut original_data = PNG_MAGIC.to_vec();
    original_data.extend_from_slice(b"extra png data here");
    std::fs::write(&img_path, &original_data).unwrap();

    let images = load_and_encode_images(&[img_path.to_string_lossy().to_string()]);
    assert_eq!(images.len(), 1);

    // Decode and verify roundtrip
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&images[0].data)
        .unwrap();
    assert_eq!(decoded, original_data);
}

#[test]
fn test_load_and_encode_images_pdf_support() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("test.pdf");
    let mut pdf_data = b"%PDF-1.4 ".to_vec();
    pdf_data.extend_from_slice(b"fake pdf content for testing");
    std::fs::write(&path, &pdf_data).unwrap();

    let images = load_and_encode_images(&[path.to_string_lossy().to_string()]);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].media_type, "application/pdf");
}

#[test]
fn test_strip_document_tags() {
    let content = "User sent a PDF\n[document: /home/user/.oxicrab/media/telegram_123.pdf]";
    let stripped = strip_document_tags(content);
    assert_eq!(stripped, "User sent a PDF");
    assert!(!stripped.contains("[document:"));
}

#[test]
fn test_strip_document_tags_preserves_other_content() {
    let content = "text [image: /path/img.jpg] and [document: /path/doc.pdf] more text";
    let stripped = strip_document_tags(content);
    assert!(stripped.contains("[image: /path/img.jpg]"));
    assert!(!stripped.contains("[document:"));
    assert!(stripped.contains("more text"));
}

#[test]
fn test_load_and_encode_images_rejects_fake_pdf() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("fake.pdf");
    // Not a real PDF (wrong magic bytes)
    std::fs::write(&path, b"this is not a pdf").unwrap();

    let images = load_and_encode_images(&[path.to_string_lossy().to_string()]);
    assert!(images.is_empty(), "should reject non-PDF content");
}

// --- handle_text_response tests ---

#[test]
fn test_conversational_reply_passes_through() {
    // Short conversational replies should be returned as-is (not flagged as hallucination)
    let tool_names = vec!["memory_search".to_string(), "cron".to_string()];
    let mut messages = vec![];
    let mut correction_sent = false;

    let cases = [
        "Sure, I'll do that now.",
        "Sounds good!",
        "The first option, please.",
        "Yes",
        "No, let's skip that.",
    ];
    for reply in cases {
        let result = AgentLoop::handle_text_response(
            reply,
            &mut messages,
            None,
            false,
            &mut correction_sent,
            &tool_names,
        );
        assert!(
            matches!(result, TextAction::Return),
            "conversational reply '{}' should pass through",
            reply
        );
    }
}

#[test]
fn test_action_hallucination_caught_without_tool_forcing() {
    // Action claims should be caught by hallucination detection even without tool_choice="any"
    let tool_names = vec!["write_file".to_string()];
    let mut messages = vec![];
    let mut correction_sent = false;

    let result = AgentLoop::handle_text_response(
        "I've updated the configuration file.",
        &mut messages,
        None,
        false,
        &mut correction_sent,
        &tool_names,
    );
    assert!(
        matches!(result, TextAction::Continue),
        "action claim should trigger correction"
    );
    assert!(correction_sent);
}

#[test]
fn test_action_hallucination_repeatable_correction() {
    // After correction_sent is already true, a second action claim should STILL be caught
    let tool_names = vec!["write_file".to_string()];
    let mut messages = vec![];
    let mut correction_sent = true; // already corrected once

    let result = AgentLoop::handle_text_response(
        "I've written the new module.",
        &mut messages,
        None,
        false,
        &mut correction_sent,
        &tool_names,
    );
    assert!(
        matches!(result, TextAction::Continue),
        "repeated action claim should still be corrected"
    );
}

#[test]
fn test_legitimate_tool_response_passes_through() {
    // After tools were actually called, text responses pass through
    let tool_names = vec!["write_file".to_string()];
    let mut messages = vec![];
    let mut correction_sent = false;

    let result = AgentLoop::handle_text_response(
        "I've updated the configuration file.",
        &mut messages,
        None,
        true, // tools were called
        &mut correction_sent,
        &tool_names,
    );
    assert!(
        matches!(result, TextAction::Return),
        "after real tool calls, text should pass through"
    );
}

// --- Multi-iteration hallucination correction tests ---

#[test]
fn test_false_no_tools_claim_always_fires() {
    // false-no-tools correction should fire even after correction_sent is true
    let tool_names = vec!["exec".to_string(), "read_file".to_string()];
    let mut messages = vec![];
    let mut correction_sent = true; // already corrected once

    let result = AgentLoop::handle_text_response(
        "I don't have access to tools to help with that.",
        &mut messages,
        None,
        false,
        &mut correction_sent,
        &tool_names,
    );
    assert!(
        matches!(result, TextAction::Continue),
        "false no-tools claim should always trigger correction"
    );
}

#[test]
fn test_text_after_tools_called_passes_action_claims() {
    // After tools have been called (any_tools_called=true), text claiming actions
    // should pass through since the model actually DID call tools
    let tool_names = vec![
        "exec".to_string(),
        "read_file".to_string(),
        "write_file".to_string(),
    ];
    let mut messages = vec![];
    let mut correction_sent = false;

    let claims = [
        "I've updated the configuration file.",
        "I've created a new module for the project.",
        "Changes have been applied successfully.",
        "I've executed the commands.",
        "All tests passed.",
    ];
    for claim in claims {
        let result = AgentLoop::handle_text_response(
            claim,
            &mut messages,
            None,
            true, // tools WERE called
            &mut correction_sent,
            &tool_names,
        );
        assert!(
            matches!(result, TextAction::Return),
            "claim '{}' should pass through after tools were called",
            claim
        );
        assert!(
            !correction_sent,
            "correction should not be sent after real tool use"
        );
    }
}

#[test]
fn test_empty_tool_names_disables_false_no_tools_check() {
    // When no tools are registered, the false-no-tools check should not fire
    let tool_names: Vec<String> = vec![];
    let mut messages = vec![];
    let mut correction_sent = false;

    let result = AgentLoop::handle_text_response(
        "I don't have access to tools.",
        &mut messages,
        None,
        false,
        &mut correction_sent,
        &tool_names,
    );
    assert!(
        matches!(result, TextAction::Return),
        "no-tools claim should pass through when no tools are registered"
    );
}

#[test]
fn test_mentions_multiple_tools_triggers_correction() {
    // A response listing many tool names (without calling them) should be caught
    let tool_names = vec![
        "web_search".to_string(),
        "weather".to_string(),
        "cron".to_string(),
        "exec".to_string(),
        "read_file".to_string(),
    ];
    let mut messages = vec![];
    let mut correction_sent = false;

    let result = AgentLoop::handle_text_response(
        "## Available Tools\n- web_search: Search the web\n- weather: Get weather\n- cron: Schedule jobs\n- exec: Run commands",
        &mut messages,
        None,
        false,
        &mut correction_sent,
        &tool_names,
    );
    assert!(
        matches!(result, TextAction::Continue),
        "listing multiple tools without calling them should trigger correction"
    );
}

// --- Media cleanup tests ---

#[test]
fn test_cleanup_old_media_no_dir() {
    // Should not error when media dir doesn't exist
    // cleanup_old_media uses home_dir, so we can't easily test with a custom path.
    // Instead, test the no-op case: TTL=0 is never called, and missing dir returns Ok.
    // This is a smoke test that the function doesn't panic.
    let result = cleanup_old_media(9999);
    assert!(result.is_ok());
}

// --- extract_media_paths tests ---

#[test]
fn test_extract_media_paths_json_media_path() {
    // Create a temp file so the path exists
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();
    let json = format!(
        r#"{{"url":"https://example.com/img.png","mediaPath":"{}","mediaSize":1234}}"#,
        path
    );
    let paths = extract_media_paths(&json);
    assert_eq!(paths, vec![path]);
}

#[test]
fn test_extract_media_paths_saved_to_pattern() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();
    let text = format!("Screenshot saved to: {}\nSize: 12345 bytes", path);
    let paths = extract_media_paths(&text);
    assert_eq!(paths, vec![path]);
}

#[test]
fn test_extract_media_paths_nonexistent_path_ignored() {
    let json = r#"{"mediaPath":"/tmp/nonexistent_test_file_12345.png"}"#;
    let paths = extract_media_paths(json);
    assert!(paths.is_empty());
}

#[test]
fn test_extract_media_paths_plain_text_no_match() {
    let paths = extract_media_paths("Just a normal tool result with no media");
    assert!(paths.is_empty());
}

#[test]
fn test_extract_media_paths_deduplicates() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();
    // Both JSON and text pattern point to same file
    let text = format!(r#"{{"mediaPath":"{}"}}"#, path);
    // Only returns once despite being findable via JSON parse
    let paths = extract_media_paths(&text);
    assert_eq!(paths.len(), 1);
}
