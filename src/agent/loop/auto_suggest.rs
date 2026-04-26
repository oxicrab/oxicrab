//! Glue between the trajectory store and `propose_skill`. Fires
//! asynchronously after a turn end when
//! `agents.defaults.trajectory.autoSuggest.enabled = true` AND a
//! repeating cross-session sequence clears the threshold. Stages (does
//! NOT promote) the skill; an operator reviews + promotes via the
//! existing `skill_propose` tool.
//!
//! When `auto_suggest.useLlmBody = true`, a small LLM call generates
//! a purpose-written skill body instead of the fixed template. Mirrors
//! `OpenCrust`'s auto-skill writer.

use super::AgentLoop;
use crate::agent::trajectory::skill_suggester::{
    SkillCandidate, default_skill_body, find_candidates, name_from_fingerprint, pick_top_uncovered,
};
use crate::providers::base::{ChatRequest, LLMProvider, Message};
use std::sync::Arc;
use tracing::{debug, info, warn};

impl AgentLoop {
    /// Fire-and-forget: schedule a background scan for repeating
    /// cross-session tool sequences and stage the top match. No-op
    /// when disabled or when no `session_key` is available.
    pub(super) fn maybe_spawn_skill_auto_suggest(&self, session_key: Option<&str>) {
        if !self.trajectory_config.enabled {
            return;
        }
        if !self.trajectory_config.auto_suggest.enabled {
            return;
        }
        let _ = session_key; // present for future per-session scoping
        let db = self.memory.db();
        let workspace = self.workspace.clone();
        let cfg = self.trajectory_config.auto_suggest.clone();
        let provider = self.provider.clone();
        let model = self.model.clone();
        tokio::spawn(async move { run_scan(&db, &workspace, &cfg, provider, &model).await });
    }
}

async fn run_scan(
    db: &Arc<crate::agent::memory::memory_db::MemoryDB>,
    workspace: &std::path::Path,
    cfg: &crate::config::TrajectoryAutoSuggestConfig,
    provider: Arc<dyn LLMProvider>,
    model: &str,
) {
    let candidates = match find_candidates(
        db,
        cfg.min_occurrences,
        cfg.min_sequence_length,
        cfg.max_sequence_steps,
    ) {
        Ok(c) => c,
        Err(e) => {
            warn!("auto_suggest: candidate query failed: {e}");
            return;
        }
    };
    let Some(pick) = pick_top_uncovered(candidates, cfg.min_occurrences) else {
        debug!(
            "auto_suggest: no uncovered candidate ≥ {}",
            cfg.min_occurrences
        );
        return;
    };
    if let Err(e) = stage_candidate(workspace, &pick, cfg, provider.as_ref(), model).await {
        warn!("auto_suggest: failed to stage candidate: {e}");
    }
}

async fn stage_candidate(
    workspace: &std::path::Path,
    pick: &SkillCandidate,
    cfg: &crate::config::TrajectoryAutoSuggestConfig,
    provider: &dyn LLMProvider,
    model: &str,
) -> anyhow::Result<()> {
    let name = name_from_fingerprint(&pick.fingerprint);
    let body = if cfg.use_llm_body {
        match generate_llm_body(pick, provider, model).await {
            Ok(b) if !b.trim().is_empty() => b,
            Ok(_) => {
                warn!("auto_suggest: LLM body was empty, falling back to template");
                default_skill_body(pick)
            }
            Err(e) => {
                warn!("auto_suggest: LLM body generation failed ({e}), using template");
                default_skill_body(pick)
            }
        }
    } else {
        default_skill_body(pick)
    };
    let workspace_skills = workspace.join("skills");
    let path = crate::agent::skills::propose::propose_skill(&workspace_skills, &name, &body)?;
    info!(
        "auto_suggest: staged skill '{name}' at {} ({} occurrences)",
        path.display(),
        pick.occurrences
    );
    Ok(())
}

/// Ask a small LLM to write a focused skill body for the candidate
/// workflow. Constrained: ≤500 words, must include "When to use"
/// and "Tool sequence" sections, no preamble.
async fn generate_llm_body(
    pick: &SkillCandidate,
    provider: &dyn LLMProvider,
    model: &str,
) -> anyhow::Result<String> {
    let steps_list: String = pick
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| format!("  {}. {s}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "I detected this tool sequence repeating across {n} sessions:\n\n{steps}\n\n\
         Write a concise skill markdown file (≤500 words) that an AI agent will load \
         when the user describes a task matching this workflow. Output only the \
         markdown — no preamble, no code fences. Use this structure:\n\n\
         # <descriptive title>\n\n\
         <one-paragraph description of what this skill does and what kind of user request triggers it>\n\n\
         ## When to use\n\n\
         <bullet list of trigger phrases or scenarios>\n\n\
         ## Tool sequence\n\n\
         <numbered list of tool calls in order with brief rationale per step>\n\n\
         ## Notes\n\n\
         <any caveats, error-recovery hints, or auth requirements>\n",
        n = pick.occurrences,
        steps = steps_list,
    );
    let req = ChatRequest {
        model: Some(model.to_string()),
        messages: vec![Message::user(prompt)],
        temperature: Some(0.3),
        max_tokens: 1200,
        ..Default::default()
    };
    let resp = provider.chat(&req).await?;
    Ok(resp.content.unwrap_or_default().trim().to_string())
}
