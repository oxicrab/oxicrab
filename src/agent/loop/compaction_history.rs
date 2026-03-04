use super::{AgentLoop, DEFAULT_HISTORY_SIZE, RECOVERY_CONTEXT_MAX_CHARS};
use crate::agent::compaction::strip_orphaned_tool_messages;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write as _;
use tracing::{debug, warn};

impl AgentLoop {
    pub(super) async fn get_compacted_history(
        &self,
        session: &crate::session::Session,
    ) -> Result<Vec<HashMap<String, Value>>> {
        if self.compactor.is_none() || !self.compaction_config.enabled {
            return Ok(session.get_history(DEFAULT_HISTORY_SIZE));
        }

        let full_history = session.get_full_history();
        if full_history.is_empty() {
            return Ok(vec![]);
        }

        let keep_recent = self.compaction_config.keep_recent;
        let threshold = u64::from(self.compaction_config.threshold_tokens);

        // Prefer provider-reported input tokens (precise), fall back to heuristic
        let token_est = session
            .metadata
            .get("last_input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| {
                crate::agent::compaction::estimate_messages_tokens(&full_history) as u64
            });

        if token_est < threshold {
            return Ok(session.get_history(DEFAULT_HISTORY_SIZE));
        }

        if full_history.len() <= keep_recent {
            return Ok(full_history);
        }

        let old_messages = &full_history[..full_history.len() - keep_recent];
        let recent_messages = &full_history[full_history.len() - keep_recent..];

        if old_messages.is_empty() {
            return Ok(recent_messages.to_vec());
        }

        // Get existing summary from metadata
        let previous_summary = session
            .metadata
            .get("compaction_summary")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        // Extract last user message for recovery context
        let last_user_msg = full_history
            .iter()
            .rev()
            .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
            .and_then(|m| m.get("content").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();

        // Await any in-flight checkpoint task before reading
        if let Some(handle) = self.checkpoint_handle.lock().await.take() {
            let _ = handle.await;
        }
        // Get most recent checkpoint if available
        let checkpoint = self.last_checkpoint.lock().await.clone();
        let cognitive_crumb = self.cognitive_breadcrumb.lock().await.clone();

        // Pre-compaction flush: extract important context before messages are lost
        if self.compaction_config.pre_flush_enabled
            && let Some(ref compactor) = self.compactor
        {
            // Check if we already flushed for this message count to avoid double-flush
            let old_msg_count = old_messages.len();
            let already_flushed = session
                .metadata
                .get("pre_flush_msg_count")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|c| c as usize >= old_msg_count);

            if !already_flushed {
                let mut flushed_content = false;
                match compactor.flush_to_memory(old_messages).await {
                    Ok(ref facts) if !facts.is_empty() => {
                        let filtered = crate::agent::memory::quality::filter_lines(facts);
                        if filtered.trim().is_empty() {
                            debug!("pre-compaction flush: all facts filtered by quality gates");
                        } else if let Err(e) = self
                            .memory
                            .append_to_section("Pre-compaction context", &filtered)
                        {
                            warn!("failed to write pre-compaction flush: {}", e);
                        } else {
                            debug!(
                                "pre-compaction flush: saved {} bytes to daily notes ({} filtered)",
                                filtered.len(),
                                facts.len().saturating_sub(filtered.len())
                            );
                            flushed_content = true;
                        }
                    }
                    Err(e) => {
                        warn!("pre-compaction flush failed (non-fatal): {}", e);
                    }
                    _ => {}
                }
                // Only mark flushed when content was actually persisted, so a
                // retry can attempt extraction again if nothing was saved.
                if flushed_content {
                    match self.sessions.get_or_create(&session.key).await {
                        Ok(mut latest) => {
                            latest.metadata.insert(
                                "pre_flush_msg_count".to_string(),
                                Value::Number(serde_json::Number::from(old_msg_count as u64)),
                            );
                            if let Err(e) = self.sessions.save(&latest).await {
                                warn!("failed to save pre-flush marker: {}", e);
                            }
                        }
                        Err(e) => warn!("failed to reload session for pre-flush marker: {}", e),
                    }
                }
            }
        }

        // Compact old messages
        if let Some(ref compactor) = self.compactor {
            match compactor.compact(old_messages, &previous_summary).await {
                Ok(summary) => {
                    // Build recovery-enriched summary
                    let mut recovery_summary = summary.clone();
                    if let Some(ref cp) = checkpoint {
                        let _ = write!(recovery_summary, "\n\n[Checkpoint] {}", cp);
                    }
                    if let Some(ref crumb) = cognitive_crumb {
                        let _ = write!(recovery_summary, "\n\n{}", crumb);
                    }
                    if !last_user_msg.is_empty() {
                        // Truncate last user message to avoid bloating the summary
                        let truncated_msg: String = last_user_msg
                            .chars()
                            .take(RECOVERY_CONTEXT_MAX_CHARS)
                            .collect();
                        let _ = write!(
                            recovery_summary,
                            "\n\n[Recovery] The conversation was compacted. \
                             Continue from where you left off. Last user request: {}",
                            truncated_msg
                        );
                    }

                    // Cap enriched summary to prevent unbounded growth across compaction cycles
                    if recovery_summary.len() > 2000 {
                        let mut pos = 2000;
                        while pos > 0 && !recovery_summary.is_char_boundary(pos) {
                            pos -= 1;
                        }
                        recovery_summary.truncate(pos);
                    }
                    // Cache summary locally so it survives save failures
                    *self.last_checkpoint.lock().await = Some(recovery_summary.clone());

                    // Persist the enriched summary so the next compaction cycle
                    // builds incrementally on the same context the LLM actually saw
                    // (including checkpoint/recovery annotations).
                    match self.sessions.get_or_create(&session.key).await {
                        Ok(mut latest) => {
                            latest.metadata.insert(
                                "compaction_summary".to_string(),
                                Value::String(recovery_summary.clone()),
                            );
                            if let Err(e) = self.sessions.save(&latest).await {
                                warn!(
                                    "failed to persist compaction summary: {} — next compaction \
                                     may re-summarize the same messages",
                                    e
                                );
                            }
                        }
                        Err(e) => {
                            warn!("failed to reload session for compaction summary: {}", e);
                        }
                    }

                    // Return recovery-enriched summary + recent messages
                    let mut result = vec![HashMap::from([
                        ("role".to_string(), Value::String("system".to_string())),
                        (
                            "content".to_string(),
                            Value::String(format!(
                                "[Previous conversation summary: {}]",
                                recovery_summary
                            )),
                        ),
                    ])];
                    result.extend(recent_messages.iter().cloned());

                    // Strip orphaned tool messages that lost their pair during compaction
                    strip_orphaned_tool_messages(&mut result);

                    Ok(result)
                }
                Err(e) => {
                    if previous_summary.is_empty() {
                        // No previous summary — return full history (oversized but not lost)
                        warn!(
                            "compaction failed with no previous summary: {}, returning full history",
                            e
                        );
                        Ok(full_history)
                    } else {
                        // Reuse the last successful summary rather than losing all context
                        warn!("compaction failed: {}, falling back to previous summary", e);
                        let mut result = vec![HashMap::from([
                            ("role".to_string(), Value::String("system".to_string())),
                            (
                                "content".to_string(),
                                Value::String(format!(
                                    "[Previous conversation summary: {}]",
                                    previous_summary
                                )),
                            ),
                        ])];
                        result.extend(recent_messages.iter().cloned());
                        strip_orphaned_tool_messages(&mut result);
                        Ok(result)
                    }
                }
            }
        } else {
            Ok(recent_messages.to_vec())
        }
    }
}
