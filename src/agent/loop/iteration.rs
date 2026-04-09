use super::config::{AgentLoopResult, AgentRunOverrides};
use super::hallucination::{self, TextAction};
use super::{
    AUTO_CONTINUE_MAX, AUTO_CONTINUE_MIN_TOOL_CALLS, AgentLoop, CHARS_PER_TOKEN_ESTIMATE,
    EMPTY_RESPONSE_RETRIES, MAX_RETRY_DELAY_SECS, MIN_WRAPUP_ITERATION, PREFLIGHT_COMPACTION_RATIO,
    RETRY_BACKOFF_BASE, WRAPUP_THRESHOLD_RATIO,
};
use crate::agent::cognitive::CheckpointTracker;
use crate::agent::context::ContextBuilder;
use crate::providers::base::{LLMProvider, Message, ToolCallRequest};

use super::helpers::{
    ApprovalContext, execute_tool_call, extract_media_paths, start_typing, strip_think_tags,
};
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
        let mut total_tool_calls: usize = 0;
        let mut auto_continue_count: usize = 0;

        // Clear request-scoped deferred tool activations from previous retries/reuse.
        self.tool_search_activated.clear(&activation_scope).await;
        self.pending_buttons.clear(&activation_scope);
        let result = async {
            let mut activated_snapshot = std::collections::HashSet::new();

            let (mut tool_names, mut tools_arc) =
                self.prepare_tool_definitions(
                    &activated_snapshot,
                    overrides.routing_policy.as_ref(),
                );

            self.augment_system_prompt(
                &mut messages,
                &tool_names,
                overrides.routing_policy.as_ref(),
            );

            let wrapup_threshold = Self::compute_wrapup_threshold(effective_max_iterations);

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

            // Pre-flight token estimation: trim oldest non-system messages if
            // estimated token count exceeds 80% of the compaction threshold.
            // Prevents wasted API calls that would fail with context-length errors.
            if self.compaction_config.threshold_tokens > 0 {
                let msg_chars: usize = messages.iter().map(|m| m.content.len()).sum();
                let tool_def_chars: usize = tools_arc.iter().map(|td| {
                    td.name.len() + td.description.len() + td.parameters.to_string().len()
                }).sum();
                let estimated_tokens =
                    (msg_chars + tool_def_chars) / CHARS_PER_TOKEN_ESTIMATE;
                let context_limit =
                    self.compaction_config.threshold_tokens as usize;
                let threshold = context_limit * PREFLIGHT_COMPACTION_RATIO / 5;
                if estimated_tokens > threshold {
                    debug!(
                        "pre-flight: estimated {} tokens exceeds 80% of {} limit, \
                         trimming oldest messages",
                        estimated_tokens, context_limit
                    );
                    // Drop oldest non-system messages until under threshold.
                    // Keep system prompt (index 0) and the most recent messages.
                    while messages.len() > 2 {
                        let recalc: usize = messages
                            .iter()
                            .map(|m| m.content.len())
                            .sum::<usize>()
                            / CHARS_PER_TOKEN_ESTIMATE;
                        if recalc <= threshold {
                            break;
                        }
                        // Remove the first non-system message
                        if messages.get(1).is_some_and(|m| m.role != "system") {
                            messages.remove(1);
                        } else {
                            break;
                        }
                    }
                }
            }

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
            self.record_tokens_background(&response, cost_model, overrides.request_id.as_deref());

            if response.has_tool_calls() {
                any_tools_called = true;
                total_tool_calls += response.tool_calls.len();
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
                        let (rebuilt_names, rebuilt_arc) = self.prepare_tool_definitions(
                            &activated_snapshot,
                            overrides.routing_policy.as_ref(),
                        );
                        tool_names = rebuilt_names;
                        tools_arc = rebuilt_arc;
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

                        // Auto-continue: if the LLM stopped mid-task after
                        // significant tool use and we have budget remaining,
                        // re-prompt to finish the work.
                        if total_tool_calls >= AUTO_CONTINUE_MIN_TOOL_CALLS
                            && auto_continue_count < AUTO_CONTINUE_MAX
                            && iteration < effective_max_iterations - 1
                        {
                            auto_continue_count += 1;
                            debug!(
                                "auto-continue {}/{}: LLM returned text after {} tool calls, \
                                 re-prompting",
                                auto_continue_count, AUTO_CONTINUE_MAX, total_tool_calls
                            );
                            ContextBuilder::add_assistant_message(
                                &mut messages,
                                Some(&content),
                                None,
                                response.reasoning_content.as_deref(),
                                response.reasoning_signature.as_deref(),
                                response.redacted_thinking_blocks.clone(),
                            );
                            messages.push(Message::user(
                                "You stopped mid-task after using tools. \
                                 Continue with the remaining work, or if \
                                 you're done, provide your final response."
                                    .to_string(),
                            ));
                            continue;
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

    /// Build filtered tool definitions, names, and Arc for the agent loop.
    fn prepare_tool_definitions(
        &self,
        activated: &std::collections::HashSet<String>,
        routing_policy: Option<&crate::router::RoutingPolicy>,
    ) -> (
        Vec<String>,
        Arc<Vec<crate::providers::base::ToolDefinition>>,
    ) {
        let mut defs = self.tools.get_tool_definitions_with_activated(activated);
        Self::apply_tool_definition_filters(
            &mut defs,
            &self.exfiltration_guard,
            &self.tools,
            routing_policy,
            activated,
        );
        if let Some(policy) = routing_policy {
            debug!(
                "router policy active: reason={} tools={}",
                policy.reason,
                defs.len()
            );
        }
        let names: Vec<String> = defs.iter().map(|td| td.name.clone()).collect();
        (names, Arc::new(defs))
    }

    /// Augment the system prompt with tool awareness, router hint, and
    /// cognitive routines.
    fn augment_system_prompt(
        &self,
        messages: &mut [Message],
        tool_names: &[String],
        routing_policy: Option<&crate::router::RoutingPolicy>,
    ) {
        if !tool_names.is_empty()
            && let Some(system_msg) = messages.first_mut()
        {
            system_msg.content.push_str(
                "\n\nYou have tools available. If a user asks you to perform actions, \
                 call the matching tool directly — do not claim tools are unavailable.",
            );
        }

        if let Some(hint) = routing_policy.and_then(|p| p.context_hint.as_ref())
            && let Some(system_msg) = messages.first_mut()
        {
            use std::fmt::Write;
            let capped = if hint.len() > 1000 {
                &hint[..hint.floor_char_boundary(1000)]
            } else {
                hint.as_str()
            };
            let _ = write!(system_msg.content, "\n\n## Active Interaction\n\n{capped}");
        }

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
    }

    /// Compute the iteration at which a wrap-up hint should be injected.
    fn compute_wrapup_threshold(max_iterations: usize) -> usize {
        let threshold = (max_iterations as f64 * WRAPUP_THRESHOLD_RATIO).ceil() as usize;
        let threshold = threshold.max(MIN_WRAPUP_ITERATION);
        if threshold >= max_iterations {
            max_iterations.saturating_sub(1).max(1)
        } else {
            threshold
        }
    }

    /// Apply exfiltration guard and router policy filters to tool definitions.
    ///
    /// Shared between initial setup and post-tool_search activation rebuild.
    fn apply_tool_definition_filters(
        tool_defs: &mut Vec<crate::providers::base::ToolDefinition>,
        exfil_guard: &crate::config::ExfiltrationGuardConfig,
        tools: &crate::agent::tools::ToolRegistry,
        routing_policy: Option<&crate::router::RoutingPolicy>,
        activated: &std::collections::HashSet<String>,
    ) {
        // Exfiltration guard: hide network-outbound tools from the LLM
        if exfil_guard.enabled {
            let allowed = &exfil_guard.allow_tools;
            tool_defs.retain(|td| {
                let is_network = tools
                    .get(&td.name)
                    .is_some_and(|t| t.capabilities().network_outbound);
                !is_network || allowed.allows(&td.name)
            });
        }

        // Router tool filter: constrain available tools for GuidedLLM/SemanticFilter paths
        if let Some(policy) = routing_policy {
            tool_defs.retain(|td| {
                policy.allowed_tools.contains(&td.name)
                    || activated.contains(&td.name)
                    || td.name == "add_buttons"
                    || td.name == "tool_search"
            });
        }
    }

    /// Execute tool calls using wave-based concurrency.
    ///
    /// Tool calls are partitioned into waves based on their concurrency
    /// classification:
    /// - `ReadOnly` calls (action is `read_only`) are batched together
    ///   and run concurrently via `spawn` + `join_all`.
    /// - `SideEffect` calls break the current wave and run sequentially.
    /// - `Exclusive` calls (shell, tmux) always run alone in their own wave.
    ///
    /// Results are collected in the original tool call order regardless of
    /// execution order within waves.
    async fn execute_tools(
        &self,
        tool_calls: &[ToolCallRequest],
        tool_names: &[String],
        exec_ctx: &ExecutionContext,
        exfil_guard: Option<&crate::config::ExfiltrationGuardConfig>,
        routing_policy: Option<&crate::router::RoutingPolicy>,
    ) -> Vec<ToolResult> {
        use crate::agent::tools::base::ToolConcurrency;

        let allow_tools: Option<crate::config::DenyByDefaultList> =
            exfil_guard.map(|g| g.allow_tools.clone());
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
        // Clone approval fields for spawned tasks (cheap Arc clones)
        let approval_store = self.approval_store.clone();
        let approval_config = self.approval_config.clone();
        let approval_tx = self.outbound_tx.clone();
        let exec_channel = exec_ctx.channel.clone();
        let exec_chat_id = exec_ctx.chat_id.clone();
        let exec_sender_id = exec_ctx
            .metadata
            .get("user_id")
            .and_then(|v| v.as_str())
            .unwrap_or(&exec_ctx.channel)
            .to_string();

        // Single tool call: skip wave partitioning overhead
        if tool_calls.len() == 1 {
            let tc = &tool_calls[0];
            if blocked_by_router(&tc.name) {
                crate::router::metrics::record_blocked_tool_attempt();
                return vec![ToolResult::error(format!(
                    "Tool '{}' is not allowed in this routed turn.",
                    tc.name
                ))];
            }
            return vec![
                execute_tool_call(
                    &self.tools,
                    &tc.name,
                    &tc.arguments,
                    tool_names,
                    exec_ctx,
                    allow_tools.as_ref(),
                    Some(&self.workspace),
                    Some(ApprovalContext {
                        store: &approval_store,
                        config: &approval_config,
                        outbound_tx: &approval_tx,
                        leak_detector: &self.leak_detector,
                        channel: &exec_channel,
                        chat_id: &exec_chat_id,
                        sender_id: &exec_sender_id,
                    }),
                )
                .await,
            ];
        }

        // Classify each tool call's effective concurrency
        let classifications: Vec<ToolConcurrency> = tool_calls
            .iter()
            .map(|tc| self.classify_tool_call_concurrency(tc))
            .collect();

        // Partition into waves: consecutive ReadOnly calls form a parallel
        // wave; SideEffect calls form single-item sequential waves;
        // Exclusive calls always get their own wave.
        let mut waves: Vec<Vec<usize>> = Vec::new();
        let mut current_readonly_wave: Vec<usize> = Vec::new();

        for (i, class) in classifications.iter().enumerate() {
            match class {
                ToolConcurrency::Exclusive | ToolConcurrency::SideEffect => {
                    if !current_readonly_wave.is_empty() {
                        waves.push(std::mem::take(&mut current_readonly_wave));
                    }
                    waves.push(vec![i]);
                }
                ToolConcurrency::ReadOnly => {
                    current_readonly_wave.push(i);
                }
            }
        }
        if !current_readonly_wave.is_empty() {
            waves.push(current_readonly_wave);
        }

        let wave_count = waves.len();
        let parallel_waves = waves.iter().filter(|w| w.len() > 1).count();
        if parallel_waves > 0 {
            debug!(
                "wave execution: {} tool call(s) in {} wave(s), {} parallel",
                tool_calls.len(),
                wave_count,
                parallel_waves
            );
        }

        // Pre-allocate results with placeholders, indexed by original position
        let mut results: Vec<Option<ToolResult>> = (0..tool_calls.len()).map(|_| None).collect();

        // Execute each wave
        let shared_names: Arc<Vec<String>> = Arc::from(tool_names.to_vec());
        for wave in &waves {
            let is_parallel = wave.len() > 1;

            if is_parallel {
                // Parallel wave: spawn all ReadOnly calls and join
                let handles: Vec<_> = wave
                    .iter()
                    .map(|&idx| {
                        let tc = &tool_calls[idx];
                        let registry = self.tools.clone();
                        let tc_name = tc.name.clone();
                        let tc_args = tc.arguments.clone();
                        let available = shared_names.clone();
                        let ctx = exec_ctx.clone();
                        let allow = allow_tools.clone();
                        let ws = self.workspace.clone();
                        let blocked = blocked_by_router(&tc_name);
                        let a_store = approval_store.clone();
                        let a_config = approval_config.clone();
                        let a_tx = approval_tx.clone();
                        let a_channel = exec_channel.clone();
                        let a_chat_id = exec_chat_id.clone();
                        let a_sender_id = exec_sender_id.clone();
                        let a_leak = self.leak_detector.clone();
                        tokio::task::spawn(async move {
                            if blocked {
                                crate::router::metrics::record_blocked_tool_attempt();
                                return ToolResult::error(format!(
                                    "Tool '{tc_name}' is not allowed \
                                     in this routed turn."
                                ));
                            }
                            execute_tool_call(
                                &registry,
                                &tc_name,
                                &tc_args,
                                &available,
                                &ctx,
                                allow.as_ref(),
                                Some(&ws),
                                Some(ApprovalContext {
                                    store: &a_store,
                                    config: &a_config,
                                    outbound_tx: &a_tx,
                                    leak_detector: &a_leak,
                                    channel: &a_channel,
                                    chat_id: &a_chat_id,
                                    sender_id: &a_sender_id,
                                }),
                            )
                            .await
                        })
                    })
                    .collect();

                let wave_results = futures_util::future::join_all(handles).await;
                for (wave_pos, &idx) in wave.iter().enumerate() {
                    results[idx] = Some(match &wave_results[wave_pos] {
                        Ok(result) => result.clone(),
                        Err(join_err) => {
                            error!("Tool task panicked: {:?}", join_err);
                            ToolResult::error("Tool crashed unexpectedly")
                        }
                    });
                }
            } else {
                // Sequential wave: single SideEffect or Exclusive call
                for &idx in wave {
                    let tc = &tool_calls[idx];
                    if blocked_by_router(&tc.name) {
                        crate::router::metrics::record_blocked_tool_attempt();
                        results[idx] = Some(ToolResult::error(format!(
                            "Tool '{}' is not allowed in this routed turn.",
                            tc.name
                        )));
                        continue;
                    }
                    results[idx] = Some(
                        execute_tool_call(
                            &self.tools,
                            &tc.name,
                            &tc.arguments,
                            tool_names,
                            exec_ctx,
                            allow_tools.as_ref(),
                            Some(&self.workspace),
                            Some(ApprovalContext {
                                store: &approval_store,
                                config: &approval_config,
                                outbound_tx: &approval_tx,
                                leak_detector: &self.leak_detector,
                                channel: &exec_channel,
                                chat_id: &exec_chat_id,
                                sender_id: &exec_sender_id,
                            }),
                        )
                        .await,
                    );
                }
            }
        }

        // Unwrap results in original order
        results
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                r.unwrap_or_else(|| {
                    error!("missing result for tool call index {i}");
                    ToolResult::error("Tool execution result was lost")
                })
            })
            .collect()
    }

    /// Determine the effective concurrency for a single tool call.
    ///
    /// Priority: tool-level `Exclusive` overrides everything. For
    /// action-based tools, the specific action's `read_only` flag
    /// determines `ReadOnly` vs `SideEffect`. For single-purpose tools,
    /// the tool-level `concurrency` field is used directly.
    fn classify_tool_call_concurrency(
        &self,
        tc: &ToolCallRequest,
    ) -> crate::agent::tools::base::ToolConcurrency {
        use crate::agent::tools::base::ToolConcurrency;

        let Some(tool) = self.tools.get(&tc.name) else {
            return ToolConcurrency::SideEffect;
        };
        let caps = tool.capabilities();

        if caps.concurrency == ToolConcurrency::Exclusive {
            return ToolConcurrency::Exclusive;
        }

        // For action-based tools, check the specific action's read_only flag
        if !caps.actions.is_empty() {
            let action = tc
                .arguments
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let is_readonly = caps.actions.iter().any(|a| a.name == action && a.read_only);
            if is_readonly {
                return ToolConcurrency::ReadOnly;
            }
            return ToolConcurrency::SideEffect;
        }

        // Single-purpose tools: use the declared concurrency
        caps.concurrency
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
            let missing: Vec<&str> = tool_calls[results.len()..]
                .iter()
                .map(|tc| tc.name.as_str())
                .collect();
            error!(
                "tool result count mismatch: expected {}, got {}, missing {} result(s) for {:?}",
                tool_calls.len(),
                results.len(),
                tool_calls.len() - results.len(),
                missing
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

    /// Fire-and-forget token recording on a blocking thread.
    fn record_tokens_background(
        &self,
        response: &crate::providers::base::LLMResponse,
        model: &str,
        request_id: Option<&str>,
    ) {
        let db = self.memory.db();
        let model = model.to_string();
        let input = response.input_tokens.unwrap_or(0);
        let output = response.output_tokens.unwrap_or(0);
        let cache_create = response.cache_creation_input_tokens.unwrap_or(0);
        let cache_read = response.cache_read_input_tokens.unwrap_or(0);
        let req_id = request_id.map(str::to_string);
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
                let cost_model = response.actual_model.as_deref().unwrap_or(effective_model);
                self.record_tokens_background(&response, cost_model, request_id);
                Ok(response.content)
            }
            Err(e) => {
                warn!("post-loop summary LLM call failed: {}", e);
                Ok(None)
            }
        }
    }
}
