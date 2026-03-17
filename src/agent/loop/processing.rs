use super::AgentLoop;
use super::config::AgentRunOverrides;
use super::helpers::{
    load_and_encode_images, strip_audio_tags, strip_document_tags, strip_image_tags,
    transcribe_audio_tags,
};
use crate::agent::tools::base::ExecutionContext;
use crate::bus::{InboundMessage, OutboundMessage};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use tracing::{debug, info, warn};
use uuid::Uuid;

impl AgentLoop {
    pub(super) async fn process_message_unlocked(
        &self,
        msg: InboundMessage,
    ) -> Result<Option<OutboundMessage>> {
        if msg.channel == "system" {
            return self.process_system_message(msg).await;
        }

        // Send typing indicator before processing
        if let Some(ref tx) = self.typing_tx
            && tx
                .send((msg.channel.clone(), msg.chat_id.clone()))
                .await
                .is_err()
        {
            debug!("typing indicator channel closed");
        }

        info!("Processing message from {}:{}", msg.channel, msg.sender_id);

        // Check for event-triggered cron jobs in the background.
        // Periodically rebuild the matcher from the cron store (every 60s)
        // so new/modified event jobs are picked up at runtime.
        if let Some(cron_svc) = &self.cron_service {
            // Check-and-claim: CAS on epoch-seconds timestamp to prevent
            // concurrent messages from triggering duplicate rebuilds.
            let now_epoch = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let last = self
                .event_matcher_last_rebuild
                .load(std::sync::atomic::Ordering::Relaxed);
            let needs_rebuild = now_epoch.saturating_sub(last) >= 60
                && self
                    .event_matcher_last_rebuild
                    .compare_exchange(
                        last,
                        now_epoch,
                        std::sync::atomic::Ordering::AcqRel,
                        std::sync::atomic::Ordering::Relaxed,
                    )
                    .is_ok();
            if needs_rebuild && let Ok(jobs) = cron_svc.list_jobs(true) {
                let mut new_matcher = crate::cron::event_matcher::EventMatcher::from_jobs(&jobs);
                if let Some(ref matcher_mutex) = self.event_matcher
                    && let Ok(mut guard) = matcher_mutex.lock()
                {
                    new_matcher.merge_fired_state(&guard);
                    *guard = new_matcher;
                }
            }

            if let Some(matcher_mutex) = &self.event_matcher {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64);
                let triggered = matcher_mutex
                    .lock()
                    .map(|mut matcher| matcher.check_message(&msg.content, &msg.channel, now_ms))
                    .unwrap_or_default();
                for job in triggered {
                    let cron_svc = cron_svc.clone();
                    let job_id = job.id.clone();
                    info!("Event-triggered cron job '{}' ({})", job.name, job.id);
                    tokio::spawn(async move {
                        if let Err(e) = cron_svc.run_job(&job_id, true).await {
                            warn!("Event-triggered job '{}' failed: {}", job_id, e);
                        }
                    });
                }
            }
        }

        let session_key = msg.session_key();
        // Load session early — the router needs RouterContext from session metadata
        debug!("Loading session: {}", session_key);
        let session = self.sessions.get_or_create(&session_key).await?;

        // Load router context and prune expired directives
        let mut router_context =
            crate::router::context::RouterContext::from_session_metadata(&session.metadata);
        router_context.prune_expired(crate::router::now_ms());

        // Router decides the processing path
        let decision = self
            .router
            .route(&msg.content, &router_context, msg.action.as_ref());

        // Capture tool_filter and context_hint from routing decision before
        // falling through to the normal pipeline
        let (router_tool_filter, router_context_hint) = match &decision {
            crate::router::RoutingDecision::DirectDispatch {
                tool,
                params,
                source,
                directive_index,
            } => {
                // Extract compaction_summary from the already-loaded session so
                // handle_direct_dispatch doesn't need to reload it.
                let context_summary = session
                    .metadata
                    .get("compaction_summary")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string);
                // Handle direct dispatch inline — returns early
                return self
                    .handle_direct_dispatch(
                        tool.clone(),
                        params.clone(),
                        source,
                        *directive_index,
                        &msg,
                        &session_key,
                        &mut router_context,
                        context_summary,
                    )
                    .await;
            }
            crate::router::RoutingDecision::GuidedLLM {
                tool_subset,
                context_hint,
            } => (Some(tool_subset.clone()), Some(context_hint.clone())),
            // NOTE: SemanticFilter is not yet produced by the router; handled
            // here for exhaustiveness in case it is wired in the future.
            crate::router::RoutingDecision::SemanticFilter { tool_subset } => {
                (Some(tool_subset.clone()), None)
            }
            crate::router::RoutingDecision::FullLLM => (None, None),
        };

        // Build execution context for tool calls
        let context_summary = session
            .metadata
            .get("compaction_summary")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        let exec_ctx = Self::build_execution_context_with_metadata(
            &msg.channel,
            &msg.chat_id,
            context_summary,
            msg.metadata.clone(),
        );

        debug!("Getting compacted history");
        let checkpoint_before = self.last_checkpoint.lock().await.clone();
        let history = self
            .get_compacted_history_timed(&session, &session.key)
            .await?;
        debug!("Got {} history messages", history.len());

        // Transcribe any audio files before other processing
        let msg_content = if let Some(ref lazy) = self.transcriber
            && let Some(svc) = lazy.get()
        {
            transcribe_audio_tags(&msg.content, svc).await
        } else {
            strip_audio_tags(&msg.content)
        };

        // Inbound secret scanning: redact secrets before they reach the LLM or
        // get persisted in session history / memory. Uses redact() for a single
        // pass; scan() only runs when redaction occurred (to get pattern names).
        let msg_content = {
            let redacted = self.leak_detector.redact(&msg_content);
            if redacted == msg_content {
                msg_content
            } else {
                let names: Vec<&str> = self
                    .leak_detector
                    .scan(&msg_content)
                    .iter()
                    .map(|m| m.name)
                    .collect();
                warn!(
                    "security: secret detected in inbound message from {}:{}: {:?} — redacted",
                    msg.channel, msg.sender_id, names
                );
                redacted
            }
        };

        // Prompt injection preflight check
        if matches!(
            check_prompt_guard(
                self.prompt_guard.as_ref(),
                &self.prompt_guard_config,
                &msg_content,
                "inbound message",
            ),
            PromptGuardVerdict::Blocked
        ) {
            return Ok(Some(OutboundMessage::from_inbound(msg, "I can't process this message as it appears to contain prompt injection patterns.").build()));
        }

        // Load and encode any attached images (skip audio files)
        let audio_extensions = ["ogg", "mp3", "mp4", "m4a", "wav", "webm", "flac", "oga"];
        let image_media: Vec<String> = msg
            .media
            .iter()
            .filter(|p| {
                let ext = std::path::Path::new(p)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or_default();
                !audio_extensions.contains(&ext)
            })
            .cloned()
            .collect();

        let images = if image_media.is_empty() {
            vec![]
        } else {
            info!(
                "Loading {} media files for LLM: {:?}",
                image_media.len(),
                image_media
            );
            let imgs = load_and_encode_images(&image_media);
            info!("Encoded {} images for LLM", imgs.len());
            imgs
        };

        // Strip [image: ...] and [document: ...] tags from content when media was
        // successfully encoded, since the LLM receives them as content blocks and
        // doesn't need the file paths (which can cause it to try read_file on binary data).
        let content = if images.is_empty() {
            msg_content
        } else {
            strip_document_tags(&strip_image_tags(&msg_content))
        };

        debug!("Acquiring context lock");
        let is_group = msg
            .metadata
            .get(crate::bus::meta::IS_GROUP)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or_default();

        // Refresh provider context (may run external commands with 5s timeout)
        // outside the main lock to avoid blocking other sessions.
        {
            let mut ctx = self.context.lock().await;
            ctx.refresh_provider_context().await;
        }
        let messages = {
            let mut ctx = self.context.lock().await;
            ctx.build_messages(
                &history,
                &content,
                Some(&msg.channel),
                Some(&msg.chat_id),
                Some(&msg.sender_id),
                images,
                is_group,
                None,
            )?
        };
        debug!("Built {} messages, starting agent loop", messages.len());

        let request_id = format!("req-{}", Uuid::new_v4());

        // Complexity-aware routing: score the message and resolve a model override
        let (complexity_score, complexity_band) = if let Some(ref scorer) = self.complexity_scorer {
            let score = scorer.score(&content);
            // Derive band name from thresholds for analytics
            let band = if let Some(ref r) = self.routing
                && let Some(thresholds) = r.chat_thresholds()
            {
                if score.composite >= thresholds.heavy {
                    "heavy"
                } else if score.composite >= thresholds.standard {
                    "standard"
                } else {
                    "light"
                }
            } else {
                "light"
            };
            debug!(
                "complexity score={:.3} band={} forced={:?} request_id={}",
                score.composite, band, score.forced, request_id
            );
            (Some(score), Some(band.to_string()))
        } else {
            (None, None)
        };

        // Resolve complexity to provider overrides
        let complexity_overrides = complexity_score.as_ref().and_then(|score| {
            self.routing
                .as_ref()
                .and_then(|r| r.resolve_chat(score.composite))
                .filter(|o| o.provider.is_some())
        });

        // Extract optional response_format from inbound message metadata (set by
        // the gateway HTTP API when callers request structured JSON output).
        let response_format = msg
            .metadata
            .get(crate::bus::meta::RESPONSE_FORMAT)
            .and_then(crate::gateway::response_format_from_json);

        let mut overrides = match (complexity_overrides, response_format) {
            (Some(cx), Some(rf)) => AgentRunOverrides {
                response_format: Some(rf),
                request_id: Some(request_id.clone()),
                ..cx
            },
            (Some(cx), None) => AgentRunOverrides {
                request_id: Some(request_id.clone()),
                ..cx
            },
            (None, Some(rf)) => AgentRunOverrides {
                response_format: Some(rf),
                request_id: Some(request_id.clone()),
                ..AgentRunOverrides::default()
            },
            (None, None) => AgentRunOverrides {
                request_id: Some(request_id.clone()),
                ..AgentRunOverrides::default()
            },
        };

        // Apply router-derived tool filter and context hint
        if router_tool_filter.is_some() {
            overrides.tool_filter = router_tool_filter;
        }
        if router_context_hint.is_some() {
            overrides.context_hint = router_context_hint;
        }

        // Record complexity event off the async runtime (fire-and-forget)
        if let (Some(score), Some(band)) = (&complexity_score, &complexity_band) {
            let db = self.memory.db();
            let rid = request_id.clone();
            let composite = score.composite;
            let band = band.clone();
            let model_override = overrides.model.clone();
            let forced = score.forced;
            let channel = msg.channel.clone();
            let content_snap = content.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = db.record_complexity_event(
                    &rid,
                    composite,
                    &band,
                    model_override.as_deref(),
                    forced,
                    Some(&channel),
                    &content_snap,
                ) {
                    debug!("failed to record complexity event: {}", e);
                }
            });
        }

        let typing_ctx = Some((msg.channel.clone(), msg.chat_id.clone()));
        let loop_result = self
            .run_agent_loop_with_overrides(messages, typing_ctx, &exec_ctx, &overrides)
            .await?;

        // Extract directives from tool results and update router context
        for (tool_name, meta) in &loop_result.tool_metadata {
            Self::update_router_context(&mut router_context, meta, tool_name);
        }

        // Only reload session if compaction updated it (wrote compaction_summary).
        // Compare the actual checkpoint value, not just presence, so that
        // second+ compaction runs within the same session lifetime are detected.
        let checkpoint_after = self.last_checkpoint.lock().await.clone();
        let compaction_ran = checkpoint_after.is_some() && checkpoint_after != checkpoint_before;
        let mut session = if compaction_ran {
            debug!("compaction updated session, reloading");
            self.sessions.get_or_create(&session_key).await?
        } else {
            session
        };

        // Save router context to session metadata
        router_context.to_session_metadata(&mut session.metadata);

        let extra = HashMap::new();
        // Use the redacted content (msg_content), not the original (msg.content),
        // so that secrets detected by inbound scanning are not persisted to disk.
        session.add_message("user".to_string(), content.clone(), extra.clone());
        // Always save an assistant message to maintain user/assistant alternation.
        // Broken alternation causes the Anthropic provider to merge consecutive user
        // messages, which garbles conversation context for future turns.
        let response_text = loop_result
            .content
            .as_deref()
            .unwrap_or("I wasn't able to generate a response.");
        let mut assistant_extra = HashMap::new();
        if !loop_result.tools_used.is_empty() {
            assistant_extra.insert(
                crate::bus::meta::TOOLS_USED.to_string(),
                Value::Array(
                    loop_result
                        .tools_used
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        if let Some(ref rc) = loop_result.reasoning_content {
            assistant_extra.insert("reasoning_content".to_string(), Value::String(rc.clone()));
        }
        if let Some(ref rs) = loop_result.reasoning_signature {
            assistant_extra.insert("reasoning_signature".to_string(), Value::String(rs.clone()));
        }
        session.add_message(
            "assistant".to_string(),
            response_text.to_string(),
            assistant_extra,
        );
        // Store provider-reported input tokens for precise compaction threshold checks
        if let Some(tokens) = loop_result.input_tokens {
            session.metadata.insert(
                crate::bus::meta::LAST_INPUT_TOKENS.to_string(),
                Value::Number(serde_json::Number::from(tokens)),
            );
        }
        self.sessions.save(&session).await?;

        // Background fact extraction
        if let (Some(compactor), Some(assistant_content)) = (&self.compactor, &loop_result.content)
            && self.compaction_config.extraction_enabled
            && msg.channel != "system"
        {
            let compactor = compactor.clone();
            let memory = self.memory.clone();
            let user_msg = content.clone();
            let assistant_msg = assistant_content.clone();
            let task_tracker = self.task_tracker.clone();
            let task_name = format!("fact_extraction_{}", chrono::Utc::now().timestamp());
            // Use spawn_auto_cleanup since this is a one-off task that should remove itself
            task_tracker
                .spawn_auto_cleanup(task_name, async move {
                    let existing = memory.read_today_section("Facts").unwrap_or_default();
                    match compactor
                        .extract_facts(&user_msg, &assistant_msg, &existing)
                        .await
                    {
                        Ok(facts) => {
                            if !facts.is_empty() {
                                let filtered =
                                    crate::agent::memory::quality::filter_lines(&facts);
                                if filtered.trim().is_empty() {
                                    debug!("fact extraction: all lines filtered by quality gates");
                                } else if let Err(e) =
                                    memory.append_to_section("Facts", &filtered)
                                {
                                    warn!("failed to save facts to daily note: {}", e);
                                } else {
                                    debug!(
                                        "saved extracted facts to daily note ({} bytes, {} filtered)",
                                        filtered.len(),
                                        facts.len().saturating_sub(filtered.len())
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to extract facts from conversation: {}", e);
                        }
                    }
                })
                .await;
        }

        if let Some(content) = loop_result.content {
            // Suppress sending if the LLM returned a [SILENT] response
            if content.starts_with("[SILENT]") {
                debug!("Suppressing silent response");
                return Ok(None);
            }
            Ok(Some(
                OutboundMessage::from_inbound(msg, content)
                    .media(loop_result.media)
                    .merge_metadata(loop_result.response_metadata)
                    .build(),
            ))
        } else {
            warn!(
                "agent loop produced no response for {}:{}",
                msg.channel, msg.chat_id
            );
            Ok(Some(
                OutboundMessage::from_inbound(
                    msg,
                    "I wasn't able to generate a response. Please try again.",
                )
                .build(),
            ))
        }
    }

    async fn process_system_message(&self, msg: InboundMessage) -> Result<Option<OutboundMessage>> {
        info!("Processing system message from {}", msg.sender_id);

        // Inbound secret scanning
        let msg_content = self.leak_detector.redact(&msg.content);
        if msg_content != msg.content {
            warn!("security: secrets detected in system message content — redacting");
        }

        // Prompt guard
        if matches!(
            check_prompt_guard(
                self.prompt_guard.as_ref(),
                &self.prompt_guard_config,
                &msg_content,
                "system message",
            ),
            PromptGuardVerdict::Blocked
        ) {
            return Ok(None);
        }

        let parts: Vec<&str> = msg.chat_id.splitn(2, ':').collect();
        let (origin_channel, origin_chat_id) = if parts.len() == 2 {
            (parts[0].to_string(), parts[1].to_string())
        } else {
            ("cli".to_string(), msg.chat_id.clone())
        };

        let session_key = format!("{origin_channel}:{origin_chat_id}");
        // Lock the target session to prevent concurrent modification.
        // process_message() locks on msg.session_key() which is "system:{chat_id}",
        // but we modify the origin session "{origin_channel}:{origin_chat_id}".
        let target_lock = self.session_lock(&session_key);
        let _target_guard = target_lock.lock().await;
        let session = self.sessions.get_or_create(&session_key).await?;

        let history = self
            .get_compacted_history_timed(&session, &session_key)
            .await?;

        // Refresh provider context outside the main lock to avoid blocking other sessions
        {
            let mut context = self.context.lock().await;
            context.refresh_provider_context().await;
        }
        let messages = {
            let mut context = self.context.lock().await;
            context.build_messages(
                &history,
                &msg_content,
                Some(origin_channel.as_str()),
                Some(origin_chat_id.as_str()),
                None,
                vec![],
                false, // background tasks are not group-scoped
                None,  // no entity context for background tasks
            )?
        };

        let typing_ctx = Some((origin_channel.clone(), origin_chat_id.clone()));
        let context_summary = session
            .metadata
            .get("compaction_summary")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        let exec_ctx = Self::build_execution_context_with_metadata(
            &origin_channel,
            &origin_chat_id,
            context_summary,
            msg.metadata.clone(),
        );
        let request_id = format!("req-{}", Uuid::new_v4());
        let system_overrides = AgentRunOverrides {
            request_id: Some(request_id),
            ..AgentRunOverrides::default()
        };
        let loop_result = self
            .run_agent_loop_with_overrides(messages, typing_ctx, &exec_ctx, &system_overrides)
            .await?;
        let assistant_extra = loop_result.to_assistant_extra();
        let final_content = loop_result
            .content
            .unwrap_or_else(|| "Background task completed.".to_string());

        let mut session = self.sessions.get_or_create(&session_key).await?;
        let extra = HashMap::new();
        session.add_message(
            "user".to_string(),
            format!("[System: {}] {}", msg.sender_id, msg_content),
            extra.clone(),
        );
        session.add_message(
            "assistant".to_string(),
            final_content.clone(),
            assistant_extra,
        );
        self.sessions.save(&session).await?;

        Ok(Some(
            OutboundMessage::builder(
                origin_channel.clone(),
                origin_chat_id.clone(),
                final_content,
            )
            .media(loop_result.media)
            .metadata(msg.metadata)
            .merge_metadata(loop_result.response_metadata)
            .build(),
        ))
    }

    /// Attempt to persist a "remember that..." message directly to memory,
    /// bypassing the LLM. Returns `Ok(Some(response))` if handled, `Ok(None)` if
    /// the caller should fall through to normal LLM processing.
    async fn try_remember_fast_path(
        &self,
        content: &str,
        session_key: &str,
    ) -> Result<Option<String>> {
        use crate::agent::memory::quality::{QualityVerdict, check_quality};
        use crate::agent::memory::remember::is_duplicate_of_entries;

        // Quality gate: reject low-signal content
        let response = match check_quality(content) {
            QualityVerdict::Reject(reason) => {
                info!("remember fast path: rejected ({:?})", reason);
                "That doesn't seem like something worth remembering. Try being more specific."
                    .to_string()
            }
            QualityVerdict::Reframed(reframed) => {
                let recent = self.memory.get_recent_daily_entries(50).unwrap_or_default();
                if is_duplicate_of_entries(&reframed, &recent) {
                    info!("remember fast path: duplicate detected, skipping write");
                    "I already have that noted.".to_string()
                } else {
                    self.memory.append_today(&reframed)?;
                    info!(
                        "remember fast path: wrote {} chars to daily notes (reframed)",
                        reframed.len()
                    );
                    format!("Noted (reframed for accuracy): {reframed}")
                }
            }
            QualityVerdict::Pass => {
                let recent = self.memory.get_recent_daily_entries(50).unwrap_or_default();
                if is_duplicate_of_entries(content, &recent) {
                    info!("remember fast path: duplicate detected, skipping write");
                    "I already have that noted.".to_string()
                } else {
                    self.memory.append_today(content)?;
                    info!(
                        "remember fast path: wrote {} chars to daily notes",
                        content.len()
                    );
                    format!("Noted! I'll remember: {content}")
                }
            }
        };

        // Single session load + save for all branches
        let mut session = self.sessions.get_or_create(session_key).await?;
        let extra = HashMap::new();
        session.add_message(
            "user".to_string(),
            format!("remember that {content}"),
            extra.clone(),
        );
        session.add_message("assistant".to_string(), response.clone(), extra);
        self.sessions.save(&session).await?;

        Ok(Some(response))
    }

    /// Run `get_compacted_history` with timing instrumentation.
    /// Logs a warning when compaction takes more than 2 seconds.
    async fn get_compacted_history_timed(
        &self,
        session: &crate::session::Session,
        session_label: &str,
    ) -> anyhow::Result<Vec<std::collections::HashMap<String, serde_json::Value>>> {
        let start = std::time::Instant::now();
        let history = self.get_compacted_history(session).await?;
        let elapsed = start.elapsed();
        if elapsed > std::time::Duration::from_secs(2) {
            info!(
                "compaction took {:.1}s for session {session_label}",
                elapsed.as_secs_f64()
            );
        }
        Ok(history)
    }

    fn build_execution_context_with_metadata(
        channel: &str,
        chat_id: &str,
        context_summary: Option<String>,
        metadata: HashMap<String, Value>,
    ) -> ExecutionContext {
        ExecutionContext {
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            context_summary,
            metadata,
        }
    }

    /// Execute a tool directly without LLM involvement (buttons, directives,
    /// static rules, config commands, remember fast path).
    #[allow(clippy::too_many_arguments)]
    async fn handle_direct_dispatch(
        &self,
        tool: String,
        params: serde_json::Value,
        source: &crate::router::DispatchSource,
        directive_index: Option<usize>,
        msg: &InboundMessage,
        session_key: &str,
        router_context: &mut crate::router::context::RouterContext,
        context_summary: Option<String>,
    ) -> Result<Option<OutboundMessage>> {
        let source_label = source.label();
        info!(
            "direct dispatch: tool={tool} source={source_label} channel={}",
            msg.channel
        );

        // Inbound secret scanning: redact secrets before they reach tools or
        // get persisted in session history / memory.
        let msg_content = {
            let redacted = self.leak_detector.redact(&msg.content);
            if redacted != msg.content {
                warn!("security: secrets detected in direct dispatch message — redacting");
            }
            redacted
        };

        // Prompt injection preflight check
        if matches!(
            check_prompt_guard(
                self.prompt_guard.as_ref(),
                &self.prompt_guard_config,
                &msg_content,
                "direct dispatch",
            ),
            PromptGuardVerdict::Blocked
        ) {
            return Ok(Some(
                OutboundMessage::from_inbound(msg.clone(), "I can't process this message as it appears to contain prompt injection patterns.").build(),
            ));
        }

        // Handle remember fast path — uses its own session management
        if tool == "_remember" {
            let remember_content =
                crate::agent::memory::remember::extract_remember_content(&msg_content)
                    .unwrap_or_else(|| msg_content.clone());
            let response = if let Ok(Some(text)) = self
                .try_remember_fast_path(&remember_content, session_key)
                .await
            {
                text
            } else {
                warn!("remember fast path failed, returning fallback");
                "I wasn't able to save that. Please try again.".to_string()
            };
            return Ok(Some(
                OutboundMessage::from_inbound(msg.clone(), response).build(),
            ));
        }

        // Validate tool exists
        let Some(tool_ref) = self.tools.get(&tool) else {
            return Ok(Some(
                OutboundMessage::from_inbound(
                    msg.clone(),
                    format!("Action failed: tool '{tool}' is not available."),
                )
                .build(),
            ));
        };

        // Reject approval-required tools in direct dispatch
        if tool_ref.requires_approval() {
            return Ok(Some(
                OutboundMessage::from_inbound(
                    msg.clone(),
                    format!("Action failed: tool '{tool}' requires approval."),
                )
                .build(),
            ));
        }

        // Secret-scan params
        let params = match redact_dispatch_params(&self.leak_detector, params) {
            Ok(p) => p,
            Err(msg_text) => {
                return Ok(Some(
                    OutboundMessage::from_inbound(msg.clone(), msg_text).build(),
                ));
            }
        };

        // Build execution context from message metadata (context_summary was
        // extracted from the session that the caller already loaded)
        let ctx = Self::build_execution_context_with_metadata(
            &msg.channel,
            &msg.chat_id,
            context_summary,
            msg.metadata.clone(),
        );

        // Execute tool
        let result = match self.tools.execute(&tool, params, &ctx).await {
            Ok(r) => r,
            Err(e) => {
                warn!("direct dispatch tool execution failed: {e}");
                let sanitized = crate::utils::path_sanitize::sanitize_error_message(
                    &format!("{e}"),
                    Some(self.workspace.as_path()),
                );
                return Ok(Some(
                    OutboundMessage::from_inbound(
                        msg.clone(),
                        format!("Action failed: {sanitized}"),
                    )
                    .build(),
                ));
            }
        };

        // Secret-scan tool result output
        let result_content = self.leak_detector.redact(&result.content);
        if result_content != result.content {
            warn!(
                "direct dispatch: secrets detected in tool '{}' output — redacting",
                tool
            );
        }

        // Consume single-use directive BEFORE updating context (which may replace
        // the directives vector via install_directives(), invalidating the index)
        if let Some(idx) = directive_index
            && router_context
                .action_directives
                .get(idx)
                .is_some_and(|d| d.single_use)
        {
            router_context.remove_directive_at(idx);
        }

        // Extract directives from result metadata (may replace directives vector)
        if let Some(ref meta) = result.metadata {
            Self::update_router_context(router_context, meta, &tool);
        }

        // Save router context and session history
        let mut session = self.sessions.get_or_create(session_key).await?;
        router_context.to_session_metadata(&mut session.metadata);
        session.add_message(
            "user",
            format!("[action: {tool} via {source_label}]"),
            HashMap::new(),
        );
        session.add_message("assistant", &result_content, HashMap::new());
        if let Err(e) = self.sessions.save(&session).await {
            warn!("failed to save session after direct dispatch: {e}");
        }

        // Build outbound with buttons from tool metadata
        let mut metadata = HashMap::new();
        if let Some(ref meta) = result.metadata
            && let Some(buttons) = meta.get("suggested_buttons")
        {
            metadata.insert(crate::bus::meta::BUTTONS.to_string(), buttons.clone());
        }

        Ok(Some(
            OutboundMessage::builder(msg.channel.clone(), msg.chat_id.clone(), result_content)
                .reply_to(
                    msg.metadata
                        .get(crate::bus::meta::TS)
                        .and_then(|v| v.as_str())
                        .unwrap_or_default(),
                )
                .metadata(metadata)
                .build(),
        ))
    }

    /// Update router context from tool result metadata (`active_tool`, directives).
    fn update_router_context(
        ctx: &mut crate::router::context::RouterContext,
        metadata: &HashMap<String, Value>,
        producing_tool: &str,
    ) {
        if let Some(active) = metadata.get("active_tool").and_then(|v| v.as_str()) {
            if active == producing_tool {
                ctx.set_active_tool(Some(active.to_string()));
            } else {
                warn!(
                    "tool '{}' tried to set active_tool to '{}' — rejected",
                    producing_tool, active
                );
            }
        }
        if let Some(directives_val) = metadata.get("action_directives")
            && let Ok(mut directives) = serde_json::from_value::<
                Vec<crate::router::context::ActionDirective>,
            >(directives_val.clone())
        {
            directives.retain(|d| {
                if d.tool == producing_tool {
                    true
                } else {
                    warn!(
                        "tool '{}' tried to set directive for tool '{}' — rejected",
                        producing_tool, d.tool
                    );
                    false
                }
            });
            ctx.install_directives(directives);
        }
        ctx.updated_at_ms = crate::router::now_ms();
    }

    pub async fn process_direct(
        &self,
        content: &str,
        session_key: &str,
        channel: &str,
        chat_id: &str,
    ) -> Result<String> {
        let result = self
            .process_direct_with_overrides(
                content,
                session_key,
                channel,
                chat_id,
                &AgentRunOverrides::default(),
            )
            .await?;
        Ok(result.content)
    }

    /// Like [`process_direct`](Self::process_direct) but accepts per-invocation
    /// overrides for model and `max_iterations` (used by cron jobs).
    ///
    /// Returns a [`DirectResult`] with both the response text and any metadata
    /// (e.g. interactive buttons) so callers can forward them to channels.
    pub async fn process_direct_with_overrides(
        &self,
        content: &str,
        session_key: &str,
        channel: &str,
        chat_id: &str,
        overrides: &AgentRunOverrides,
    ) -> Result<super::config::DirectResult> {
        // Acquire per-session lock to serialize access to the session being modified.
        let lock_key = session_key.to_string();
        let lock = self.session_lock(&lock_key);
        let _guard = lock.lock().await;

        // Inbound secret scanning for direct calls (cron, subagents)
        let redacted_content: Option<String> = {
            let matches = self.leak_detector.scan(content);
            if matches.is_empty() {
                None
            } else {
                let names: Vec<&str> = matches.iter().map(|m| m.name).collect();
                warn!(
                    "secret detected in direct call to {}/{}: {:?} — redacting",
                    channel, chat_id, names
                );
                Some(self.leak_detector.redact(content))
            }
        };
        let content = redacted_content.as_deref().unwrap_or(content);

        // Prompt injection preflight check
        if matches!(
            check_prompt_guard(
                self.prompt_guard.as_ref(),
                &self.prompt_guard_config,
                content,
                "direct call",
            ),
            PromptGuardVerdict::Blocked
        ) {
            return Ok(super::config::DirectResult {
                content: "I can't process this message as it appears to contain prompt injection patterns.".to_string(),
                metadata: HashMap::new(),
            });
        }

        // Short-circuit for action dispatch (button/webhook/cron with explicit tool call)
        if let Some(ref dispatch) = overrides.action {
            info!(
                "direct call action dispatch: tool={} source={}",
                dispatch.tool,
                dispatch.source.label()
            );

            // Validate tool exists
            let Some(tool_ref) = self.tools.get(&dispatch.tool) else {
                return Ok(super::config::DirectResult {
                    content: format!("Action failed: tool '{}' is not available.", dispatch.tool),
                    metadata: HashMap::new(),
                });
            };

            // Reject approval-required tools in action dispatch
            if tool_ref.requires_approval() {
                return Ok(super::config::DirectResult {
                    content: format!("Action failed: tool '{}' requires approval.", dispatch.tool),
                    metadata: HashMap::new(),
                });
            }

            // Secret-scan params
            let params = match redact_dispatch_params(&self.leak_detector, dispatch.params.clone())
            {
                Ok(p) => p,
                Err(msg_text) => {
                    return Ok(super::config::DirectResult {
                        content: msg_text,
                        metadata: HashMap::new(),
                    });
                }
            };

            let ctx = Self::build_execution_context_with_metadata(
                channel,
                chat_id,
                None,
                overrides.metadata.clone(),
            );
            match self.tools.execute(&dispatch.tool, params, &ctx).await {
                Ok(result) => {
                    // Secret-scan tool result output
                    let result_content = self.leak_detector.redact(&result.content);
                    if result_content != result.content {
                        warn!(
                            "direct call action dispatch: secrets detected in tool '{}' output — redacting",
                            dispatch.tool
                        );
                    }

                    // Save session history
                    let mut session = self.sessions.get_or_create(session_key).await?;
                    session.add_message(
                        "user",
                        format!(
                            "[action: {} via {}]",
                            dispatch.tool,
                            dispatch.source.label()
                        ),
                        HashMap::new(),
                    );
                    session.add_message("assistant", &result_content, HashMap::new());
                    if let Err(e) = self.sessions.save(&session).await {
                        warn!("failed to save session after direct dispatch: {e}");
                    }

                    let mut meta = HashMap::new();
                    if let Some(ref rm) = result.metadata
                        && let Some(buttons) = rm.get("suggested_buttons")
                    {
                        meta.insert(crate::bus::meta::BUTTONS.to_string(), buttons.clone());
                    }
                    return Ok(super::config::DirectResult {
                        content: result_content,
                        metadata: meta,
                    });
                }
                Err(e) => {
                    let sanitized = crate::utils::path_sanitize::sanitize_error_message(
                        &format!("{e}"),
                        Some(self.workspace.as_path()),
                    );
                    return Ok(super::config::DirectResult {
                        content: format!("Action failed: {sanitized}"),
                        metadata: HashMap::new(),
                    });
                }
            }
        }

        let session = self.sessions.get_or_create(session_key).await?;
        let history = self
            .get_compacted_history_timed(&session, session_key)
            .await?;

        // Refresh provider context outside the main lock to avoid blocking other sessions
        {
            let mut ctx = self.context.lock().await;
            ctx.refresh_provider_context().await;
        }
        let messages = {
            let mut ctx = self.context.lock().await;
            ctx.build_messages(
                &history,
                content,
                Some(channel),
                Some(chat_id),
                None,
                vec![],
                false, // process_direct is not group-scoped
                None,  // no entity context for direct processing
            )?
        };

        let typing_ctx = Some((channel.to_string(), chat_id.to_string()));
        let exec_ctx = Self::build_execution_context_with_metadata(
            channel,
            chat_id,
            None,
            overrides.metadata.clone(),
        );
        let request_id = format!("req-{}", Uuid::new_v4());

        let effective_overrides = if overrides.request_id.is_some() {
            overrides.clone()
        } else {
            AgentRunOverrides {
                request_id: Some(request_id),
                ..overrides.clone()
            }
        };

        let loop_result = self
            .run_agent_loop_with_overrides(messages, typing_ctx, &exec_ctx, &effective_overrides)
            .await?;
        let assistant_extra = loop_result.to_assistant_extra();
        let response = loop_result
            .content
            .unwrap_or_else(|| "No response generated.".to_string());

        let mut session = self.sessions.get_or_create(session_key).await?;
        let extra = HashMap::new();
        session.add_message("user".to_string(), content.to_string(), extra.clone());
        session.add_message("assistant".to_string(), response.clone(), assistant_extra);
        self.sessions.save(&session).await?;

        Ok(super::config::DirectResult {
            content: response,
            metadata: loop_result.response_metadata,
        })
    }
}

/// Scan dispatch parameters for leaked secrets and redact them.
///
/// Returns `Ok(params)` (possibly redacted) on success, or `Err(user_message)`
/// when redaction produced invalid JSON.
fn redact_dispatch_params(
    leak_detector: &crate::safety::LeakDetector,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let params_str = serde_json::to_string(&params).unwrap_or_default();
    let redacted = leak_detector.redact(&params_str);
    if redacted == params_str {
        Ok(params)
    } else {
        warn!("security: secrets redacted from dispatch params");
        serde_json::from_str(&redacted).map_err(|e| {
            format!(
                "Action failed: parameters contain secrets that could not be safely redacted ({e})"
            )
        })
    }
}

/// Outcome of a prompt-guard scan.
enum PromptGuardVerdict {
    /// Content is clean (or guard is disabled / warn-only).
    Pass,
    /// Content matched a block-listed pattern - caller should return early.
    Blocked,
}

/// Run the prompt guard (if present) against `content` and log any matches.
///
/// Returns [`PromptGuardVerdict::Blocked`] when the config is set to block and
/// at least one pattern matched; otherwise [`PromptGuardVerdict::Pass`].
fn check_prompt_guard(
    guard: Option<&crate::safety::prompt_guard::PromptGuard>,
    config: &crate::config::PromptGuardConfig,
    content: &str,
    label: &str,
) -> PromptGuardVerdict {
    let Some(guard) = guard else {
        return PromptGuardVerdict::Pass;
    };
    let matches = guard.scan(content);
    if matches.is_empty() {
        return PromptGuardVerdict::Pass;
    }
    for m in &matches {
        warn!(
            "security: prompt injection in {label} ({:?}): {}",
            m.category, m.pattern_name
        );
    }
    if config.should_block() {
        PromptGuardVerdict::Blocked
    } else {
        PromptGuardVerdict::Pass
    }
}
