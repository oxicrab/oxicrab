//! Glue between the trajectory store and `propose_skill`. Fires
//! asynchronously after a turn end when
//! `agents.defaults.trajectory.autoSuggest.enabled = true` AND a
//! repeating cross-session sequence clears the threshold. Stages (does
//! NOT promote) the skill; an operator reviews + promotes via the
//! existing `skill_propose` tool.

use super::AgentLoop;
use crate::agent::trajectory::skill_suggester::{
    SkillCandidate, default_skill_body, find_candidates, name_from_fingerprint, pick_top_uncovered,
};
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
        tokio::task::spawn_blocking(move || run_scan(&db, &workspace, &cfg));
    }
}

fn run_scan(
    db: &Arc<crate::agent::memory::memory_db::MemoryDB>,
    workspace: &std::path::Path,
    cfg: &crate::config::TrajectoryAutoSuggestConfig,
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
    if let Err(e) = stage_candidate(workspace, &pick) {
        warn!("auto_suggest: failed to stage candidate: {e}");
    }
}

fn stage_candidate(workspace: &std::path::Path, pick: &SkillCandidate) -> anyhow::Result<()> {
    let name = name_from_fingerprint(&pick.fingerprint);
    let body = default_skill_body(pick);
    let workspace_skills = workspace.join("skills");
    let path = crate::agent::skills::propose::propose_skill(&workspace_skills, &name, &body)?;
    info!(
        "auto_suggest: staged skill '{name}' at {} ({} occurrences)",
        path.display(),
        pick.occurrences
    );
    Ok(())
}
