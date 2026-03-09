use super::config::{AgentLoopResult, AgentRunOverrides};
use super::hallucination::{self, CorrectionState, TextAction};
use super::tool_filter::{TOOL_FILTER_THRESHOLD, infer_tool_categories};
use super::{
    AgentLoop, EMPTY_RESPONSE_RETRIES, MAX_RETRY_DELAY_SECS, MIN_WRAPUP_ITERATION,
    RETRY_BACKOFF_BASE, WRAPUP_THRESHOLD_RATIO,
};
use crate::agent::cognitive::CheckpointTracker;
use crate::agent::context::ContextBuilder;
use crate::providers::base::{LLMProvider, Message, ToolCallRequest};

use super::helpers::{execute_tool_call, extract_media_paths, start_typing, strip_think_tags};
use crate::agent::tools::base::ExecutionContext;
use anyhow::Result;
use tracing::{debug, error, warn};

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
        user_has_action_intent: bool,
    ) -> Result<AgentLoopResult> {
        let effective_model = overrides.model.as_deref().unwrap_or(&self.model);
        let effective_provider = overrides.provider.as_ref().unwrap_or(&self.provider);
        let effective_max_iterations = overrides.max_iterations.unwrap_or(self.max_iterations);
        let mut empty_retries_left = EMPTY_RESPONSE_RETRIES;
        let mut any_tools_called = false;
        let mut correction_state = CorrectionState::new();
        let mut last_input_tokens: Option<u64> = None;
        let mut tools_used: Vec<String> = Vec::new();
        let mut collected_media: Vec<String> = Vec::new();
        let mut checkpoint_tracker = CheckpointTracker::new(self.cognitive_config.clone());

        // Clear deferred tool activations from previous runs
        self.tool_search_activated.lock().await.clear();
        let mut activated_snapshot = std::collections::HashSet::new();

        // Tool pre-filtering: when total tools > threshold, select only
        // categories relevant to the user's message to reduce prompt noise.
        let tools_defs = {
            let total_tools = self.tools.tool_names().len();
            if total_tools > TOOL_FILTER_THRESHOLD {
                let user_content = messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "user")
                    .map_or("", |m| m.content.as_str());
                let categories = infer_tool_categories(user_content);
                let defs = self
                    .tools
                    .get_filtered_definitions_with_activated(&categories, &activated_snapshot);
                debug!(
                    "tool pre-filter: {}/{} tools in {} categories",
                    defs.len(),
                    total_tools,
                    categories.len()
                );
                defs
            } else {
                self.tools
                    .get_tool_definitions_with_activated(&activated_snapshot)
            }
        };

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

        // Extract tool names for hallucination detection (may be rebuilt if tool_search activates deferred tools)
        let mut tool_names: Vec<String> = tools_defs.iter().map(|td| td.name.clone()).collect();
        // Build Aho-Corasick automaton for single-pass tool mention scanning
        let mut tool_mention_ac = aho_corasick::AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(&tool_names)
            .ok();

        // Anti-hallucination instruction in the system prompt. The tool definitions
        // sent via the API `tools` parameter already list all available tools with
        // descriptions, so we don't duplicate the name list here — just reinforce
        // that tools ARE available and should be called directly.
        if !tool_names.is_empty()
            && let Some(system_msg) = messages.first_mut()
        {
            system_msg.content.push_str(
                "\n\nYou have tools available. If a user asks for external actions, \
                 do not claim tools are unavailable — call the matching tool directly.",
            );
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
            let response = effective_provider
                .chat_with_retry(
                    crate::providers::base::ChatRequest {
                        messages: messages.clone(),
                        tools: Some(tools_defs.clone()),
                        model: Some(effective_model.to_string()),
                        max_tokens: self.max_tokens,
                        temperature: current_temp,
                        tool_choice,
                        response_format: overrides.response_format.clone(),
                    },
                    Some(crate::providers::base::RetryConfig::default()),
                )
                .await;

            // Stop typing indicator after LLM call returns (guard aborts on drop)
            drop(typing_guard);

            let response = response?;

            // Track provider-reported input token count for precise compaction decisions
            if response.input_tokens.is_some() {
                last_input_tokens = response.input_tokens;
            }

            // Record token usage — use actual_model from fallback provider when
            // the primary failed and a different provider served it
            let cost_model = response.actual_model.as_deref().unwrap_or(effective_model);
            if let Err(e) = self.memory.db().record_tokens(
                cost_model,
                response.input_tokens.unwrap_or(0),
                response.output_tokens.unwrap_or(0),
                response.cache_creation_input_tokens.unwrap_or(0),
                response.cache_read_input_tokens.unwrap_or(0),
                "main",
                overrides.request_id.as_deref(),
            ) {
                warn!("failed to record token usage: {}", e);
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
                );

                // Start periodic typing indicator before tool execution
                let typing_guard = start_typing(self.typing_tx.as_ref(), typing_context.as_ref());

                let exfil_ref = if self.exfiltration_guard.enabled {
                    Some(&self.exfiltration_guard)
                } else {
                    None
                };
                let results = self
                    .execute_tools(&response.tool_calls, &tool_names, exec_ctx, exfil_ref)
                    .await;

                // Stop typing indicator after tool execution (guard aborts on drop)
                drop(typing_guard);

                self.handle_tool_results(
                    &mut messages,
                    &response.tool_calls,
                    results,
                    &mut collected_media,
                    &mut checkpoint_tracker,
                )
                .await;

                // If tool_search activated new deferred tools, rebuild tool
                // definitions so the LLM sees their schemas in the next iteration.
                if self.tools.deferred_count() > 0 {
                    let current = self.tool_search_activated.lock().await.clone();
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
                        tool_names = tools_defs.iter().map(|td| td.name.clone()).collect();
                        tool_mention_ac = aho_corasick::AhoCorasick::builder()
                            .ascii_case_insensitive(true)
                            .build(&tool_names)
                            .ok();
                    }
                }
            } else if let Some(content) = response.content {
                match hallucination::handle_text_response(
                    &content,
                    &mut messages,
                    response.reasoning_content.as_deref(),
                    any_tools_called,
                    &mut correction_state,
                    &tool_names,
                    &tools_used,
                    user_has_action_intent,
                    Some(&self.memory.db()),
                    overrides.request_id.as_deref(),
                    tool_mention_ac.as_ref(),
                ) {
                    TextAction::Continue => {}
                    TextAction::Return => {
                        let content = strip_think_tags(&content);
                        return Ok(AgentLoopResult {
                            content: Some(content),
                            input_tokens: last_input_tokens,
                            tools_used,
                            media: collected_media,
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
            return Ok(AgentLoopResult {
                content: Some(content),
                input_tokens: last_input_tokens,
                tools_used,
                media: collected_media,
            });
        }

        Ok(AgentLoopResult {
            content: None,
            input_tokens: last_input_tokens,
            tools_used,
            media: collected_media,
        })
    }

    /// Execute tool calls — single-tool fast-path or parallel `spawn`+`join_all`.
    async fn execute_tools(
        &self,
        tool_calls: &[ToolCallRequest],
        tool_names: &[String],
        exec_ctx: &ExecutionContext,
        exfil_guard: Option<&crate::config::ExfiltrationGuardConfig>,
    ) -> Vec<(String, bool)> {
        let allow_tools: Option<Vec<String>> = exfil_guard.map(|g| g.allow_tools.clone());
        if tool_calls.len() == 1 {
            let tc = &tool_calls[0];
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
            let handles: Vec<_> = tool_calls
                .iter()
                .map(|tc| {
                    let registry = self.tools.clone();
                    let tc_name = tc.name.clone();
                    let tc_args = tc.arguments.clone();
                    let available = tool_names.to_vec();
                    let ctx = exec_ctx.clone();
                    let allow = allow_tools.clone();
                    let ws = self.workspace.clone();
                    tokio::task::spawn(async move {
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
                        ("Tool crashed unexpectedly".to_string(), true)
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
        results: Vec<(String, bool)>,
        collected_media: &mut Vec<String>,
        checkpoint_tracker: &mut CheckpointTracker,
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
                results.push(("Tool execution result was lost".to_string(), true));
            }
        }
        for (tc, (result_str, is_error)) in tool_calls.iter().zip(results) {
            if !is_error {
                collected_media.extend(extract_media_paths(&result_str));
            }
            ContextBuilder::add_tool_result(messages, &tc.id, &tc.name, &result_str, is_error);
        }

        // Scan tool results for leaked secrets before they enter LLM context
        for tc in tool_calls {
            if let Some(msg) = messages
                .iter_mut()
                .rev()
                .find(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some(&tc.id))
            {
                let redacted = self.leak_detector.redact(&msg.content);
                if redacted != msg.content {
                    warn!("secret detected in tool '{}' output — redacting", tc.name);
                    msg.content = redacted;
                }
            }
        }

        // Scan tool results for prompt injection
        if let Some(ref guard) = self.prompt_guard {
            for tc in tool_calls {
                if let Some(msg) = messages
                    .iter_mut()
                    .rev()
                    .find(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some(&tc.id))
                {
                    let tool_matches = guard.scan(&msg.content);
                    if !tool_matches.is_empty() {
                        for m in &tool_matches {
                            warn!(
                                "prompt injection in tool '{}' output ({:?}): {}",
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
        }

        // Record tool calls for cognitive checkpoint tracking
        let called_tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
        checkpoint_tracker.record_tool_calls(&called_tool_names);

        // Inject cognitive pressure message if a new threshold was crossed
        if let Some(pressure_msg) = checkpoint_tracker.pressure_message() {
            messages.push(Message::system(pressure_msg));
        }

        // Update cognitive breadcrumb for compaction recovery
        if self.cognitive_config.enabled {
            *self.cognitive_breadcrumb.lock().await = Some(checkpoint_tracker.breadcrumb());
        }
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
        match effective_provider
            .chat_with_retry(
                crate::providers::base::ChatRequest {
                    messages: messages.clone(),
                    model: Some(effective_model.to_string()),
                    max_tokens: self.max_tokens,
                    temperature: self.temperature,
                    ..Default::default()
                },
                Some(crate::providers::base::RetryConfig::default()),
            )
            .await
        {
            Ok(response) => {
                let cost_model = response.actual_model.as_deref().unwrap_or(effective_model);
                if let Err(e) = self.memory.db().record_tokens(
                    cost_model,
                    response.input_tokens.unwrap_or(0),
                    response.output_tokens.unwrap_or(0),
                    response.cache_creation_input_tokens.unwrap_or(0),
                    response.cache_read_input_tokens.unwrap_or(0),
                    "main",
                    request_id,
                ) {
                    warn!("failed to record token usage: {}", e);
                }
                Ok(response.content)
            }
            Err(e) => {
                warn!("post-loop summary LLM call failed: {}", e);
                Ok(None)
            }
        }
    }
}
