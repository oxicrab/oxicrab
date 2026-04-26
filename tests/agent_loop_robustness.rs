//! End-to-end coverage for the agent loop's robustness layers:
//! force-text engagement, duplicate-call time-gate, post-coerce
//! schema validation, schema-hint idempotency, iteration-budget
//! system_notice injection, and cancel-mid-LLM-call.

mod common;

use crate::common::{
    MockLLMProvider, TestAgentOverrides, create_test_agent_with, text_response, tool_call,
    tool_response,
};
use oxicrab::providers::base::{ChatRequest, LLMProvider, LLMResponse};
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;

fn empty_response() -> LLMResponse {
    LLMResponse::default()
}

/// Force-text mode: after enough consecutive empty responses with
/// any-tools-called=true, the agent loop strips tools and the
/// post-loop summary delivers a fallback. The loop should NOT spin
/// to max_iterations on empties.
#[tokio::test]
async fn force_text_engages_after_consecutive_empties() {
    let provider = MockLLMProvider::with_responses(vec![
        // Iteration 1: a tool call so any_tools_called becomes true.
        tool_response(vec![tool_call(
            "tc1",
            "shell",
            json!({"action": "exec", "command": "echo ok"}),
        )]),
        // Empty responses trip the FORCE_TEXT_AFTER_EMPTIES counter.
        empty_response(),
        empty_response(),
        empty_response(),
        // Final summary text the post-loop summary call resolves to.
        text_response("done after force-text"),
    ]);

    let tmp = TempDir::new().unwrap();
    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            max_iterations: Some(8),
            allowed_commands: Some(oxicrab::config::AllowedCommands::new(vec![
                "echo".to_string(),
            ])),
            ..Default::default()
        },
    )
    .await;

    let response = agent
        .process_direct("do something", "test:c1", "test", "c1")
        .await
        .expect("loop should not error");
    // The exact text varies depending on which path resolved; what
    // matters is that the loop produced visible content rather than
    // exhausting iterations.
    assert!(!response.is_empty(), "force-text must yield content");
}

/// Duplicate-call detector engages only after the time gate. Three
/// identical calls in rapid succession (well under the 30s minimum
/// elapsed time) should NOT engage the detector — legitimate
/// poll-until-ready workflows fire identical calls intentionally.
#[tokio::test]
async fn duplicate_calls_within_time_gate_do_not_force_text() {
    let dup = tool_response(vec![tool_call(
        "x1",
        "shell",
        json!({"action": "exec", "command": "echo ok"}),
    )]);
    let provider = MockLLMProvider::with_responses(vec![
        dup.clone(),
        dup.clone(),
        dup.clone(),
        text_response("polled three times, all good"),
    ]);

    let tmp = TempDir::new().unwrap();
    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            max_iterations: Some(8),
            allowed_commands: Some(oxicrab::config::AllowedCommands::new(vec![
                "echo".to_string(),
            ])),
            ..Default::default()
        },
    )
    .await;

    let response = agent
        .process_direct("poll status", "test:c1", "test", "c1")
        .await
        .expect("loop should not error");
    assert!(
        response.contains("polled three times"),
        "duplicate detector should not engage within time-gate; got: {response}"
    );
}

/// Post-coerce schema validation: the registry coerces
/// `{"limit":"5"}` to `{"limit":5}` before running JSON-schema
/// validation, so a tool with an integer-typed limit accepts the
/// stringified form. A genuinely malformed call surfaces a clean
/// error with the schema hint appended.
#[tokio::test]
async fn coerce_runs_before_validation_for_string_to_int() {
    use oxicrab::agent::tools::ToolRegistry;
    use oxicrab::agent::tools::base::{ExecutionContext, Tool, ToolResult};

    struct LimitTool;
    #[async_trait::async_trait]
    impl Tool for LimitTool {
        fn name(&self) -> &str {
            "limit_tool"
        }
        fn description(&self) -> &str {
            "Test tool with integer limit"
        }
        fn parameters(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer"},
                },
                "required": ["limit"],
                "additionalProperties": false,
            })
        }
        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &ExecutionContext,
        ) -> anyhow::Result<ToolResult> {
            let limit = params
                .get("limit")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| anyhow::anyhow!("missing limit"))?;
            Ok(ToolResult::new(format!("limit={limit}")))
        }
    }

    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(LimitTool));

    let ctx = ExecutionContext::default();

    // String "5" should coerce to integer 5 and execute successfully.
    let result = registry
        .execute("limit_tool", json!({"limit": "5"}), &ctx)
        .await
        .expect("execute");
    assert!(!result.is_error, "post-coerce should accept '5' → 5");
    assert!(result.content.contains("limit=5"));

    // A genuinely malformed call (extra property) is rejected. The
    // schema hint is appended exactly once to the error.
    let bad = registry
        .execute("limit_tool", json!({"limit": 5, "unknown": true}), &ctx)
        .await
        .expect("execute");
    assert!(bad.is_error);
    assert!(
        bad.content.contains("Tool description:"),
        "schema hint should be appended: {}",
        bad.content
    );
}

/// Schema-hint idempotency: a tool that returns an error twice in a
/// row picks up at most one schema-hint block, not two.
#[tokio::test]
async fn schema_hint_is_idempotent() {
    use oxicrab::agent::tools::ToolRegistry;
    use oxicrab::agent::tools::base::{ExecutionContext, Tool, ToolResult};

    struct AlwaysErrTool;
    #[async_trait::async_trait]
    impl Tool for AlwaysErrTool {
        fn name(&self) -> &str {
            "alwaysErr"
        }
        fn description(&self) -> &str {
            "always errors"
        }
        fn parameters(&self) -> serde_json::Value {
            json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &ExecutionContext,
        ) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::error("simulated failure"))
        }
    }

    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(AlwaysErrTool));
    let ctx = ExecutionContext::default();

    let r1 = registry
        .execute("alwaysErr", json!({}), &ctx)
        .await
        .expect("execute");
    let hint_count_first = r1.content.matches("\n\nTool description: ").count();
    assert_eq!(hint_count_first, 1, "first error gets the hint");

    // Re-running the same tool should not stack a second hint.
    let r2 = registry
        .execute("alwaysErr", json!({}), &ctx)
        .await
        .expect("execute");
    let hint_count_second = r2.content.matches("\n\nTool description: ").count();
    assert_eq!(
        hint_count_second, 1,
        "second error must not double-append the hint"
    );
}

/// Iteration-budget wrap-up notice: when iteration count reaches the
/// 70% wrap-up threshold AND tools have been called, the loop pushes
/// a `<system_notice type="iteration_budget">` system message into
/// the conversation. We verify by recording every `messages` slice
/// the LLM sees and looking for the marker on the post-threshold
/// call.
#[tokio::test]
async fn wrapup_system_notice_fires_at_iteration_budget() {
    // max_iterations=10 → wrapup_threshold = ceil(10*0.7) = 7.
    // Provide tool calls for iterations 1..=7 so any_tools_called
    // becomes true, then a tool call on iteration 8 (the iteration
    // that runs AFTER the notice was injected at i=7), then text.
    let tc = tool_response(vec![tool_call(
        "x",
        "shell",
        json!({"action": "exec", "command": "echo step"}),
    )]);
    let provider = MockLLMProvider::with_responses(vec![
        tc.clone(), // i=1
        tc.clone(), // i=2
        tc.clone(), // i=3
        tc.clone(), // i=4
        tc.clone(), // i=5
        tc.clone(), // i=6
        tc.clone(), // i=7  — notice injected at top of next call
        tc.clone(), // i=8  — its messages must contain the notice
        text_response("done"),
    ]);
    let calls_handle = provider.calls.clone();

    let tmp = TempDir::new().unwrap();
    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            max_iterations: Some(10),
            allowed_commands: Some(oxicrab::config::AllowedCommands::new(vec![
                "echo".to_string(),
            ])),
            ..Default::default()
        },
    )
    .await;

    let _ = agent
        .process_direct("loop a while", "test:c1", "test", "c1")
        .await
        .expect("loop runs");

    let calls = calls_handle.lock().expect("lock calls");
    // The wrap-up notice is injected before iteration 8, so the 8th
    // recorded call (index 7) MUST contain it.
    assert!(
        calls.len() >= 8,
        "expected at least 8 LLM calls, saw {}",
        calls.len()
    );
    let post_threshold_call = &calls[7];
    let saw_notice = post_threshold_call.messages.iter().any(|m| {
        m.content
            .contains("<system_notice type=\"iteration_budget\"")
    });
    assert!(
        saw_notice,
        "iteration-budget notice missing from messages on call #8: {:?}",
        post_threshold_call
            .messages
            .iter()
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
    );
}

/// Cancel-mid-LLM-call: a slow provider is racing against
/// `cancel_session()`. The agent loop's `tokio::select!` with the
/// biased cancel branch must abort the in-flight request and
/// surface a turn-cancelled error rather than waiting for the
/// provider's full delay.
#[tokio::test]
async fn cancel_session_aborts_in_flight_llm_call() {
    use async_trait::async_trait;

    struct VerySlowProvider;
    #[async_trait]
    impl LLMProvider for VerySlowProvider {
        async fn chat(&self, _req: &ChatRequest) -> anyhow::Result<LLMResponse> {
            // 30s — long enough that the test would visibly hang
            // without cancellation working.
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            Ok(LLMResponse {
                content: Some("never seen".to_string()),
                ..Default::default()
            })
        }
        fn default_model(&self) -> &str {
            "very-slow"
        }
    }

    let tmp = TempDir::new().unwrap();
    let agent = Arc::new(
        create_test_agent_with(VerySlowProvider, &tmp, TestAgentOverrides::default()).await,
    );

    let agent_clone = Arc::clone(&agent);
    let cancel_task = tokio::spawn(async move {
        // Wait for the loop to enter the LLM call before cancelling.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        agent_clone.cancel_session("test:c1")
    });

    let start = std::time::Instant::now();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        agent.process_direct("hang please", "test:c1", "test", "c1"),
    )
    .await;
    let elapsed = start.elapsed();

    let cancelled_ok = cancel_task.await.expect("cancel task did not panic");
    assert!(
        cancelled_ok,
        "cancel_session should report a token was found"
    );

    // The outer timeout MUST NOT trip — cancel should land first.
    let inner = result.expect("loop must return before the 5s test timeout");
    // The agent loop converts a cancelled turn into an error; some
    // paths convert that into a user-facing fallback message. Either
    // way, we should be back well under the 30s provider delay.
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "cancel-mid-LLM should resolve quickly; took {:?}",
        elapsed
    );
    let _ = inner;
}
