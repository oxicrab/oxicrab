use super::*;
use regex::Regex;

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
        "Updated: config.toml with the new API key.",
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
        // Present progressive claims: "I'm creating...", "I am creating..."
        "Creating 4 calendar events now...",
        "I'm creating the events for you",
        "Setting up the calendar entries...",
        // Intent statements: "Let me create...", "I'll create..."
        "Let me create those events",
        "I'll add them to your calendar",
        "Going to schedule the meetings",
        // New verbs: retrieve, process, get, show, etc.
        "Let me retrieve the next available article",
        "I'll process this tool call now",
        "I've retrieved the latest data from the feed",
        "I retrieved the article for you",
        "I've processed the request successfully",
        "I processed the queue entries",
        "I'm retrieving the articles now",
        "I'm processing your request",
        "Retrieving the next batch of articles...",
        "Processing your request now...",
        "Successfully retrieved the data",
        "Successfully processed the queue",
        "Retrieved: 5 new articles from the feed",
        "Processed: all pending items",
        "Let me get the next article",
        "Let me show you the results",
        "I've generated the report",
        "I've submitted the form",
        "I've downloaded the attachment",
        "I've uploaded the file",
        "I've reviewed the changes",
        "I've scanned the inbox",
        "I've organized the workspace",
        "About to pull the latest data",
        "Going to push the changes now",
    ];
    for text in cases {
        assert!(contains_action_claims(text), "should match: {text}");
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
        // Present progressive / intent in non-claim context
        "Would you like me to start creating the events?",
        "I can help with creating events",
        "The process of creating events takes time",
    ];
    for text in cases {
        assert!(!contains_action_claims(text), "should NOT match: {text}");
    }
}

#[test]
fn test_action_claim_pattern_fragments_each_match() {
    // Each ACTION_CLAIM_PATTERNS fragment should match at least one representative string
    let representatives = [
        "I've updated the configuration file.", // FIRST_PERSON_PERFECT
        "I wrote the function as requested.",   // FIRST_PERSON_PAST
        "Changes have been applied to the project.", // PASSIVE_CHANGES
        "File has been updated successfully.",  // PASSIVE_ENTITY
        "All tests passed successfully.",       // STATUS_ALL
        "Successfully executed the command.",   // ADVERB_PAST
        "Created: a new issue in the tracker.", // TERSE_LINE_START
        "I'm creating the events for you",      // PRESENT_PROGRESSIVE
        "Creating 4 calendar events now...",    // GERUND_LINE_START
        "Let me create those events",           // INTENT_STATEMENT
    ];
    assert_eq!(
        representatives.len(),
        ACTION_CLAIM_PATTERNS.len(),
        "each pattern fragment should have a representative test case"
    );
    for (i, text) in representatives.iter().enumerate() {
        let pattern = ACTION_CLAIM_PATTERNS[i];
        let re = Regex::new(&format!("(?i){pattern}"))
            .unwrap_or_else(|_| panic!("fragment {i} is invalid regex"));
        assert!(re.is_match(text), "fragment {i} should match: {text}");
    }
}

#[test]
fn test_silent_response_prefix() {
    assert!("[SILENT] Internal note recorded.".starts_with("[SILENT]"));
    assert!(!"[silent] This should pass through.".starts_with("[SILENT]"));
    assert!(!"Here is a normal response.".starts_with("[SILENT]"));
}

// --- Parallel tool execution tests ---

use crate::agent::tools::base::{Tool, ToolResult};
use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse, Message, ToolCallRequest};
use async_trait::async_trait;
use std::collections::{HashMap, VecDeque};
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

fn make_tool_call_with_args(id: &str, name: &str, arguments: serde_json::Value) -> ToolCallRequest {
    ToolCallRequest {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
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

struct QueuedProvider {
    responses: std::sync::Mutex<VecDeque<LLMResponse>>,
}

impl QueuedProvider {
    fn new(responses: Vec<LLMResponse>) -> Self {
        Self {
            responses: std::sync::Mutex::new(VecDeque::from(responses)),
        }
    }
}

#[async_trait]
impl LLMProvider for QueuedProvider {
    async fn chat(&self, _req: &ChatRequest) -> anyhow::Result<LLMResponse> {
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default())
    }

    fn default_model(&self) -> &'static str {
        "mock-model"
    }
}

struct DeferredTool;

#[async_trait]
impl Tool for DeferredTool {
    fn name(&self) -> &'static str {
        "deferred_tool"
    }

    fn description(&self) -> &'static str {
        "Deferred test tool"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &ExecutionContext,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::new("deferred ok"))
    }
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
                    None,
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
    assert_eq!(results[0].content, "slow_result");
    assert_eq!(results[1].content, "fast_result");
    assert_eq!(results[2].content, "medium_result");
    assert!(!results[0].is_error);
    assert!(!results[1].is_error);
    assert!(!results[2].is_error);
}

#[tokio::test]
async fn test_guided_turn_tool_search_can_activate_deferred_tool() {
    let tmp = tempfile::tempdir().unwrap();
    let bus = Arc::new(crate::bus::MessageBus::default());
    let (outbound_tx, _outbound_rx) = tokio::sync::mpsc::channel(4);
    let provider = Arc::new(QueuedProvider::new(vec![
        LLMResponse {
            tool_calls: vec![make_tool_call_with_args(
                "1",
                "tool_search",
                serde_json::json!({"query": "deferred"}),
            )],
            ..Default::default()
        },
        LLMResponse {
            tool_calls: vec![make_tool_call_with_args(
                "2",
                "deferred_tool",
                serde_json::json!({}),
            )],
            ..Default::default()
        },
        LLMResponse {
            content: Some("done".to_string()),
            ..Default::default()
        },
    ]));

    let mut agent = AgentLoop::new(AgentLoopConfig::test_defaults(
        bus.clone(),
        provider,
        tmp.path().to_path_buf(),
        Arc::new(outbound_tx),
    ))
    .await
    .unwrap();

    let activated = crate::agent::tools::tool_search::ActivatedTools::new();
    let mut registry = ToolRegistry::new();
    registry.register_deferred(Arc::new(DeferredTool));
    registry.register(Arc::new(
        crate::agent::tools::tool_search::ToolSearchTool::new(
            vec![crate::agent::tools::tool_search::ToolIndexEntry {
                name: "deferred_tool".to_string(),
                description: "Deferred test tool".to_string(),
                deferred: true,
            }],
            activated.clone(),
        ),
    ));
    agent.tools = Arc::new(registry);
    agent.tool_search_activated = activated;

    let result = agent
        .run_agent_loop_with_overrides(
            vec![Message::system("system".to_string())],
            None,
            &ExecutionContext {
                channel: "test".to_string(),
                chat_id: "chat".to_string(),
                context_summary: None,
                metadata: HashMap::from([(
                    "request_id".to_string(),
                    serde_json::Value::String("req-guided".to_string()),
                )]),
            },
            &AgentRunOverrides {
                request_id: Some("req-guided".to_string()),
                routing_policy: Some(crate::router::RoutingPolicy {
                    allowed_tools: vec!["tool_search".to_string()],
                    blocked_tools: vec!["deferred_tool".to_string()],
                    context_hint: None,
                    reason: "guided",
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(result.content.as_deref(), Some("done"));
    assert!(result.tools_used.iter().any(|t| t == "tool_search"));
    assert!(result.tools_used.iter().any(|t| t == "deferred_tool"));
}

#[tokio::test]
async fn test_single_tool_no_parallel_overhead() {
    let registry = make_registry_with(vec![Arc::new(MockTool {
        tool_name: "only".into(),
        delay_ms: 0,
        response: "only_result".into(),
    })]);

    let result = execute_tool_call(
        &registry,
        "only",
        &serde_json::json!({}),
        &empty_tools(),
        &ExecutionContext::default(),
        None,
        None,
    )
    .await;

    assert_eq!(result.content, "only_result");
    assert!(!result.is_error);
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
                    None,
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
            Err(_) => ToolResult::error("Tool crashed unexpectedly"),
        })
        .collect();

    // Good tools succeed
    assert_eq!(results[0].content, "result1");
    assert!(!results[0].is_error);
    assert_eq!(results[2].content, "result2");
    assert!(!results[2].is_error);
    // Panicked tool gets error
    assert!(results[1].content.contains("crashed"));
    assert!(results[1].is_error);
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
                    None,
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
    assert_eq!(results[0].content, "good_result");
    assert!(!results[0].is_error);
    assert_eq!(results[2].content, "also_good_result");
    assert!(!results[2].is_error);
    // Error tool marked as error
    assert!(results[1].content.contains("Tool execution failed"));
    assert!(results[1].is_error);
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
    let result = execute_tool_call(
        &registry,
        "nonexistent_tool",
        &serde_json::json!({}),
        &available,
        &ExecutionContext::default(),
        None,
        None,
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("does not exist"));
    assert!(result.content.contains("read_file"));
    assert!(result.content.contains("write_file"));
    assert!(result.content.contains("exec"));
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

struct StrictSchemaTestTool;

#[async_trait]
impl Tool for StrictSchemaTestTool {
    fn name(&self) -> &'static str {
        "strict_schema_test"
    }
    fn description(&self) -> &'static str {
        "strict schema test tool"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"],
            "additionalProperties": false
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
    assert!(msg.contains("query"));
    assert!(msg.contains("required"));
}

#[test]
fn test_validate_params_wrong_type() {
    let tool = SchemaTestTool;
    // query should be string, but we pass a number
    let params = serde_json::json!({"query": 42});
    let result = validate_tool_params(&tool, &params);
    assert!(result.is_some());
    let msg = result.unwrap();
    assert!(msg.contains("query"));
    assert!(msg.contains("type"));
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

#[test]
fn test_validate_params_rejects_unknown_when_additional_properties_false() {
    let tool = StrictSchemaTestTool;
    let params = serde_json::json!({"query": "ok", "extra": 1});
    let result = validate_tool_params(&tool, &params);
    assert!(result.is_some());
    let msg = result.unwrap();
    assert!(msg.contains("unknown parameter"));
    assert!(msg.contains("extra"));
}

#[tokio::test]
async fn test_validation_rejects_before_execution() {
    // Tool with required param "query" — call without it, should get validation error
    let registry = make_registry_with(vec![Arc::new(SchemaTestTool)]);
    let result = execute_tool_call(
        &registry,
        "schema_test",
        &serde_json::json!({}),
        &empty_tools(),
        &ExecutionContext::default(),
        None,
        None,
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("query"));
    assert!(result.content.contains("required"));
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
    let result = execute_tool_call(
        &registry,
        "untrusted_mcp_tool",
        &serde_json::json!({}),
        &empty_tools(),
        &ExecutionContext::default(),
        None,
        None,
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("requires approval"));
}

#[tokio::test]
async fn test_normal_tool_not_blocked_by_approval() {
    let registry = make_registry_with(vec![Arc::new(MockTool {
        tool_name: "safe_tool".into(),
        delay_ms: 0,
        response: "safe_result".into(),
    })]);
    let result = execute_tool_call(
        &registry,
        "safe_tool",
        &serde_json::json!({}),
        &empty_tools(),
        &ExecutionContext::default(),
        None,
        None,
    )
    .await;
    assert!(!result.is_error);
    assert_eq!(result.content, "safe_result");
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
                .join(format!("test.{ext}"))
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
        let path = tmp.path().join(format!("img{i}.png"));
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
    let mut layer1_fired = false;

    let cases = [
        "Sure, I'll do that now.",
        "Sounds good!",
        "The first option, please.",
        "Yes",
        "No, let's skip that.",
    ];
    for reply in cases {
        let result = hallucination::handle_text_response(
            reply,
            &mut messages,
            false,
            &mut layer1_fired,
            &tool_names,
        );
        assert!(
            matches!(result, TextAction::Return),
            "conversational reply '{reply}' should pass through"
        );
    }
}

#[test]
fn test_action_hallucination_caught_without_tool_forcing() {
    // Action claims should be caught by hallucination detection (tool_choice is always auto)
    let tool_names = vec!["write_file".to_string()];
    let mut messages = vec![];
    let mut layer1_fired = false;

    let result = hallucination::handle_text_response(
        "I've updated the configuration file.",
        &mut messages,
        false,
        &mut layer1_fired,
        &tool_names,
    );
    assert!(
        matches!(result, TextAction::Continue),
        "action claim should trigger correction"
    );
    assert!(layer1_fired);
}

#[test]
fn test_action_hallucination_not_repeated_after_l1_correction() {
    // After layer1_fired, a second action claim should pass through (single retry exhausted)
    let tool_names = vec!["write_file".to_string()];
    let mut messages = vec![];
    let mut layer1_fired = true; // L1 already corrected

    let result = hallucination::handle_text_response(
        "I've written the new module.",
        &mut messages,
        false,
        &mut layer1_fired,
        &tool_names,
    );
    assert!(
        matches!(result, TextAction::Return),
        "after L1 correction, text should pass through"
    );
}

#[test]
fn test_legitimate_tool_response_passes_through() {
    // After tools were actually called, action claims pass through (not a hallucination)
    let tool_names = vec!["write_file".to_string()];
    let mut messages = vec![];
    let mut layer1_fired = false;

    let result = hallucination::handle_text_response(
        "I've updated the configuration file.",
        &mut messages,
        true, // tools were called
        &mut layer1_fired,
        &tool_names,
    );
    assert!(
        matches!(result, TextAction::Return),
        "after real tool calls, text should pass through"
    );
}

#[test]
fn test_text_after_tools_called_passes_action_claims() {
    // After tools have been called (any_tools_called=true), text claiming actions
    // should pass through since the model actually DID call the tools
    let tool_names = vec![
        "exec".to_string(),
        "read_file".to_string(),
        "write_file".to_string(),
    ];
    let mut messages = vec![];
    let mut layer1_fired = false;

    let claims = [
        "I've updated the configuration file.",
        "I've created a new module for the project.",
        "Changes have been applied successfully.",
        "I've executed the commands.",
        "All tests passed.",
    ];
    for claim in claims {
        let result = hallucination::handle_text_response(
            claim,
            &mut messages,
            true, // tools WERE called
            &mut layer1_fired,
            &tool_names,
        );
        assert!(
            matches!(result, TextAction::Return),
            "claim '{claim}' should pass through after tools were called"
        );
        assert!(
            !layer1_fired,
            "correction should not be sent after real tool use"
        );
    }
}

#[test]
fn test_empty_tool_names_disables_hallucination_check() {
    // When no tools are registered, hallucination detection should not fire
    let tool_names: Vec<String> = vec![];
    let mut messages = vec![];
    let mut layer1_fired = false;

    let result = hallucination::handle_text_response(
        "I've updated the configuration file.",
        &mut messages,
        false,
        &mut layer1_fired,
        &tool_names,
    );
    assert!(
        matches!(result, TextAction::Return),
        "action claim should pass through when no tools are registered"
    );
    assert!(!layer1_fired);
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
// These tests create files inside the media directory because
// extract_media_paths only accepts paths within ~/.oxicrab/media/.

fn create_media_test_file() -> (String, impl Drop) {
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    let media_dir = crate::utils::media::media_dir().unwrap();
    let name = format!("test_{}.tmp", fastrand::u32(..));
    let path = media_dir.join(&name);
    std::fs::write(&path, b"test").unwrap();
    let path_str = path.to_string_lossy().to_string();
    (path_str, Cleanup(path))
}

#[test]
fn test_extract_media_paths_json_media_path() {
    let (path, _guard) = create_media_test_file();
    let json =
        format!(r#"{{"url":"https://example.com/img.png","mediaPath":"{path}","mediaSize":1234}}"#);
    let paths = extract_media_paths(&json);
    assert_eq!(paths, vec![path]);
}

#[test]
fn test_extract_media_paths_saved_to_pattern() {
    let (path, _guard) = create_media_test_file();
    let text = format!("Screenshot saved to: {path}\nSize: 12345 bytes");
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
fn test_extract_media_paths_outside_media_dir_rejected() {
    // File exists but is outside media dir — should be rejected
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();
    let json = format!(r#"{{"mediaPath":"{path}"}}"#);
    let paths = extract_media_paths(&json);
    assert!(
        paths.is_empty(),
        "paths outside media dir should be rejected"
    );
}

#[test]
fn test_extract_media_paths_plain_text_no_match() {
    let paths = extract_media_paths("Just a normal tool result with no media");
    assert!(paths.is_empty());
}

#[test]
fn test_extract_media_paths_deduplicates() {
    let (path, _guard) = create_media_test_file();
    // Both JSON and text pattern point to same file
    let text = format!(r#"{{"mediaPath":"{path}"}}"#);
    let paths = extract_media_paths(&text);
    assert_eq!(paths.len(), 1);
}

// --- strip_think_tags tests ---

#[test]
fn test_strip_think_tags_closed() {
    let input = "<think>some reasoning</think>Here is the answer.";
    assert_eq!(strip_think_tags(input), "Here is the answer.");
}

#[test]
fn test_strip_think_tags_multiline() {
    let input = "<think>\nLet me think about this.\nOK I know.\n</think>\nHere is the answer.";
    assert_eq!(strip_think_tags(input), "Here is the answer.");
}

#[test]
fn test_strip_think_tags_unclosed() {
    let input = "<think>reasoning without end tag\nstill reasoning\nAnswer is here.";
    assert_eq!(strip_think_tags(input), "");
}

#[test]
fn test_strip_think_tags_unclosed_with_prefix() {
    let input = "Some preamble.\n<think>reasoning without end tag";
    assert_eq!(strip_think_tags(input), "Some preamble.");
}

#[test]
fn test_strip_think_tags_no_tags() {
    let input = "Just a normal response.";
    assert_eq!(strip_think_tags(input), "Just a normal response.");
}

#[test]
fn test_strip_think_tags_multiple_blocks() {
    let input = "<think>block1</think>middle<think>block2</think>end";
    assert_eq!(strip_think_tags(input), "middleend");
}

#[test]
fn test_strip_think_tags_closed_then_unclosed() {
    let input = "<think>block1</think>middle text<think>unclosed block";
    assert_eq!(strip_think_tags(input), "middle text");
}

#[test]
fn test_strip_think_tags_empty_block() {
    let input = "<think></think>content after";
    assert_eq!(strip_think_tags(input), "content after");
}
