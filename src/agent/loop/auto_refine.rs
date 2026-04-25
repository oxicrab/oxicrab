//! Glue between iteration end and `skills::refine`. Fires
//! asynchronously after a turn that loaded ≥1 skill into context AND
//! ran ≥ `min_tool_calls` tool calls. Selects the candidate skill via
//! the same query path the system prompt uses, so a turn's "active
//! skill" matches what was actually shown to the LLM.

use super::AgentLoop;
use crate::providers::base::LLMProvider;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, warn};

impl AgentLoop {
    /// Fire-and-forget refine. No-op when disabled, when too few tool
    /// calls, or when no skill was active for the turn.
    pub(super) fn maybe_spawn_skill_refine(
        &self,
        user_message: &str,
        agent_reply: &str,
        tool_call_count: usize,
    ) {
        if !self.skill_refine_config.enabled {
            return;
        }
        if tool_call_count < self.skill_refine_config.min_tool_calls {
            return;
        }
        let workspace_skills = self.workspace.join("skills");
        if !workspace_skills.exists() {
            return;
        }
        let provider: Arc<dyn LLMProvider> = self.provider.clone();
        let model = self.model.clone();
        let db = self.memory.db();
        let cfg = self.skill_refine_config.clone();
        let user = user_message.to_string();
        let reply = agent_reply.to_string();
        let context = self.context.clone();

        tokio::spawn(async move {
            let candidate = {
                let ctx = context.lock().await;
                ctx.refine_candidate_skill_name(&user)
            };
            let Some(skill_name) = candidate else {
                debug!("auto_refine: no active skill matched the user message");
                return;
            };
            let transcript = format!(
                "USER: {user}\n\nAGENT: {reply}",
                user = user.chars().take(2_000).collect::<String>(),
                reply = reply.chars().take(2_000).collect::<String>(),
            );
            match crate::agent::skills::refine::maybe_refine_skill(
                &db,
                provider.as_ref(),
                &model,
                &workspace_skills,
                &skill_name,
                &transcript,
                &cfg,
            )
            .await
            {
                Ok(Some(outcome)) if outcome.accepted => {
                    debug!(
                        "auto_refine: '{skill_name}' patched (v{} bytes {} → {})",
                        outcome.version_after, outcome.bytes_before, outcome.bytes_after
                    );
                }
                Ok(_) => debug!("auto_refine: '{skill_name}' no patch applied"),
                Err(e) => warn!("auto_refine: '{skill_name}' failed: {e}"),
            }
        });
        let _ = PathBuf::new(); // keep imports tidy
    }
}
