use super::config::{AgentLoopResult, AgentRunOverrides};
use super::hallucination::{self, TextAction};
use super::{
    AgentLoop, EMPTY_RESPONSE_RETRIES, MAX_RETRY_DELAY_SECS, MIN_WRAPUP_ITERATION,
    RETRY_BACKOFF_BASE, WRAPUP_THRESHOLD_RATIO,
};
use crate::agent::cognitive::CheckpointTracker;
use crate::agent::context::ContextBuilder;
use crate::providers::base::{LLMProvider, Message, ToolCallRequest};

use super::helpers::{execute_tool_call, extract_media_paths, start_typing, strip_think_tags};
use super::metadata::{extract_display_text, merge_suggested_buttons, prepend_display_text};
use crate::agent::tools::base::{ExecutionContext, ToolResult};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, warn};

const SESSION_KEY_META_KEY: &str = "session_key";

impl AgentLoop {
    /// Core agent loop implementation with per-invocation overrides.
    ///
    /// Iterates up to `max_iterations` rounds of: LLM call → parallel tool execution → append results.
    /// Uses `tool_choice=None` (auto) on all iterations — hallucination detection in
    /// `handle_text_response()` catches false action claims. At 70% of max iterations, a wrap-up
    /// nudge is injected.
    ///
    /// Returns an `AgentLoopResult` with response text, input tokens, tool names used, and media paths.
    pub(super) async fn run_agent_loop_with_overrides(
        &self,
        mut messages: Vec<Message>,
        typing_context: Option<(String, String)>,
        exec_ctx: &ExecutionContext,
        overrides: &AgentRunOverrides,
    ) -> Result<AgentLoopResult> {
        let effective_model = overrides.model.as_deref().unwrap_or(&self.model);
        let effective_provider = overrides.provider.as_ref().unwrap_or(&self.provider);
        let effective_max_iterations = overrides.max_iterations.unwrap_or(self.max_iterations);
        let activation_scope = overrides
            .request_id
            .clone()
            .unwrap_or_else(|| format!("run-{}", fastrand::u64(..)));
        let mut empty_retries_left = EMPTY_RESPONSE_RETRIES;
        let mut any_tools_called = false;
        let mut layer1_fired = false;
        let mut last_input_tokens: Option<u64> = None;
        let mut tools_used: Vec<String> = Vec::new();
        let mut collected_media: Vec<String> = Vec::new();
        let mut collected_tool_metadata: Vec<(String, HashMap<String, serde_json::Value>)> =
            Vec::new();
        let mut checkpoint_tracker = CheckpointTracker::new(self.cognitive_config.clone());

        // Clear request-scoped deferred tool activations from previous retries/reuse.
        self.tool_search_activated.clear(&activation_scope).await;
        self.pending_buttons.clear(&activation_scope);
        let result = async {
            let mut activated_snapshot = std::collections::HashSet::new();

            let tools_defs = self
                .tools
                .get_tool_definitions_with_activated(&activated_snapshot);

            // Exfiltration guard: hide network-outbound tools from the LLM
            let mut tools_defs = if self.exfiltration_guard.enabled {
                let allowed = &self.exfiltration_guard.allow_tools;
                tools_defs
                    .into_iter()
                    .filter(|td| {
                        let is_network = self
                            .tools
                            .get(&td.name)
                            .is_some_and(|t| t.capabilities().network_outbound);
                        !is_network || allowed.contains(&td.name)
                    })
                    .collect()
            } else {
                tools_defs
            };

            // Router tool filter: constrain available tools for GuidedLLM/SemanticFilter paths
            if let Some(policy) = overrides.routing_policy.as_ref() {
                tools_defs.retain(|td| {
                    policy.allowed_tools.contains(&td.name)
                        || activated_snapshot.contains(&td.name)
                        || td.name == "add_buttons"
                        || td.name == "tool_search"
                });
            }
            if let Some(policy) = overrides.routing_policy.as_ref() {
                debug!(
                    "router policy active: reason={} tools={}",
                    policy.reason,
                    tools_defs.len()
                );
            }

            // Extract tool names for hallucination detection (may be rebuilt if tool_search activates deferred tools)
            let mut tool_names: Vec<String> = tools_defs.iter().map(|td| td.name.clone()).collect();

            // Wrap tools in Arc for cheap cloning into each ChatRequest iteration
            let mut tools_arc = Arc::new(tools_defs);

            // Reinforce tool awareness in the system prompt. Without this, models
            // (especially via proxy APIs like OpenRouter) sometimes claim tools are
            // unavailable and fabricate responses instead of calling them.
            if !tool_names.is_empty()
                && let Some(system_msg) = messages.first_mut()
            {
                system_msg.content.push_str(
                    "\n\nYou have tools available. If a user asks you to perform actions, \
                     call the matching tool directly — do not claim tools are unavailable.",
                );
            }

            // Inject router context hint into system prompt for GuidedLLM path
            if let Some(hint) = overrides
                .routing_policy
                .as_ref()
                .and_then(|p| p.context_hint.as_ref())
                && let Some(system_msg) = messages.first_mut()
            {
                use std::fmt::Write;
                // Cap context hint to prevent excessive token usage
                let capped = if hint.len() > 1000 {
                    &hint[..hint.floor_char_boundary(1000)]
                } else {
                    hint.as_str()
                };
                let _ = write!(system_msg.content, "\n\n## Active Interaction\n\n{capped}");
            }

            // Append cognitive routines to system prompt when enabled
            if self.cognitive_config.enabled
                && let Some(system_msg) = messages.first_mut()
            {
                system_msg.content.push_str(
                    "\n\n## Cognitive Routines\n\n\
                     When working on complex tasks with many tool calls:\n\
                     - Periodically summarize your progress in your responses\n\
                     - If you receive a checkpoint hint, briefly note: what's done, \
                     what's in progress, what's next\n\
                     - Keep track of your overall plan and remaining steps",
                );
            }

            let wrapup_threshold =
                (effective_max_iterations as f64 * WRAPUP_THRESHOLD_RATIO).ceil() as usize;
            // Ensure wrapup doesn't fire on the very first iteration
            let wrapup_threshold = wrapup_threshold.max(MIN_WRAPUP_ITERATION);
            // Ensure at least 1 iteration remains after wrapup for the LLM to act on it
            let wrapup_threshold = if wrapup_threshold >= effective_max_iterations {
                effective_max_iterations.saturating_sub(1).max(1)
            } else {
                wrapup_threshold
            };

            for iteration in 1..=effective_max_iterations {
            // Inject wrap-up hint when approaching iteration limit
            if iteration == wrapup_threshold && any_tools_called {
                messages.push(Message::system(format!(
                    "You have used {iteration} of {effective_max_iterations} iterations. Begin wrapping up — summarize progress and deliver results."
                )));
            }

            // Start periodic typing indicator before LLM call
            let typing_guard = start_typing(self.typing_tx.as_ref(), typing_context.as_ref());

            // Temperature strategy: use low temperature after any tool calls for
            // deterministic tool sequences, normal temperature before the first tool
            // call (initial response). The post-loop summary uses self.temperature
            // separately, so the final user-facing text always gets normal temperature.
            let current_temp = if any_tools_called {
                self.tool_temperature
            } else {
                self.temperature
            };
            // Let the model decide when to use tools (auto mode). Hallucination detection
            // in handle_text_response() catches false action claims as a safety net.
            let tool_choice: Option<String> = None;

            // Clone needed: messages is mutated after the call (tool results appended),
            // and ChatRequest takes ownership. Cost is negligible vs. the API round-trip.
            let response = super::model_gateway::ModelGateway::invoke(
                effective_provider.as_ref(),
                super::model_gateway::ModelGateway::build_turn_request(
                    messages.clone(),
                    Arc::clone(&tools_arc),
                    effective_model,
                    self.max_tokens,
                    current_temp,
                    tool_choice,
                    overrides.response_format.clone(),
                ),
            )
            .await;

            // Stop typing indicator after LLM call returns (guard aborts on drop)
            drop(typing_guard);

            let response = response?;

            // Track provider-reported input token count for precise compaction decisions
            if response.input_tokens.is_some() {
                last_input_tokens = response.input_tokens;
            }

            // Record token usage off the async runtime (fire-and-forget)
            let cost_model = response.actual_model.as_deref().unwrap_or(effective_model);
            {
                let db = self.memory.db();
                let model = cost_model.to_string();
                let input = response.input_tokens.unwrap_or(0);
                let output = response.output_tokens.unwrap_or(0);
                let cache_create = response.cache_creation_input_tokens.unwrap_or(0);
                let cache_read = response.cache_read_input_tokens.unwrap_or(0);
                let req_id = overrides.request_id.clone();
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = db.record_tokens(
                        &model,
                        input,
                        output,
                        cache_create,
                        cache_read,
                        "main",
                        req_id.as_deref(),
                    ) {
                        warn!("failed to record token usage: {}", e);
                    }
                });
            }

            if response.has_tool_calls() {
                any_tools_called = true;
                tools_used.extend(response.tool_calls.iter().map(|tc| tc.name.clone()));
                ContextBuilder::add_assistant_message(
                    &mut messages,
                    response.content.as_deref(),
                    Some(response.tool_calls.clone()),
                    response.reasoning_content.as_deref(),
                    response.reasoning_signature.as_deref(),
                    response.redacted_thinking_blocks.clone(),
                );

                // Start periodic typing indicator before tool execution
                let typing_guard = start_typing(self.typing_tx.as_ref(), typing_context.as_ref());

                let exfil_ref = if self.exfiltration_guard.enabled {
                    Some(&self.exfiltration_guard)
                } else {
                    None
                };
                let results = self
                    .execute_tools(
                        &response.tool_calls,
                        &tool_names,
                        exec_ctx,
                        exfil_ref,
                        overrides.routing_policy.as_ref(),
                    )
                    .await;

                // Stop typing indicator after tool execution (guard aborts on drop)
                drop(typing_guard);

                self.handle_tool_results(
                    &mut messages,
                    &response.tool_calls,
                    results,
                    &mut collected_media,
                    &mut collected_tool_metadata,
                    &mut checkpoint_tracker,
                    exec_ctx,
                )
                .await;

                // If tool_search activated new deferred tools, rebuild tool
                // definitions so the LLM sees their schemas in the next iteration.
                if self.tools.deferred_count() > 0 {
                    let current = self.tool_search_activated.snapshot(&activation_scope).await;
                    if current.len() > activated_snapshot.len() {
                        let new_count = current.len() - activated_snapshot.len();
                        debug!("tool_search activated {new_count} new deferred tool(s)");
                        activated_snapshot = current;
                        tools_defs = self
                            .tools
                            .get_tool_definitions_with_activated(&activated_snapshot);
                        // Re-apply exfiltration guard
                        if self.exfiltration_guard.enabled {
                            let allowed = &self.exfiltration_guard.allow_tools;
                            tools_defs.retain(|td| {
                                let is_network = self
                                    .tools
                                    .get(&td.name)
                                    .is_some_and(|t| t.capabilities().network_outbound);
                                !is_network || allowed.contains(&td.name)
                            });
                        }
                        // Re-apply tool filter (GuidedLLM constraint)
                        if let Some(policy) = overrides.routing_policy.as_ref() {
                            tools_defs.retain(|td| {
                                policy.allowed_tools.contains(&td.name)
                                    || activated_snapshot.contains(&td.name)
                                    || td.name == "add_buttons"
                                    || td.name == "tool_search"
                            });
                        }
                        tool_names = tools_defs.iter().map(|td| td.name.clone()).collect();
                        tools_arc = Arc::new(tools_defs);
                    }
                }
            } else if let Some(content) = response.content {
                match hallucination::handle_text_response(
                    &content,
                    &mut messages,
                    any_tools_called,
                    &mut layer1_fired,
                    &tool_names,
                ) {
                    TextAction::Continue => {}
                    TextAction::Return => {
                        if layer1_fired {
                            if any_tools_called || !hallucination::contains_action_claims(&content)
                            {
                                hallucination::record_retry_success();
                            } else {
                                hallucination::record_retry_failure();
                            }
                        }
                        let content = strip_think_tags(&content);
                        let content = prepend_display_text(
                            content,
                            &collected_tool_metadata,
                            Some(&self.leak_detector),
                            self.prompt_guard
                                .as_ref()
                                .map(|g| (g, &self.prompt_guard_config)),
                        );
                        let mut response_metadata =
                            self.take_pending_buttons_metadata(&activation_scope);
                        merge_suggested_buttons(&mut response_metadata, &collected_tool_metadata);
                        return Ok(AgentLoopResult {
                            content: Some(content),
                            input_tokens: last_input_tokens,
                            tools_used,
                            media: collected_media,
                            reasoning_content: response.reasoning_content,
                            reasoning_signature: response.reasoning_signature,
                            response_metadata,
                            tool_metadata: collected_tool_metadata,
                        });
                    }
                }
            } else {
                // Empty response
                if empty_retries_left > 0 {
                    empty_retries_left -= 1;
                    let retry_num = EMPTY_RESPONSE_RETRIES - empty_retries_left;
                    let delay = (RETRY_BACKOFF_BASE.pow(retry_num as u32) as f64 + fastrand::f64())
                        .min(MAX_RETRY_DELAY_SECS);
                    warn!(
                        "LLM returned empty on iteration {}, retries left: {}, backing off {:.1}s",
                        iteration, empty_retries_left, delay
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs_f64(delay)).await;
                    continue;
                }
                warn!("LLM returned empty, no retries left - giving up");
                break;
            }
        }

        // Collect pending buttons from the add_buttons tool (if any)
        let mut response_metadata = self.take_pending_buttons_metadata(&activation_scope);
        merge_suggested_buttons(&mut response_metadata, &collected_tool_metadata);

        // If tools were called but the loop ended without final content,
        // make one more LLM call with no tools to force a text summary.
        if any_tools_called
            && let Some(content) = self
                .generate_post_loop_summary(
                    &mut messages,
                    effective_model,
                    effective_provider.as_ref(),
                    overrides.request_id.as_deref(),
                )
                .await?
        {
            let content = strip_think_tags(&content);
            let content = prepend_display_text(
                content,
                &collected_tool_metadata,
                Some(&self.leak_detector),
                self.prompt_guard
                    .as_ref()
                    .map(|g| (g, &self.prompt_guard_config)),
            );
            return Ok(AgentLoopResult {
                content: Some(content),
                input_tokens: last_input_tokens,
                tools_used,
                media: collected_media,
                reasoning_content: None,
                reasoning_signature: None,
                response_metadata,
                tool_metadata: collected_tool_metadata,
            });
        }

        // If no LLM response but tools provided display_text, use that as the response
        if let Some(display) = extract_display_text(
            &collected_tool_metadata,
            Some(&self.leak_detector),
            self.prompt_guard
                .as_ref()
                .map(|g| (g, &self.prompt_guard_config)),
        ) {
            return Ok(AgentLoopResult {
                content: Some(display),
                input_tokens: last_input_tokens,
                tools_used,
                media: collected_media,
                reasoning_content: None,
                reasoning_signature: None,
                response_metadata,
                tool_metadata: collected_tool_metadata,
            });
        }

            Ok(AgentLoopResult {
                content: None,
                input_tokens: last_input_tokens,
                tools_used,
                media: collected_media,
                reasoning_content: None,
                reasoning_signature: None,
                response_metadata,
                tool_metadata: collected_tool_metadata,
            })
        }
        .await;

        self.tool_search_activated.clear(&activation_scope).await;
        self.pending_buttons.clear(&activation_scope);
        result
    }

    /// Execute tool calls — single-tool fast-path or parallel `spawn`+`join_all`.
    async fn execute_tools(
        &self,
        tool_calls: &[ToolCallRequest],
        tool_names: &[String],
        exec_ctx: &ExecutionContext,
        exfil_guard: Option<&crate::config::ExfiltrationGuardConfig>,
        routing_policy: Option<&crate::router::RoutingPolicy>,
    ) -> Vec<ToolResult> {
        let allow_tools: Option<Vec<String>> = exfil_guard.map(|g| g.allow_tools.clone());
        let router_allow: Option<std::collections::HashSet<String>> =
            routing_policy.map(|_| tool_names.iter().cloned().collect());
        let router_block: Option<std::collections::HashSet<String>> =
            routing_policy.map(|policy| policy.blocked_tools.iter().cloned().collect());
        let blocked_by_router = |name: &str| {
            router_block
                .as_ref()
                .is_some_and(|blocked| blocked.contains(name))
                || router_allow
                    .as_ref()
                    .is_some_and(|allow| !allow.contains(name))
        };
        if tool_calls.len() == 1 {
            let tc = &tool_calls[0];
            if blocked_by_router(&tc.name) {
                crate::router::metrics::record_blocked_tool_attempt();
                return vec![ToolResult::error(format!(
                    "Tool '{}' is not allowed in this routed turn.",
                    tc.name
                ))];
            }
            vec![
                execute_tool_call(
                    &self.tools,
                    &tc.name,
                    &tc.arguments,
                    tool_names,
                    exec_ctx,
                    allow_tools.as_deref(),
                    Some(&self.workspace),
                )
                .await,
            ]
        } else {
            let shared_names: Arc<Vec<String>> = Arc::from(tool_names.to_vec());
            let handles: Vec<_> = tool_calls
                .iter()
                .map(|tc| {
                    let registry = self.tools.clone();
                    let tc_name = tc.name.clone();
                    let tc_args = tc.arguments.clone();
                    let available = shared_names.clone();
                    let ctx = exec_ctx.clone();
                    let allow = allow_tools.clone();
                    let ws = self.workspace.clone();
                    let blocked = blocked_by_router(&tc_name);
                    tokio::task::spawn(async move {
                        if blocked {
                            crate::router::metrics::record_blocked_tool_attempt();
                            return ToolResult::error(format!(
                                "Tool '{tc_name}' is not allowed in this routed turn."
                            ));
                        }
                        execute_tool_call(
                            &registry,
                            &tc_name,
                            &tc_args,
                            &available,
                            &ctx,
                            allow.as_deref(),
                            Some(&ws),
                        )
                        .await
                    })
                })
                .collect();
            futures_util::future::join_all(handles)
                .await
                .into_iter()
                .map(|join_result| match join_result {
                    Ok(result) => result,
                    Err(join_err) => {
                        error!("Tool task panicked: {:?}", join_err);
                        ToolResult::error("Tool crashed unexpectedly")
                    }
                })
                .collect()
        }
    }

    /// Collect media from tool results, scan for prompt injection, update
    /// cognitive tracking, and fire periodic checkpoints.
    #[allow(clippy::too_many_arguments)]
    async fn handle_tool_results(
        &self,
        messages: &mut Vec<Message>,
        tool_calls: &[ToolCallRequest],
        results: Vec<ToolResult>,
        collected_media: &mut Vec<String>,
        collected_tool_metadata: &mut Vec<(String, HashMap<String, serde_json::Value>)>,
        checkpoint_tracker: &mut CheckpointTracker,
        exec_ctx: &ExecutionContext,
    ) {
        // Add all results to messages in order and collect media.
        // Pad if lengths mismatch (should not happen, but ensures every tool call
        // gets a result so safety checks below still scan all entries).
        let mut results = results;
        if results.len() < tool_calls.len() {
            error!(
                "tool_calls and results length mismatch: {} vs {} — adding error results for missing entries",
                tool_calls.len(),
                results.len()
            );
            while results.len() < tool_calls.len() {
                results.push(ToolResult::error("Tool execution result was lost"));
            }
        }
        for (tc, result) in tool_calls.iter().zip(results) {
            if !result.is_error {
                collected_media.extend(extract_media_paths(&result.content));
            }
            // Collect metadata sideband (stripped from LLM context)
            if let Some(meta) = result.metadata {
                collected_tool_metadata.push((tc.name.clone(), meta));
            }
            ContextBuilder::add_tool_result(
                messages,
                &tc.id,
                &tc.name,
                &result.content,
                result.is_error,
            );
        }

        // Scan tool results for leaked secrets and prompt injection in a single
        // pass. The results were just appended in order, so index directly instead
        // of reverse-searching by tool_call_id.
        let results_start = messages.len() - tool_calls.len();
        for (i, tc) in tool_calls.iter().enumerate() {
            let msg = &mut messages[results_start + i];
            debug_assert!(msg.role == "tool" && msg.tool_call_id.as_deref() == Some(&tc.id));

            // Leak detection
            let redacted = self.leak_detector.redact(&msg.content);
            if redacted != msg.content {
                warn!(
                    "security: secret detected in tool '{}' output — redacting",
                    tc.name
                );
                msg.content = redacted;
            }

            // Prompt injection guard
            if let Some(ref guard) = self.prompt_guard {
                let tool_matches = guard.scan(&msg.content);
                if !tool_matches.is_empty() {
                    for m in &tool_matches {
                        warn!(
                            "security: prompt injection in tool '{}' output ({:?}): {}",
                            tc.name, m.category, m.pattern_name
                        );
                    }
                    if self.prompt_guard_config.should_block() {
                        msg.content = format!(
                            "[tool output redacted: prompt injection detected in '{}']",
                            tc.name
                        );
                    }
                }
            }
        }

        // Record tool calls for cognitive checkpoint tracking
        let called_tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
        checkpoint_tracker.record_tool_calls(&called_tool_names);

        // Inject cognitive pressure message if a new threshold was crossed
        if let Some(pressure_msg) = checkpoint_tracker.pressure_message() {
            messages.push(Message::system(pressure_msg));
        }

        // Update cognitive breadcrumb for compaction recovery
        if self.cognitive_config.enabled
            && let Some(session_key) = exec_ctx
                .metadata
                .get(SESSION_KEY_META_KEY)
                .and_then(serde_json::Value::as_str)
        {
            self.set_session_cognitive_breadcrumb(session_key, checkpoint_tracker.breadcrumb())
                .await;
        }
    }

    /// Read and clear pending buttons from the shared `add_buttons` tool state.
    /// Returns a metadata map with the `buttons` key if any were set.
    fn take_pending_buttons_metadata(
        &self,
        request_id: &str,
    ) -> std::collections::HashMap<String, serde_json::Value> {
        let mut meta = std::collections::HashMap::new();
        if let Some(specs) = self.pending_buttons.take(request_id) {
            let buttons_json: Vec<serde_json::Value> = specs
                .into_iter()
                .map(|b| {
                    let mut btn = serde_json::json!({
                        "id": b.id,
                        "label": b.label,
                        "style": b.style,
                    });
                    if let Some(ctx) = b.context {
                        btn["context"] = serde_json::Value::String(ctx);
                    }
                    btn
                })
                .collect();
            meta.insert(
                crate::bus::meta::BUTTONS.to_string(),
                serde_json::Value::Array(buttons_json),
            );
        }
        meta
    }

    /// Post-loop LLM call with no tools to force a text summary when the loop
    /// ended after tool calls without producing a final text response.
    async fn generate_post_loop_summary(
        &self,
        messages: &mut Vec<Message>,
        effective_model: &str,
        effective_provider: &dyn LLMProvider,
        request_id: Option<&str>,
    ) -> Result<Option<String>> {
        messages.push(Message::user(
            "Provide a brief summary of what you accomplished for the user.".to_string(),
        ));
        match super::model_gateway::ModelGateway::invoke(
            effective_provider,
            super::model_gateway::ModelGateway::build_summary_request(
                messages.clone(),
                effective_model,
                self.max_tokens,
                self.temperature,
            ),
        )
        .await
        {
            Ok(response) => {
                let cost_model = response
                    .actual_model
                    .as_deref()
                    .unwrap_or(effective_model)
                    .to_string();
                let db = self.memory.db();
                let input = response.input_tokens.unwrap_or(0);
                let output = response.output_tokens.unwrap_or(0);
                let cache_create = response.cache_creation_input_tokens.unwrap_or(0);
                let cache_read = response.cache_read_input_tokens.unwrap_or(0);
                let req_id = request_id.map(str::to_string);
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = db.record_tokens(
                        &cost_model,
                        input,
                        output,
                        cache_create,
                        cache_read,
                        "main",
                        req_id.as_deref(),
                    ) {
                        warn!("failed to record token usage: {}", e);
                    }
                });
                Ok(response.content)
            }
            Err(e) => {
                warn!("post-loop summary LLM call failed: {}", e);
                Ok(None)
            }
        }
    }
}
