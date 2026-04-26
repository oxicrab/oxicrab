use super::config::{AgentLoopResult, AgentRunOverrides};
use super::{
    AgentLoop, CHARS_PER_TOKEN_ESTIMATE, EMPTY_RESPONSE_RETRIES, MAX_RETRY_DELAY_SECS,
    MIN_WRAPUP_ITERATION, PREFLIGHT_COMPACTION_RATIO, RETRY_BACKOFF_BASE, WRAPUP_THRESHOLD_RATIO,
};
use crate::agent::cognitive::CheckpointTracker;
use crate::agent::context::ContextBuilder;
use crate::agent::trajectory::TrajectoryLogger;
use crate::providers::base::{LLMProvider, LLMResponse, Message, StreamChunk, ToolCallRequest};
use crate::providers::streaming::{BeginOutcome as StreamBeginOutcome, StreamOutcome};

use super::helpers::{
    ApprovalContext, execute_tool_call, extract_media_paths, start_typing, strip_think_tags,
};
use super::metadata::{extract_display_text, merge_suggested_buttons, prepend_display_text};
use crate::agent::tools::ToolRegistry;
use crate::agent::tools::base::{ExecutionContext, ToolConcurrency, ToolResult};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Classify the effective concurrency for a single tool call based on the
/// tool's declared capabilities and the specific action being invoked.
///
/// Priority: tool-level `Exclusive` overrides everything. For action-based
/// tools, the specific action's `read_only` flag determines `ReadOnly` vs
/// `SideEffect`. For single-purpose tools with a single action descriptor
/// and no `action` param, the descriptor's `read_only` flag is used. For
/// tools with no actions, the tool-level `concurrency` field is used.
pub(super) fn classify_tool_call_concurrency(
    registry: &ToolRegistry,
    tc: &ToolCallRequest,
) -> ToolConcurrency {
    let Some(tool) = registry.get(&tc.name) else {
        return ToolConcurrency::SideEffect;
    };
    let caps = tool.capabilities();

    if caps.concurrency == ToolConcurrency::Exclusive {
        return ToolConcurrency::Exclusive;
    }

    if !caps.actions.is_empty() {
        let action_name = tc
            .arguments
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let is_readonly = if action_name.is_empty() && caps.actions.len() == 1 {
            caps.actions[0].read_only
        } else if action_name.is_empty() && caps.actions.len() > 1 {
            debug!(
                "tool '{}' called without action param but has {} action descriptors, \
                 defaulting to SideEffect",
                tc.name,
                caps.actions.len()
            );
            false
        } else {
            caps.actions
                .iter()
                .any(|a| a.name == action_name && a.read_only)
        };
        if is_readonly {
            return ToolConcurrency::ReadOnly;
        }
        return ToolConcurrency::SideEffect;
    }

    caps.concurrency
}

/// Partition tool call indices into execution waves based on their
/// concurrency classifications.
///
/// Consecutive `ReadOnly` calls form a single parallel wave.
/// `SideEffect` and `Exclusive` calls each get their own single-item wave.
pub(super) fn partition_into_waves(classifications: &[ToolConcurrency]) -> Vec<Vec<usize>> {
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

    waves
}

const SESSION_KEY_META_KEY: &str = "session_key";

/// After this many consecutive empty responses with `any_tools_called`,
/// flip force-text mode and strip tools from subsequent LLM calls.
const FORCE_TEXT_AFTER_EMPTIES: u8 = 2;

/// After this many consecutive iterations where every tool call
/// failed, flip force-text mode so the model produces a user-facing
/// message instead of looping on the same broken tool.
const FORCE_TEXT_AFTER_ALL_ERROR: u8 = 2;

/// Number of times an identical tool fingerprint must fire before
/// the duplicate-call detector engages. Combined with the time-gate
/// below to avoid penalising legitimate poll-until-ready loops.
const DUPLICATE_CALL_THRESHOLD: u8 = 3;

/// The duplicate-call detector only engages after the run has been
/// going for at least this long. Short runs that fire the same call
/// 3× in 5 seconds are usually intentional retries, not stuck loops.
const DUPLICATE_CALL_MIN_ELAPSED_SECS: u64 = 30;

impl AgentLoop {
    /// Core agent loop implementation with per-invocation overrides.
    ///
    /// Iterates up to `max_iterations` rounds of: LLM call → parallel tool execution → append results.
    /// Uses `tool_choice=None` (auto) on all iterations. At 70% of max iterations, a wrap-up
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
        let mut last_was_tool_only = false;
        let mut tool_call_count: usize = 0;
        // Cancellation token: register a token for this session so
        // external callers (`AgentLoop::cancel_session(key)`) can
        // abort the in-flight turn. Drops on guard.
        let cancel_token = overrides.cancellation_token.clone().unwrap_or_default();
        let _cancel_guard = exec_ctx
            .metadata
            .get(SESSION_KEY_META_KEY)
            .and_then(serde_json::Value::as_str)
            .map(|sid| self.register_cancel_token(sid, cancel_token.clone()));
        // Text emitted alongside tool_calls is preserved here so we have
        // a fallback if the final EndTurn comes back empty. Sonnet 4.6
        // routinely emits text like "Let me check your calendar…" right
        // before a tool_use block — without this the user gets
        // "No response generated." even though the model DID say
        // something useful. Cleared for each new run; multiple chunks
        // joined by blank lines on fallback.
        let mut accumulated_text: Vec<String> = Vec::new();
        // Capture the trigger user message up front so the refine hook
        // can use it (and replay select_skills_for_query against it).
        let initial_user_message = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.clone())
            .unwrap_or_default();
        // Trajectory: opt-in per-turn observability. Resolved up front so
        // the hot loop just calls `if let Some(...)` without re-checking
        // `self.trajectory_config.enabled` on every event.
        let trajectory_session_key = exec_ctx
            .metadata
            .get(SESSION_KEY_META_KEY)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let (trajectory_logger, trajectory_turn) = if self.trajectory_config.enabled {
            if let Some(ref sid) = trajectory_session_key {
                let mut counters = self
                    .trajectory_turn_counters
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let entry = counters.entry(sid.clone()).or_insert(0);
                *entry = entry.saturating_add(1);
                let turn = *entry;
                drop(counters);
                let logger = TrajectoryLogger::new(self.memory.db());
                (Some(logger), turn)
            } else {
                (None, 0)
            }
        } else {
            (None, 0)
        };
        // Trajectory tool-call dispatch instants — used to compute
        // latency for `log_tool_result` events.
        let mut trajectory_call_start: Vec<std::time::Instant> = Vec::new();
        let mut last_input_tokens: Option<u64> = None;
        let mut tools_used: Vec<String> = Vec::new();
        // Force-text mode: set when the LLM is stuck in a tool-loop
        // failure pattern (consecutive empty responses, or every tool
        // result in a row coming back as is_error). Once set, all
        // tools are stripped from subsequent LLM calls so the model
        // is forced to output text.
        let mut force_text_mode = false;
        let mut consecutive_empty_responses: u8 = 0;
        let mut consecutive_all_error_iterations: u8 = 0;
        // Identical tool-call tracker: maps fingerprint(tool, args) →
        // count. The detector engages once a fingerprint hits
        // DUPLICATE_CALL_THRESHOLD AND the run has been going for
        // DUPLICATE_CALL_MIN_ELAPSED_SECS — both gates protect
        // legitimate poll-until-ready workflows from being killed.
        let run_started = std::time::Instant::now();
        let mut duplicate_call_counts: std::collections::HashMap<u64, u8> =
            std::collections::HashMap::new();
        let mut collected_media: Vec<String> = Vec::new();
        let mut collected_tool_metadata: Vec<(String, HashMap<String, serde_json::Value>)> =
            Vec::new();
        let mut checkpoint_tracker = CheckpointTracker::new(self.cognitive_config.clone());
        let mut reflection_budget = super::reflection::ReflectionBudget::new();
        // (tool, action) keys that produced a reflection in the previous
        // iteration whose outcome is not yet written back. Drained on
        // the next iteration's tool results.
        let mut pending_reflection_outcomes: std::collections::HashSet<(String, Option<String>)> =
            std::collections::HashSet::new();

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
            // Hard-budget escalation at 90% — by this point the model
            // MUST stop calling tools and produce text. The <system_notice>
            // structured tag is more LLM-readable than prose.
            let urgent_threshold = ((effective_max_iterations as f64) * 0.9).ceil() as usize;

            for iteration in 1..=effective_max_iterations {
            // Force-text mode: strip tools so the model is compelled to
            // output text. Triggered from the bottom of the previous
            // iteration when consecutive failures cross the threshold.
            // We rebuild tools_arc to empty AND inject a system notice
            // so the model knows why its tools just disappeared.
            let force_text_active = force_text_mode;
            if force_text_active && !tools_arc.is_empty() {
                warn!(
                    "force-text mode: stripping {} tool definitions for remaining iterations \
                     (consecutive_empty={} consecutive_all_error={})",
                    tools_arc.len(),
                    consecutive_empty_responses,
                    consecutive_all_error_iterations,
                );
                tools_arc = Arc::new(Vec::new());
                tool_names.clear();
                messages.push(Message::system(
                    "<system_notice type=\"force_text\">\
                     Tools have been disabled for the remainder of this turn because previous \
                     attempts produced no progress. Output a final user-facing text response \
                     using whatever information you already have.\
                     </system_notice>"
                        .to_string(),
                ));
            }
            // Inject wrap-up hint when approaching iteration limit
            if iteration == wrapup_threshold && any_tools_called {
                let remaining = effective_max_iterations.saturating_sub(iteration);
                messages.push(Message::system(format!(
                    "<system_notice type=\"iteration_budget\" remaining=\"{remaining}/{effective_max_iterations}\">\
                     You're 70% through the tool-iteration budget. Wrap up: summarize progress \
                     and deliver results in your next response.\
                     </system_notice>"
                )));
            }
            if iteration == urgent_threshold
                && urgent_threshold > wrapup_threshold
                && any_tools_called
            {
                let remaining = effective_max_iterations.saturating_sub(iteration);
                messages.push(Message::system(format!(
                    "<system_notice type=\"iteration_budget\" remaining=\"{remaining}/{effective_max_iterations}\" severity=\"urgent\">\
                     You have very few iterations left. Stop calling tools. \
                     Output a final user-facing text response NOW with whatever you have.\
                     </system_notice>"
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
            // Let the model decide when to use tools (auto mode). No
            // post-hoc hallucination check is wired — pattern-based
            // second-guessing of LLM text caused false positives.
            let tool_choice: Option<String> = None;

            // Pre-flight token estimation: trim oldest non-system messages if
            // estimated token count exceeds 80% of the compaction threshold.
            // Prevents wasted API calls that would fail with context-length errors.
            if self.compaction_config.enabled && self.compaction_config.threshold_tokens > 0 {
                let msg_bytes: usize = messages
                    .iter()
                    .map(|m| {
                        let mut bytes = m.content.len();
                        if let Some(ref rc) = m.reasoning_content {
                            bytes += rc.len();
                        }
                        bytes
                    })
                    .sum();
                let tool_def_bytes: usize = tools_arc
                    .iter()
                    .map(|td| {
                        td.name.len() + td.description.len() + td.parameters.to_string().len()
                    })
                    .sum();
                let estimated_tokens =
                    (msg_bytes + tool_def_bytes) / CHARS_PER_TOKEN_ESTIMATE;
                let context_limit = self.compaction_config.threshold_tokens as usize;
                let threshold = context_limit * PREFLIGHT_COMPACTION_RATIO / 5;
                if estimated_tokens > threshold {
                    debug!(
                        "pre-flight: estimated {} tokens exceeds 80% of {} limit, \
                         trimming oldest messages",
                        estimated_tokens, context_limit
                    );
                    let tool_tokens = tool_def_bytes / CHARS_PER_TOKEN_ESTIMATE;
                    // Drop oldest non-system messages until under threshold.
                    // Keep system prompt (index 0) and the most recent messages.
                    while messages.len() > 2 {
                        let recalc: usize = messages
                            .iter()
                            .map(|m| {
                                let mut bytes = m.content.len();
                                if let Some(ref rc) = m.reasoning_content {
                                    bytes += rc.len();
                                }
                                bytes
                            })
                            .sum::<usize>()
                            / CHARS_PER_TOKEN_ESTIMATE;
                        if recalc + tool_tokens <= threshold {
                            break;
                        }
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
            //
            // When the previous response was tool-calls-only (no text content),
            // strip tools from this request so the LLM must produce text. This
            // avoids the empty-response → post-loop-summary fallback path.
            let request = if last_was_tool_only {
                debug!("stripping tools from request after tool-only response");
                super::model_gateway::ModelGateway::build_summary_request(
                    messages.clone(),
                    effective_model,
                    self.max_tokens,
                    self.temperature,
                )
            } else {
                super::model_gateway::ModelGateway::build_turn_request(
                    messages.clone(),
                    Arc::clone(&tools_arc),
                    effective_model,
                    self.max_tokens,
                    current_temp,
                    tool_choice,
                    overrides.response_format.clone(),
                )
            };
            // Wrap the LLM call in:
            // - a hard timeout (prevents session-lock starvation on
            //   hung providers)
            // - a cancellation-token select so the user / external
            //   `cancel_session` can abort cleanly mid-call.
            // Cancellation takes priority over both timeout and
            // completion via tokio::select!'s biased polling.
            let llm_timeout_secs = self.llm_request_timeout_secs;
            // Stream every iteration that has a dispatcher attached.
            // The dispatcher's `begin_emitted` atomic ensures at-most
            // one Begin across the whole run, so mixed text+tool
            // iterations don't open a new placeholder for each text
            // chunk. Tool-call-only iterations never emit a Delta,
            // so Begin never fires and no live message exists. This
            // also covers first-turn text-only Q&A — a Delta arrives,
            // Begin fires, the user sees progressive edits.
            let stream_now = overrides.stream_dispatcher.is_some();
            let response = if stream_now {
                let dispatcher = overrides.stream_dispatcher.clone().expect("checked above");
                run_streaming_call(
                    effective_provider.as_ref(),
                    &request,
                    cancel_token.clone(),
                    llm_timeout_secs,
                    &dispatcher,
                )
                .await
            } else {
                let invoke_future = super::model_gateway::ModelGateway::invoke(
                    effective_provider.as_ref(),
                    request,
                );
                tokio::select! {
                    biased;
                    () = cancel_token.cancelled() => {
                        info!("lLM request cancelled by token");
                        Err(anyhow::anyhow!("turn cancelled"))
                    }
                    r = async {
                        if llm_timeout_secs > 0 {
                            if let Ok(r) = tokio::time::timeout(
                                std::time::Duration::from_secs(llm_timeout_secs.into()),
                                invoke_future,
                            )
                            .await
                            {
                                r
                            } else {
                                warn!(
                                    "lLM request timed out after {}s — releasing session lock",
                                    llm_timeout_secs
                                );
                                Err(anyhow::anyhow!(
                                    "LLM request timed out after {llm_timeout_secs}s"
                                ))
                            }
                        } else {
                            invoke_future.await
                        }
                    } => r,
                }
            };

            // Stop typing indicator after LLM call returns (guard aborts on drop)
            drop(typing_guard);

            let mut response = response?;

            // Track provider-reported input token count for precise compaction decisions
            if response.input_tokens.is_some() {
                last_input_tokens = response.input_tokens;
            }

            // Record token usage off the async runtime (fire-and-forget)
            let cost_model = response.actual_model.as_deref().unwrap_or(effective_model);
            self.record_tokens_background(&response, cost_model, overrides.request_id.as_deref());

            // Phantom-tool-call guard: some API gateways inject a
            // tool_calls payload while finish_reason indicates a real
            // stop (content_filter, error, stop). Without this check
            // we dispatch the phantom call, get nothing back, and
            // loop until max_iterations.
            //
            // MUST run before the tool-only / empty-response
            // classifications below — otherwise a real text-stop with
            // a phantom tool_calls block would set
            // last_was_tool_only=true and the NEXT iteration would
            // strip tools for what was actually a normal text turn.
            if response.has_tool_calls() && !response.is_tool_use_finish() {
                warn!(
                    "discarding phantom tool_calls block: finish_reason={:?} \
                     (gateway injection or provider bug)",
                    response.finish_reason
                );
                response.tool_calls.clear();
            }

            // Track whether this response had tool calls but no text — if so,
            // the next iteration will strip tools to force a text response.
            last_was_tool_only = response.has_tool_calls() && response.content.is_none();

            // Force-text counters: empty-response counter increments
            // when the model returned nothing AND no tool calls. Reset
            // on any non-empty response so transient flakiness doesn't
            // accumulate.
            //
            // Dual-state invariant: `consecutive_empty_responses`
            // resets on a tool-only response (since `has_tool_calls()`
            // is true → else branch → reset). That is intentional:
            // tool-only responses are NOT empty for force-text
            // purposes, and the SEPARATE `last_was_tool_only` flag
            // (computed below from the same response) drives the
            // tools-stripped retry on the next iteration. Both gates
            // must agree on this classification — keep them updated
            // together if you change one.
            if !response.has_tool_calls() && response.content.is_none() {
                consecutive_empty_responses = consecutive_empty_responses.saturating_add(1);
                if any_tools_called && consecutive_empty_responses >= FORCE_TEXT_AFTER_EMPTIES {
                    force_text_mode = true;
                }
            } else {
                consecutive_empty_responses = 0;
            }

            if response.has_tool_calls() {
                // Duplicate-call detector. Fingerprint each call
                // (tool name + canonical args) and bump the counter.
                // After DUPLICATE_CALL_THRESHOLD AND past the
                // time-gate, flip force-text mode and inject a
                // notice so the model knows why we're stopping it.
                let elapsed_secs = run_started.elapsed().as_secs();
                let mut hit_duplicate = false;
                for tc in &response.tool_calls {
                    let fp = fingerprint_tool_call(&tc.name, &tc.arguments);
                    let count = duplicate_call_counts.entry(fp).or_insert(0);
                    *count = count.saturating_add(1);
                    if *count >= DUPLICATE_CALL_THRESHOLD
                        && elapsed_secs >= DUPLICATE_CALL_MIN_ELAPSED_SECS
                    {
                        hit_duplicate = true;
                        warn!(
                            "duplicate-call threshold hit: tool='{}' count={} elapsed={}s",
                            tc.name, count, elapsed_secs
                        );
                    }
                }
                if hit_duplicate {
                    force_text_mode = true;
                    messages.push(Message::system(
                        "<system_notice type=\"duplicate_calls\">\
                         The same tool call has been repeated several times. \
                         Tools have been disabled. Output a final text response \
                         summarising what you have so far.\
                         </system_notice>"
                            .to_string(),
                    ));
                    response.tool_calls.clear();
                    // Fall through; with cleared tool_calls the response
                    // looks like an empty turn, which the empty-handler
                    // below will deal with.
                }
            }
            if response.has_tool_calls() {
                any_tools_called = true;
                tools_used.extend(response.tool_calls.iter().map(|tc| tc.name.clone()));
                // Capture any text that came alongside the tool_calls.
                // This is the prose Sonnet emits before a tool_use block
                // ("Let me check…"), which is otherwise lost when the
                // final iteration returns content=None. Used as the
                // fallback content if the post-loop summary also fails.
                if let Some(ref text) = response.content
                    && !text.trim().is_empty()
                {
                    accumulated_text.push(text.clone());
                }
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
                // Trajectory: log every dispatched call BEFORE execution
                // so a panicking tool still leaves a breadcrumb. Latency
                // is measured against the wave start instant.
                if let (Some(logger), Some(sid)) = (
                    trajectory_logger.as_ref(),
                    trajectory_session_key.as_ref(),
                ) {
                    let wave_start = std::time::Instant::now();
                    trajectory_call_start.clear();
                    for tc in &response.tool_calls {
                        let action = tc
                            .arguments
                            .get("action")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty());
                        logger.log_tool_call(sid, trajectory_turn, &tc.name, action);
                        trajectory_call_start.push(wave_start);
                    }
                }
                let mut results = self
                    .execute_tools(
                        &response.tool_calls,
                        &tool_names,
                        exec_ctx,
                        exfil_ref,
                        overrides.routing_policy.as_ref(),
                        &initial_user_message,
                    )
                    .await;
                tool_call_count += response.tool_calls.len();

                // Force-text counter: track iterations where every
                // tool call came back as is_error=true. After
                // FORCE_TEXT_AFTER_ALL_ERROR consecutive such
                // iterations, strip tools so the model is compelled
                // to acknowledge the failure to the user instead of
                // looping on broken tools.
                if !results.is_empty() && results.iter().all(|r| r.is_error) {
                    consecutive_all_error_iterations =
                        consecutive_all_error_iterations.saturating_add(1);
                    if consecutive_all_error_iterations >= FORCE_TEXT_AFTER_ALL_ERROR {
                        force_text_mode = true;
                    }
                } else {
                    consecutive_all_error_iterations = 0;
                }
                // Trajectory: log results paired with the dispatched calls.
                if let (Some(logger), Some(sid)) = (
                    trajectory_logger.as_ref(),
                    trajectory_session_key.as_ref(),
                ) {
                    for (idx, tc) in response.tool_calls.iter().enumerate() {
                        let result = results.get(idx);
                        let is_error = result.is_some_and(|r| r.is_error);
                        let action = tc
                            .arguments
                            .get("action")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty());
                        let latency_ms = trajectory_call_start
                            .get(idx)
                            .map_or(0, |t| t.elapsed().as_millis() as i64);
                        logger.log_tool_result(
                            sid,
                            trajectory_turn,
                            &tc.name,
                            action,
                            is_error,
                            latency_ms,
                        );
                    }
                }

                // Stop typing indicator after tool execution (guard aborts on drop)
                drop(typing_guard);

                // First: write back outcomes for any reflections produced
                // in the previous iteration whose tools just ran again.
                // Done before the new reflection pass so the metric
                // ("did the retry succeed") reflects this iteration's
                // outcome, not a future one.
                if self.reflection_config.enabled
                    && self.reflection_config.persist_to_db
                    && !pending_reflection_outcomes.is_empty()
                    && let Some(req_id) = overrides.request_id.as_deref()
                {
                    self.write_back_reflection_outcomes(
                        &response.tool_calls,
                        &results,
                        &mut pending_reflection_outcomes,
                        req_id,
                    );
                }

                // Reflection pass: for each error result, optionally produce
                // a structured hypothesis + retry strategy and inject it
                // into the result content for the next iteration. Bounded
                // by `ReflectionConfig` per-request and per-tool caps.
                if self.reflection_config.enabled {
                    self.augment_results_with_reflection(
                        &response.tool_calls,
                        &mut results,
                        &mut reflection_budget,
                        &mut pending_reflection_outcomes,
                        effective_provider.as_ref(),
                        effective_model,
                        overrides.request_id.as_deref(),
                    )
                    .await;
                }

                self.handle_tool_results(
                    &mut messages,
                    &response.tool_calls,
                    results,
                    &mut collected_media,
                    &mut collected_tool_metadata,
                    &mut checkpoint_tracker,
                    &mut duplicate_call_counts,
                    exec_ctx,
                )
                .await;

                // Mid-turn injection: drain any queued user messages
                // and append them as new user-role Message entries
                // for the next iteration. The queue handle is on
                // AgentLoop, set by process_message_with_pending.
                // Skipped for cron / direct dispatch (no queue handle).
                if let Some(queue_arc) = self.current_pending_queue() {
                    let drained: Vec<crate::bus::InboundMessage> = {
                        let mut q = queue_arc
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        std::mem::take(&mut *q)
                    };
                    if !drained.is_empty() {
                        info!(
                            "mid-turn injection: {} queued message(s) appended to in-flight turn",
                            drained.len()
                        );
                        for queued in &drained {
                            inject_queued_message(&mut messages, queued);
                        }
                    }
                }

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
                if let (Some(logger), Some(sid)) = (
                    trajectory_logger.as_ref(),
                    trajectory_session_key.as_ref(),
                ) {
                    logger.log_turn_end(sid, trajectory_turn);
                }
                self.maybe_spawn_skill_auto_suggest(trajectory_session_key.as_deref());
                self.maybe_spawn_skill_refine(
                    &initial_user_message,
                    &content,
                    tool_call_count,
                );
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
            } else {
                // Empty response — if tools were already called, give the
                // model one nudge to continue (call another tool) or
                // summarize. Falling straight to the no-tools post-loop
                // summary lets the model fabricate text from tool data
                // alone (observed in production: cron list -> empty ->
                // post-loop summary invented "I'll fire the job" + "but I
                // don't have a tool"). The nudge keeps tools available for
                // one more iteration; if it stays empty, fall to summary.
                if any_tools_called {
                    if consecutive_empty_responses <= 1 {
                        warn!(
                            "lLM returned empty after tool calls on iteration {}, \
                             nudging to continue or summarize",
                            iteration
                        );
                        messages.push(Message::system(
                            "Your previous tool call(s) returned results. \
                             Decide your next move: either call another tool to \
                             continue the work, or write a final user-facing \
                             text response that summarizes what you found. \
                             Empty responses are not allowed."
                                .to_string(),
                        ));
                        continue;
                    }
                    warn!(
                        "lLM returned empty after tool calls on iteration {} (post-nudge), \
                         falling through to post-loop summary",
                        iteration
                    );
                    break;
                }
                if empty_retries_left > 0 {
                    empty_retries_left -= 1;
                    let retry_num = EMPTY_RESPONSE_RETRIES - empty_retries_left;
                    // saturating_pow guards against overflow if
                    // EMPTY_RESPONSE_RETRIES is ever bumped.
                    let delay = (RETRY_BACKOFF_BASE.saturating_pow(retry_num as u32) as f64
                        + fastrand::f64())
                        .min(MAX_RETRY_DELAY_SECS);
                    warn!(
                        "lLM returned empty on iteration {}, retries left: {}, backing off {:.1}s",
                        iteration, empty_retries_left, delay
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs_f64(delay)).await;
                    continue;
                }
                warn!("lLM returned empty, no retries left - giving up");
                break;
            }
        }

        // Collect pending buttons from the add_buttons tool (if any)
        let mut response_metadata = self.take_pending_buttons_metadata(&activation_scope);
        merge_suggested_buttons(&mut response_metadata, &collected_tool_metadata);

        // If tools were called but the loop ended without final content,
        // make one more LLM call with no tools to force a text summary.
        if any_tools_called
            && let Some(summary) = self
                .generate_post_loop_summary(
                    &mut messages,
                    effective_model,
                    effective_provider.as_ref(),
                    overrides.request_id.as_deref(),
                )
                .await?
        {
            let content = strip_think_tags(&summary.content);
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
                reasoning_content: summary.reasoning_content,
                reasoning_signature: summary.reasoning_signature,
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

            // Last resort before returning content=None: surface the
            // text the model emitted alongside its tool_calls earlier
            // in the run. Without this, an empty final EndTurn loses
            // whatever the model already said (e.g. "Looking at your
            // calendar…"). The user has been confused by exactly this.
            if !accumulated_text.is_empty() {
                let fallback = accumulated_text.join("\n\n");
                let fallback = strip_think_tags(&fallback);
                let fallback = prepend_display_text(
                    fallback,
                    &collected_tool_metadata,
                    Some(&self.leak_detector),
                    self.prompt_guard
                        .as_ref()
                        .map(|g| (g, &self.prompt_guard_config)),
                );
                warn!(
                    "loop ended with empty final content; falling back to {} chars of text \
                     emitted alongside earlier tool_calls",
                    fallback.len()
                );
                return Ok(AgentLoopResult {
                    content: Some(fallback),
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

        // Router tool filter: constrain available tools for GuidedLLM/
        // SemanticFilter paths. `add_buttons` is always available because
        // it's a UX-only helper (no side effects, no data access) — the
        // routing policy shouldn't block interactive affordances.
        //
        // `tool_search` is deliberately NOT exempt here: it would let a
        // constrained turn activate arbitrary deferred/MCP tools that the
        // policy did not approve, and those tools then match via
        // `activated.contains()` on subsequent iterations. Allow it only
        // when the policy explicitly lists it.
        if let Some(policy) = routing_policy {
            tool_defs.retain(|td| {
                policy.allowed_tools.contains(&td.name)
                    || activated.contains(&td.name)
                    || td.name == "add_buttons"
            });
        }
    }

    /// For each tool result with `is_error = true`, attempt to produce a
    /// reflection (hypothesis + retry strategy) and append it to the
    /// result content as a `<reflection>…</reflection>` block. Bounded
    /// by `ReflectionConfig` per-request and per-tool caps.
    ///
    /// When `ReflectionConfig.persist_to_db` is true and the agent has
    /// a memory database, the reflection is persisted to the
    /// `tool_reflections` table so operators can analyse failure modes
    /// over time.
    #[allow(clippy::too_many_arguments)]
    async fn augment_results_with_reflection(
        &self,
        tool_calls: &[ToolCallRequest],
        results: &mut [ToolResult],
        budget: &mut super::reflection::ReflectionBudget,
        pending_outcomes: &mut std::collections::HashSet<(String, Option<String>)>,
        provider: &dyn LLMProvider,
        model: &str,
        request_id: Option<&str>,
    ) {
        debug_assert_eq!(tool_calls.len(), results.len());
        for (tc, result) in tool_calls.iter().zip(results.iter_mut()) {
            if !result.is_error {
                continue;
            }
            let action = tc.arguments.get("action").and_then(|v| v.as_str());
            let Some(reflection) = super::reflection::reflect_on_failure(
                &self.reflection_config,
                budget,
                provider,
                model,
                &self.leak_detector,
                Some(&self.memory.db()),
                request_id.unwrap_or(""),
                &tc.name,
                action,
                &result.content,
            )
            .await
            else {
                continue;
            };

            let block = reflection.render_block();
            result.content.push_str(&block);

            if self.reflection_config.persist_to_db {
                let now_ms = chrono::Utc::now().timestamp_millis();
                let rec = crate::agent::memory::memory_db::ReflectionRecord {
                    request_id: request_id.unwrap_or("").to_string(),
                    tool_name: reflection.tool.clone(),
                    action: reflection.action.clone(),
                    attempt_number: reflection.attempt_number,
                    error_excerpt: reflection.error_excerpt.clone(),
                    hypothesis: reflection.hypothesis.clone(),
                    retry_strategy: reflection.retry_strategy.clone(),
                    next_outcome: None,
                    created_at_ms: now_ms,
                };
                if let Err(e) = self.memory.db().insert_tool_reflection(&rec) {
                    warn!("reflection: failed to persist record: {e}");
                }
            }

            // Mark this (tool, action) as having a pending outcome so the
            // next iteration's tool result can be written back via
            // `update_reflection_outcome`.
            pending_outcomes.insert((reflection.tool.clone(), reflection.action.clone()));
        }
    }

    /// Write `next_outcome` for any reflection produced in the previous
    /// iteration whose tool ran again now. Drains matching keys from
    /// `pending_outcomes` so a third iteration that re-runs the same
    /// tool doesn't double-write.
    ///
    /// Anything still in `pending_outcomes` after the matching pass was
    /// not called again on this iteration — its "next outcome" is no
    /// longer observable, so the entry is cleared. Without this, a
    /// later iteration that happens to call the same `(tool, action)`
    /// would be miscredited as the reflection's retry outcome.
    fn write_back_reflection_outcomes(
        &self,
        tool_calls: &[ToolCallRequest],
        results: &[ToolResult],
        pending_outcomes: &mut std::collections::HashSet<(String, Option<String>)>,
        request_id: &str,
    ) {
        for (tc, result) in tool_calls.iter().zip(results.iter()) {
            let action = tc
                .arguments
                .get("action")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let key = (tc.name.clone(), action.clone());
            if !pending_outcomes.remove(&key) {
                continue;
            }
            let outcome = if result.is_error { "error" } else { "success" };
            if let Err(e) = self.memory.db().update_reflection_outcome(
                request_id,
                &tc.name,
                action.as_deref(),
                outcome,
            ) {
                warn!("reflection: failed to update outcome: {e}");
            }
        }
        // Drop unmatched pending entries: the "next outcome" semantics
        // only apply to the immediately following iteration. Leaving
        // them in the set would cross-credit a future unrelated call.
        pending_outcomes.clear();
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
        user_intent: &str,
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
        // Clone approval + judge fields for spawned tasks (cheap Arc clones)
        let approval_store = self.approval_store.clone();
        let approval_config = self.approval_config.clone();
        let approval_tx = self.outbound_tx.clone();
        let judge_config = self.judge_config.clone();
        let judge_provider = self.provider.clone();
        let judge_model = self.model.clone();
        let user_intent_owned = user_intent.to_string();
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
                    if self.judge_config.enabled {
                        Some(super::helpers::JudgeContext {
                            config: &self.judge_config,
                            provider: self.provider.as_ref(),
                            model: &self.model,
                            user_intent,
                        })
                    } else {
                        None
                    },
                )
                .await,
            ];
        }

        // Classify each tool call's effective concurrency
        let classifications: Vec<ToolConcurrency> = tool_calls
            .iter()
            .map(|tc| classify_tool_call_concurrency(&self.tools, tc))
            .collect();

        let waves = partition_into_waves(&classifications);

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
                        let j_config = judge_config.clone();
                        let j_provider = judge_provider.clone();
                        let j_model = judge_model.clone();
                        let j_intent = user_intent_owned.clone();
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
                                if j_config.enabled {
                                    Some(super::helpers::JudgeContext {
                                        config: &j_config,
                                        provider: j_provider.as_ref(),
                                        model: &j_model,
                                        user_intent: &j_intent,
                                    })
                                } else {
                                    None
                                },
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
                            error!("tool task panicked: {:?}", join_err);
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
                            if judge_config.enabled {
                                Some(super::helpers::JudgeContext {
                                    config: &judge_config,
                                    provider: judge_provider.as_ref(),
                                    model: &judge_model,
                                    user_intent: &user_intent_owned,
                                })
                            } else {
                                None
                            },
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
        duplicate_call_counts: &mut HashMap<u64, u8>,
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
                // A successful repeat (poll loop, page-load wait,
                // retry-after-status) should not be flagged as a
                // stuck-in-a-rut loop. Reset the per-fingerprint
                // counter so the duplicate detector only fires when
                // the same args genuinely fail to make progress.
                let fp = fingerprint_tool_call(&tc.name, &tc.arguments);
                duplicate_call_counts.remove(&fp);

                // Prefer pre-truncation media paths recorded by
                // TruncationMiddleware in metadata. Re-scanning the
                // truncated content can miss paths that landed past
                // the cap.
                let from_meta = result
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get(crate::bus::meta::MEDIA_PATHS))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    });
                if let Some(paths) = from_meta {
                    collected_media.extend(paths);
                } else {
                    collected_media.extend(extract_media_paths(&result.content));
                }
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
        messages: &mut [Message],
        effective_model: &str,
        effective_provider: &dyn LLMProvider,
        request_id: Option<&str>,
    ) -> Result<Option<PostLoopSummary>> {
        // Diagnostics from production showed the failure mode is NOT
        // budget exhaustion (input_tokens ~10k, no reasoning content,
        // finish_reason=stop) — the model just decided "I'm done"
        // with empty content. Replaying the full history including
        // the tool_use/tool_result protocol blocks lets the model
        // think it has already "answered" by calling tools.
        //
        // Fix: build a CLEAN message list for the summary call. No
        // tool_use blocks, no tool_result blocks — just a system
        // prompt, the original user question, and the tool data
        // stitched into a plain user-role message. The model cannot
        // confuse this with an already-completed turn.
        const POST_LOOP_MAX_TOKENS_FLOOR: u32 = 16_384;
        let summary_max_tokens = self.max_tokens.max(POST_LOOP_MAX_TOKENS_FLOOR);

        let summary_messages = build_clean_summary_messages(messages);
        let first = self
            .invoke_summary_call(
                summary_messages.clone(),
                effective_model,
                effective_provider,
                summary_max_tokens,
                request_id,
            )
            .await;
        if let Some(s) = first {
            return Ok(Some(s));
        }

        // Even the clean structure came back empty — rare but possible
        // with model flakiness. Retry once with a more directive final
        // user message.
        warn!("post-loop summary returned empty on first attempt; retrying with directive prompt");
        let mut retry_messages = summary_messages;
        retry_messages.push(Message::user(
            "You did not respond. Reply NOW with a brief user-facing message that includes \
             the key fields from the tool data above. An empty reply is unacceptable."
                .to_string(),
        ));
        Ok(self
            .invoke_summary_call(
                retry_messages,
                effective_model,
                effective_provider,
                summary_max_tokens,
                request_id,
            )
            .await)
    }

    async fn invoke_summary_call(
        &self,
        messages: Vec<Message>,
        effective_model: &str,
        effective_provider: &dyn LLMProvider,
        max_tokens: u32,
        request_id: Option<&str>,
    ) -> Option<PostLoopSummary> {
        match super::model_gateway::ModelGateway::invoke(
            effective_provider,
            super::model_gateway::ModelGateway::build_summary_request(
                messages,
                effective_model,
                max_tokens,
                self.temperature,
            ),
        )
        .await
        {
            Ok(response) => {
                let cost_model = response.actual_model.as_deref().unwrap_or(effective_model);
                self.record_tokens_background(&response, cost_model, request_id);
                if response.content.is_none() {
                    // Diagnostics: when this fires, the model spent its
                    // output budget on something other than user-facing
                    // text. The two common causes — context-near-cap
                    // (provider cut us off) and thinking-budget-exhausted
                    // (extended thinking consumed all output tokens) —
                    // both surface here.
                    warn!(
                        "post-loop summary returned content=None: \
                         finish_reason={:?} input_tokens={:?} \
                         had_reasoning_content={}",
                        response.finish_reason,
                        response.input_tokens,
                        response.reasoning_content.is_some(),
                    );
                }
                response.content.map(|content| PostLoopSummary {
                    content,
                    reasoning_content: response.reasoning_content,
                    reasoning_signature: response.reasoning_signature,
                })
            }
            Err(e) => {
                warn!("post-loop summary LLM call failed: {e}");
                None
            }
        }
    }
}

/// Structured return from `generate_post_loop_summary` so thinking-block
/// content survives the final formatting step instead of being silently
/// dropped.
struct PostLoopSummary {
    content: String,
    reasoning_content: Option<String>,
    reasoning_signature: Option<String>,
}

/// Append a mid-turn-queued [`InboundMessage`] to the in-flight
/// `messages` vector, preserving the queued message's media
/// (encoded as image content blocks), action-dispatch payload
/// (rendered as a structured note so the model can react), and
/// any interesting metadata. The bare `queued.content`-only
/// rendering would silently drop attachments, button clicks, and
/// webhook payloads that arrived mid-turn.
fn inject_queued_message(messages: &mut Vec<Message>, queued: &crate::bus::InboundMessage) {
    use std::fmt::Write as _;
    let mut text = format!("[New message from user mid-turn] {}", queued.content);

    if let Some(action) = &queued.action {
        // Render the action dispatch so the model sees what was
        // clicked / which webhook fired. Source label disambiguates
        // button vs webhook vs directive vs tool-chain.
        let _ = write!(
            text,
            "\n[mid-turn action: tool={} source={} params={}]",
            action.tool,
            action.source.label(),
            serde_json::to_string(&action.params).unwrap_or_else(|_| "<unserializable>".into())
        );
    }

    let interesting_meta: Vec<String> = queued
        .metadata
        .iter()
        .filter(|(k, _)| {
            // Limit to channel-side identifiers users might care
            // about; skip internal session bookkeeping like ts.
            matches!(k.as_str(), "sender_id" | "is_group" | "user_id")
        })
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    if !interesting_meta.is_empty() {
        let _ = write!(
            text,
            "\n[mid-turn metadata: {}]",
            interesting_meta.join(", ")
        );
    }

    let (images, warnings) = AgentLoop::encode_non_audio_media(&queued.media);
    if !warnings.is_empty() {
        let _ = write!(text, "\n[mid-turn media warnings: {}]", warnings.join("; "));
    }

    if images.is_empty() {
        messages.push(Message::user(text));
    } else {
        messages.push(Message::user_with_images(text, images));
    }
}

/// Hash a tool call to a u64 for the duplicate-call detector.
///
/// Canonicalises the args by serialising through `serde_json::Value`
/// (sorts object keys lexicographically) so semantically identical
/// payloads with different key order collapse to the same fingerprint.
fn fingerprint_tool_call(name: &str, args: &serde_json::Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut h);
    canonical_json_string(args).hash(&mut h);
    h.finish()
}

/// Stable serialisation that sorts object keys. Two calls with
/// `{"a":1,"b":2}` and `{"b":2,"a":1}` should fingerprint identically.
fn canonical_json_string(v: &serde_json::Value) -> String {
    use serde_json::Value;
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{k:?}:{}", canonical_json_string(&map[k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical_json_string).collect();
            format!("[{}]", parts.join(","))
        }
        other => other.to_string(),
    }
}

/// Build a clean message list for the post-loop summary call. The
/// inner agent loop's `messages` carries the full
/// `tool_use`/`tool_result` protocol; replaying that list lets the
/// model decide it has already "answered" by calling tools. We strip
/// the protocol entirely and present:
///
/// - a focused system prompt
/// - the most recent user-role message that triggered the turn
/// - all tool-result content concatenated as a plain user message
///
/// This bypasses the model's internal "did I already answer?" check
/// because the result blocks no longer carry `tool_call_id` linkage.
const POST_LOOP_TOOL_DATA_CAP: usize = 8_000;

fn build_clean_summary_messages(messages: &[Message]) -> Vec<Message> {
    use std::fmt::Write as _;
    let system = Message::system(
        "You are responding to the user. The user's question is below, followed by data \
         from tools that were just executed on their behalf. Write a brief user-facing reply \
         that incorporates the relevant fields from the tool data. Output text only — \
         no tool calls, no preamble, no meta-commentary about what you did."
            .to_string(),
    );

    // Find the original user query — the last user-role message in the
    // input list whose content isn't itself synthesized (we filter out
    // anything that looks like our own injected nudge).
    let user_query = messages
        .iter()
        .rev()
        .find(|m| {
            m.role == "user"
                && !m.content.starts_with("You did not respond.")
                && !m.content.starts_with("The user's tool calls are complete")
        })
        .map_or_else(
            || Message::user("(original question not recoverable)".to_string()),
            |m| Message::user(m.content.clone()),
        );

    // Collect tool-result content in chronological order, with a
    // total-size cap so a runaway tool can't blow the prompt.
    let mut tool_block = String::new();
    let mut total = 0usize;
    let _ = writeln!(tool_block, "Tool results:");
    for msg in messages {
        if msg.role != "tool" {
            continue;
        }
        if msg.content.trim().is_empty() {
            continue;
        }
        if total + msg.content.len() > POST_LOOP_TOOL_DATA_CAP {
            tool_block.push_str("\n…(further tool results truncated)\n");
            break;
        }
        tool_block.push('\n');
        tool_block.push_str(&msg.content);
        tool_block.push('\n');
        total += msg.content.len();
    }
    if total == 0 {
        // No tool results to surface — rare (the loop only enters
        // post-loop summary after any_tools_called=true) but stay
        // defensive.
        tool_block.push_str("(no tool output captured)\n");
    }

    vec![system, user_query, Message::user(tool_block)]
}

/// Drive a streaming LLM call and pump deltas into `dispatcher`.
/// Emits Begin (at most once across the whole agent run, guarded by
/// the dispatcher's `begin_emitted` flag) plus Delta chunks. Does
/// NOT emit End — the iteration loop's outer caller
/// (`processing.rs`) emits End after the loop completes so it can
/// carry final content plus button metadata in a single
/// `chat.update` / `editMessage`. Buttons therefore land on the
/// streamed message rather than on a sidecar.
///
/// Returns the same `LLMResponse` shape as the non-streaming path so
/// downstream loop logic is unchanged. Tool-call-only iterations
/// never emit a Delta, so Begin never fires for them — the channel
/// layer doesn't see anything for those iterations.
async fn run_streaming_call(
    provider: &dyn LLMProvider,
    request: &crate::providers::base::ChatRequest,
    cancel: tokio_util::sync::CancellationToken,
    timeout_secs: u32,
    dispatcher: &crate::providers::streaming::StreamDispatcher,
) -> Result<LLMResponse> {
    use futures_util::StreamExt;

    let stream_open = if timeout_secs > 0 {
        let Ok(r) = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs.into()),
            provider.chat_stream(request, cancel.clone()),
        )
        .await
        else {
            metrics::counter!(
                "oxicrab_streaming_fallback_to_nonstream_total",
                "reason" => "open_timeout",
            )
            .increment(1);
            return Err(anyhow::anyhow!(
                "stream open timed out after {timeout_secs}s"
            ));
        };
        r
    } else {
        provider.chat_stream(request, cancel.clone()).await
    };

    let mut stream = match stream_open {
        Ok(s) => s,
        Err(e) => {
            metrics::counter!(
                "oxicrab_streaming_fallback_to_nonstream_total",
                "reason" => "open_error",
            )
            .increment(1);
            return Err(e);
        }
    };

    let mut accumulated = String::new();
    let mut response: Option<LLMResponse> = None;
    let mut error: Option<String> = None;

    loop {
        let next = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            c = stream.next() => c,
        };
        let Some(chunk) = next else {
            break;
        };
        match chunk {
            StreamChunk::Delta { text } => {
                if text.is_empty() {
                    continue;
                }
                accumulated.push_str(&text);
                // Try to start the live message. The dispatcher's
                // begin_emitted atomic guarantees at-most-one Begin
                // across the whole agent run, so subsequent
                // streamed iterations skip Begin and just send
                // Deltas against the existing message.
                match dispatcher.begin() {
                    StreamBeginOutcome::Sent | StreamBeginOutcome::AlreadySent => {
                        let _ = dispatcher.delta(&accumulated);
                    }
                    StreamBeginOutcome::ReceiverGone => {
                        debug!("stream consumer receiver dropped; ceasing dispatch");
                    }
                }
            }
            StreamChunk::Finish { response: r } => {
                response = Some(r);
                break;
            }
            StreamChunk::Error { message } => {
                error = Some(message);
                break;
            }
            StreamChunk::ReasoningDelta { .. }
            | StreamChunk::ToolCallStart { .. }
            | StreamChunk::ToolCallArgs { .. } => {
                // Not displayed live. Reasoning is recovered from
                // the final response; tool calls are handled by the
                // post-stream loop.
            }
        }
    }

    if let Some(resp) = response {
        Ok(resp)
    } else if let Some(err) = error {
        // Stream errored mid-flight. If a Begin was emitted, commit
        // whatever we accumulated as the final state so the user
        // sees a coherent partial answer rather than a "thinking…"
        // placeholder stuck forever.
        if dispatcher.has_begun() {
            let _ = dispatcher.end(StreamOutcome::Failed, &accumulated, None);
        }
        metrics::counter!(
            "oxicrab_streaming_fallback_to_nonstream_total",
            "reason" => "stream_error",
        )
        .increment(1);
        Err(anyhow::anyhow!("stream error: {err}"))
    } else {
        // Cancelled mid-stream. Same treatment: commit the
        // accumulated content so the cancelled turn doesn't leave
        // the visible message at the placeholder.
        if dispatcher.has_begun() {
            let _ = dispatcher.end(StreamOutcome::Cancelled, &accumulated, None);
        }
        Err(anyhow::anyhow!("turn cancelled"))
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the wave-classification helpers. The helpers themselves
    //! are `pub(super)` and need access to a real `ToolRegistry`, so we
    //! build a minimal in-process registry with hand-rolled tools that
    //! exercise each branch of `classify_tool_call_concurrency`.
    use super::*;

    #[test]
    fn fingerprint_collapses_object_key_order() {
        let a = json!({"a": 1, "b": 2, "c": 3});
        let b = json!({"c": 3, "b": 2, "a": 1});
        assert_eq!(
            fingerprint_tool_call("foo", &a),
            fingerprint_tool_call("foo", &b)
        );
    }

    #[test]
    fn fingerprint_collapses_nested_object_key_order() {
        let a = json!({"outer": {"x": 1, "y": 2}});
        let b = json!({"outer": {"y": 2, "x": 1}});
        assert_eq!(
            fingerprint_tool_call("foo", &a),
            fingerprint_tool_call("foo", &b)
        );
    }

    #[test]
    fn fingerprint_distinguishes_different_args() {
        let a = json!({"action": "list"});
        let b = json!({"action": "delete"});
        assert_ne!(
            fingerprint_tool_call("foo", &a),
            fingerprint_tool_call("foo", &b)
        );
    }

    #[test]
    fn fingerprint_distinguishes_different_tool_names() {
        let a = json!({"action": "list"});
        assert_ne!(
            fingerprint_tool_call("foo", &a),
            fingerprint_tool_call("bar", &a)
        );
    }

    #[test]
    fn fingerprint_preserves_array_order() {
        let a = json!([1, 2, 3]);
        let b = json!([3, 2, 1]);
        assert_ne!(
            fingerprint_tool_call("foo", &a),
            fingerprint_tool_call("foo", &b)
        );
    }

    use crate::providers::base::ToolCallRequest;
    use async_trait::async_trait;
    use oxicrab_core::actions;
    use oxicrab_core::tools::base::{
        ExecutionContext, Tool, ToolCapabilities, ToolCategory, ToolConcurrency, ToolResult,
    };
    use serde_json::{Value, json};
    use std::sync::Arc;

    /// Tool whose declared concurrency is `Exclusive` regardless of action.
    struct ExclusiveTool;
    #[async_trait]
    impl Tool for ExclusiveTool {
        fn name(&self) -> &'static str {
            "excl"
        }
        fn description(&self) -> &'static str {
            ""
        }
        fn parameters(&self) -> Value {
            json!({})
        }
        fn capabilities(&self) -> ToolCapabilities {
            ToolCapabilities {
                concurrency: ToolConcurrency::Exclusive,
                ..Default::default()
            }
        }
        async fn execute(
            &self,
            _params: Value,
            _ctx: &ExecutionContext,
        ) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::new("ok"))
        }
    }

    /// Action-based tool with one read-only and one mutating action.
    struct MultiActionTool;
    #[async_trait]
    impl Tool for MultiActionTool {
        fn name(&self) -> &'static str {
            "multi"
        }
        fn description(&self) -> &'static str {
            ""
        }
        fn parameters(&self) -> Value {
            json!({})
        }
        fn capabilities(&self) -> ToolCapabilities {
            ToolCapabilities {
                actions: actions![read: ro, write],
                category: ToolCategory::Productivity,
                ..Default::default()
            }
        }
        async fn execute(
            &self,
            _params: Value,
            _ctx: &ExecutionContext,
        ) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::new("ok"))
        }
    }

    /// Single-action tool whose only descriptor is read-only. With no
    /// `action` param, the descriptor's flag should still apply.
    struct SingleReadOnlyAction;
    #[async_trait]
    impl Tool for SingleReadOnlyAction {
        fn name(&self) -> &'static str {
            "single_ro"
        }
        fn description(&self) -> &'static str {
            ""
        }
        fn parameters(&self) -> Value {
            json!({})
        }
        fn capabilities(&self) -> ToolCapabilities {
            ToolCapabilities {
                actions: actions![lookup: ro],
                ..Default::default()
            }
        }
        async fn execute(
            &self,
            _params: Value,
            _ctx: &ExecutionContext,
        ) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::new("ok"))
        }
    }

    fn registry_with(tools: Vec<Arc<dyn Tool>>) -> ToolRegistry {
        let mut r = ToolRegistry::new();
        for t in tools {
            r.register(t);
        }
        r
    }

    fn call(name: &str, args: Value) -> ToolCallRequest {
        ToolCallRequest {
            id: format!("call-{name}"),
            name: name.to_string(),
            arguments: args,
        }
    }

    #[test]
    fn classify_exclusive_tool_overrides_action_inference() {
        let r = registry_with(vec![Arc::new(ExclusiveTool)]);
        let c = classify_tool_call_concurrency(&r, &call("excl", json!({"action": "read"})));
        assert_eq!(c, ToolConcurrency::Exclusive);
    }

    #[test]
    fn classify_unknown_tool_falls_back_to_side_effect() {
        let r = registry_with(vec![]);
        let c = classify_tool_call_concurrency(&r, &call("nope", json!({})));
        assert_eq!(c, ToolConcurrency::SideEffect);
    }

    #[test]
    fn classify_multiaction_with_known_readonly_action_is_readonly() {
        let r = registry_with(vec![Arc::new(MultiActionTool)]);
        let c = classify_tool_call_concurrency(&r, &call("multi", json!({"action": "read"})));
        assert_eq!(c, ToolConcurrency::ReadOnly);
    }

    #[test]
    fn classify_multiaction_with_mutating_action_is_side_effect() {
        let r = registry_with(vec![Arc::new(MultiActionTool)]);
        let c = classify_tool_call_concurrency(&r, &call("multi", json!({"action": "write"})));
        assert_eq!(c, ToolConcurrency::SideEffect);
    }

    #[test]
    fn classify_multiaction_unknown_action_is_side_effect_not_readonly() {
        // Safety default: an action name that doesn't match any
        // descriptor must not be treated as read-only just because the
        // tool happens to have a read-only action.
        let r = registry_with(vec![Arc::new(MultiActionTool)]);
        let c = classify_tool_call_concurrency(&r, &call("multi", json!({"action": "bogus"})));
        assert_eq!(c, ToolConcurrency::SideEffect);
    }

    #[test]
    fn classify_multiaction_missing_action_param_defaults_side_effect() {
        // The gap noted in the review: when a multi-action tool is
        // called with no `action` param, fall back to SideEffect rather
        // than incorrectly treating it as ReadOnly. Prevents racing a
        // mutating call with concurrent reads.
        let r = registry_with(vec![Arc::new(MultiActionTool)]);
        let c = classify_tool_call_concurrency(&r, &call("multi", json!({})));
        assert_eq!(c, ToolConcurrency::SideEffect);
    }

    #[test]
    fn classify_single_action_tool_inherits_descriptor_flag_when_action_omitted() {
        // For tools with exactly one action descriptor, omitting the
        // `action` param is unambiguous — the descriptor's read-only
        // flag should apply directly so callers don't lose parallelism
        // for genuinely read-only tools that have no action dispatch.
        let r = registry_with(vec![Arc::new(SingleReadOnlyAction)]);
        let c = classify_tool_call_concurrency(&r, &call("single_ro", json!({})));
        assert_eq!(c, ToolConcurrency::ReadOnly);
    }

    #[test]
    fn partition_into_waves_groups_consecutive_readonly_calls() {
        let waves = partition_into_waves(&[
            ToolConcurrency::ReadOnly,
            ToolConcurrency::ReadOnly,
            ToolConcurrency::SideEffect,
            ToolConcurrency::ReadOnly,
        ]);
        assert_eq!(waves, vec![vec![0, 1], vec![2], vec![3]]);
    }

    #[test]
    fn partition_into_waves_exclusive_breaks_runs() {
        let waves = partition_into_waves(&[
            ToolConcurrency::ReadOnly,
            ToolConcurrency::Exclusive,
            ToolConcurrency::ReadOnly,
            ToolConcurrency::ReadOnly,
        ]);
        assert_eq!(waves, vec![vec![0], vec![1], vec![2, 3]]);
    }

    #[test]
    fn partition_into_waves_empty_input_yields_no_waves() {
        let waves = partition_into_waves(&[]);
        assert!(waves.is_empty());
    }
}
