use super::AgentLoop;
use super::config::AgentRunOverrides;
use super::helpers::{
    load_and_encode_images, strip_audio_tags, strip_document_tags, strip_image_tags,
    transcribe_audio_tags,
};
use super::intent;
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
        if let Some(ref tx) = self.typing_tx {
            let _ = tx.send((msg.channel.clone(), msg.chat_id.clone())).await;
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
                let new_matcher = crate::cron::event_matcher::EventMatcher::from_jobs(&jobs);
                if let Some(ref matcher_mutex) = self.event_matcher
                    && let Ok(mut guard) = matcher_mutex.lock()
                {
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
        // Reuse session to avoid repeated lookups
        debug!("Loading session: {}", session_key);
        let session = self.sessions.get_or_create(&session_key).await?;

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
        let history = self.get_compacted_history(&session).await?;
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
        // get persisted in session history / memory.
        let msg_content = {
            let matches = self.leak_detector.scan(&msg_content);
            if matches.is_empty() {
                msg_content
            } else {
                let names: Vec<&str> = matches.iter().map(|m| m.name).collect();
                warn!(
                    "secret detected in inbound message from {}:{}: {:?} — redacting",
                    msg.channel, msg.sender_id, names
                );
                self.leak_detector.redact(&msg_content)
            }
        };

        // Prompt injection preflight check
        if let Some(ref guard) = self.prompt_guard {
            let matches = guard.scan(&msg_content);
            if !matches.is_empty() {
                for m in &matches {
                    warn!(
                        "prompt injection detected ({:?}): {}",
                        m.category, m.pattern_name
                    );
                }
                if self.prompt_guard_config.should_block() {
                    return Ok(Some(OutboundMessage::from_inbound(msg, "I can't process this message as it appears to contain prompt injection patterns.").build()));
                }
            }
        }

        // Remember fast path: bypass LLM for explicit "remember that..." messages
        if let Some(content) =
            crate::agent::memory::remember::extract_remember_content(&msg_content)
        {
            let response = match self.try_remember_fast_path(&content, &session_key).await {
                Ok(resp) => resp,
                Err(e) => {
                    warn!("remember fast path failed, falling through to LLM: {}", e);
                    None
                }
            };
            if let Some(response_text) = response {
                return Ok(Some(
                    OutboundMessage::from_inbound(msg, response_text).build(),
                ));
            }
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

        let messages = {
            let mut ctx = self.context.lock().await;
            ctx.refresh_provider_context().await;
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

        let user_action_intent = self.classify_and_record_intent(&content, Some(&request_id));

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

        let overrides = match (complexity_overrides, response_format) {
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

        // Record complexity event for analytics
        if let (Some(score), Some(band)) = (&complexity_score, &complexity_band)
            && let Err(e) = self.memory.db().record_complexity_event(
                &request_id,
                score.composite,
                band,
                overrides.model.as_deref(),
                score.forced,
                Some(&msg.channel),
                &content,
            )
        {
            debug!("failed to record complexity event: {}", e);
        }

        let typing_ctx = Some((msg.channel.clone(), msg.chat_id.clone()));
        let loop_result = self
            .run_agent_loop_with_overrides(
                messages,
                typing_ctx,
                &exec_ctx,
                &overrides,
                user_action_intent,
            )
            .await?;

        // Reload session in case compaction updated it during the agent loop
        // (compaction saves a compaction_summary to session metadata)
        let mut session = self.sessions.get_or_create(&session_key).await?;
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

        let parts: Vec<&str> = msg.chat_id.splitn(2, ':').collect();
        let (origin_channel, origin_chat_id) = if parts.len() == 2 {
            (parts[0].to_string(), parts[1].to_string())
        } else {
            ("cli".to_string(), msg.chat_id.clone())
        };

        let session_key = format!("{origin_channel}:{origin_chat_id}");
        let session = self.sessions.get_or_create(&session_key).await?;

        let history = self.get_compacted_history(&session).await?;

        let messages = {
            let mut context = self.context.lock().await;
            context.refresh_provider_context().await;
            context.build_messages(
                &history,
                &msg.content,
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
        let user_action_intent = self.classify_and_record_intent(&msg.content, Some(&request_id));
        let system_overrides = AgentRunOverrides {
            request_id: Some(request_id),
            ..AgentRunOverrides::default()
        };
        let loop_result = self
            .run_agent_loop_with_overrides(
                messages,
                typing_ctx,
                &exec_ctx,
                &system_overrides,
                user_action_intent,
            )
            .await?;
        let final_content = loop_result
            .content
            .unwrap_or_else(|| "Background task completed.".to_string());

        let mut session = self.sessions.get_or_create(&session_key).await?;
        let extra = HashMap::new();
        session.add_message(
            "user".to_string(),
            format!("[System: {}] {}", msg.sender_id, msg.content),
            extra.clone(),
        );
        let mut assistant_extra = HashMap::new();
        if !loop_result.tools_used.is_empty() {
            assistant_extra.insert(
                crate::bus::meta::TOOLS_USED.to_string(),
                Value::Array(
                    loop_result
                        .tools_used
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
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

    /// Classify user message intent and record the metric to the database.
    /// Returns `true` if the message has action intent (should trigger tool use).
    fn classify_and_record_intent(&self, content: &str, request_id: Option<&str>) -> bool {
        let regex_intent = intent::classify_action_intent(content);
        let (semantic_result, semantic_score) = if regex_intent {
            (None, None)
        } else {
            #[cfg(feature = "embeddings")]
            {
                self.memory
                    .embedding_service()
                    .and_then(|svc| intent::classify_action_intent_semantic(content, svc))
                    .map_or((None, None), |(result, score)| (Some(result), Some(score)))
            }
            #[cfg(not(feature = "embeddings"))]
            (None, None)
        };
        let user_action_intent = regex_intent || semantic_result.unwrap_or_default();

        let intent_method = if regex_intent {
            "regex"
        } else if semantic_result == Some(true) {
            "semantic"
        } else {
            "none"
        };
        if let Err(e) = self.memory.db().record_intent_event(
            "classification",
            Some(intent_method),
            semantic_score,
            None,
            content,
            request_id,
        ) {
            debug!("failed to record intent metric: {}", e);
        }

        user_action_intent
    }

    fn build_execution_context(
        channel: &str,
        chat_id: &str,
        context_summary: Option<String>,
    ) -> ExecutionContext {
        ExecutionContext {
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            context_summary,
            ..Default::default()
        }
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

    pub async fn process_direct(
        &self,
        content: &str,
        session_key: &str,
        channel: &str,
        chat_id: &str,
    ) -> Result<String> {
        self.process_direct_with_overrides(
            content,
            session_key,
            channel,
            chat_id,
            &AgentRunOverrides::default(),
        )
        .await
    }

    /// Like [`process_direct`](Self::process_direct) but accepts per-invocation
    /// overrides for model and `max_iterations` (used by cron jobs).
    pub async fn process_direct_with_overrides(
        &self,
        content: &str,
        session_key: &str,
        channel: &str,
        chat_id: &str,
        overrides: &AgentRunOverrides,
    ) -> Result<String> {
        // Acquire per-session lock to prevent concurrent processing within the same session
        let lock_key = format!("{channel}:{chat_id}");
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
        if let Some(ref guard) = self.prompt_guard {
            let matches = guard.scan(content);
            if !matches.is_empty() {
                for m in &matches {
                    warn!(
                        "prompt injection detected in direct call ({:?}): {}",
                        m.category, m.pattern_name
                    );
                }
                if self.prompt_guard_config.should_block() {
                    return Ok(
                        "I can't process this message as it appears to contain prompt injection patterns."
                            .to_string(),
                    );
                }
            }
        }

        let session = self.sessions.get_or_create(session_key).await?;
        let history = self.get_compacted_history(&session).await?;

        let messages = {
            let mut ctx = self.context.lock().await;
            ctx.refresh_provider_context().await;
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
        let exec_ctx = Self::build_execution_context(channel, chat_id, None);
        let request_id = format!("req-{}", Uuid::new_v4());
        let user_action_intent = self.classify_and_record_intent(content, Some(&request_id));

        let effective_overrides = if overrides.request_id.is_some() {
            overrides.clone()
        } else {
            AgentRunOverrides {
                request_id: Some(request_id),
                ..overrides.clone()
            }
        };

        let loop_result = self
            .run_agent_loop_with_overrides(
                messages,
                typing_ctx,
                &exec_ctx,
                &effective_overrides,
                user_action_intent,
            )
            .await?;
        let response = loop_result
            .content
            .unwrap_or_else(|| "No response generated.".to_string());

        let mut session = self.sessions.get_or_create(session_key).await?;
        let extra = HashMap::new();
        session.add_message("user".to_string(), content.to_string(), extra.clone());
        let mut assistant_extra = HashMap::new();
        if !loop_result.tools_used.is_empty() {
            assistant_extra.insert(
                crate::bus::meta::TOOLS_USED.to_string(),
                Value::Array(
                    loop_result
                        .tools_used
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        session.add_message("assistant".to_string(), response.clone(), assistant_extra);
        self.sessions.save(&session).await?;

        Ok(response)
    }
}
