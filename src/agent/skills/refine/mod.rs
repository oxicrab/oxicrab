//! Skill auto-refine — patches existing skills based on session
//! learnings via a two-round confidence-gated refinement. The signal:
//! a skill was loaded into context, the agent ran ≥ N tool calls, and
//! the LLM thinks the skill body could be tightened to match what just
//! happened.
//!
//! Round 1 asks for a JSON assessment with `should_patch`,
//! `confidence`, and `reason`. Round 2 only fires when confidence
//! clears the configured threshold and produces the new body.
//!
//! Patches are written atomically (`tmp` + rename). A `{name}-CHANGELOG.md`
//! sidecar records every accepted patch so an operator can audit
//! changes. The version field uses `1.{N}.0` where N = count of
//! accepted refinements (read from the `skill_refinements` table).

use crate::agent::memory::memory_db::{MemoryDB, SkillRefinementRecord};
use crate::providers::base::{ChatRequest, LLMProvider, Message};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info, warn};

#[cfg(test)]
mod tests;

/// One patch attempt. `accepted` is the gate decision; the rest is
/// audit metadata persisted to `skill_refinements` on accept.
#[derive(Debug, Clone)]
pub struct RefineOutcome {
    pub accepted: bool,
    pub confidence: f32,
    pub reason: String,
    pub bytes_before: usize,
    pub bytes_after: usize,
    pub version_after: String,
}

#[derive(Deserialize)]
struct RoundOneResponse {
    should_patch: bool,
    confidence: f32,
    #[serde(default)]
    reason: String,
}

#[derive(Deserialize)]
struct RoundTwoResponse {
    body: String,
}

/// Resolve the path of a skill given its name. Mirrors the layout
/// used by `propose.rs` (`<workspace_skills>/<name>.md` for flat skills,
/// `<workspace_skills>/<name>/SKILL.md` for folder-format).
pub fn skill_file_path(workspace_skills: &Path, name: &str) -> Option<PathBuf> {
    let flat = workspace_skills.join(format!("{name}.md"));
    if flat.exists() {
        return Some(flat);
    }
    let folder = workspace_skills.join(name).join("SKILL.md");
    if folder.exists() {
        return Some(folder);
    }
    None
}

/// Path of the changelog sidecar — stored alongside the skill file
/// (`{stem}-CHANGELOG.md`) so it never collides with the skill body.
pub fn changelog_path(skill_path: &Path) -> PathBuf {
    let parent = skill_path.parent().unwrap_or(Path::new(""));
    let stem = skill_path
        .file_stem()
        .map_or_else(|| "skill".to_string(), |s| s.to_string_lossy().into_owned());
    parent.join(format!("{stem}-CHANGELOG.md"))
}

/// Try to refine `skill_name`. Returns `Ok(Some(outcome))` when the
/// skill exists and a refinement attempt completed; `Ok(None)` when
/// the file is missing. Errors are reserved for LLM/IO failures.
///
/// `transcript` should be a compact rendering of the just-completed
/// turn — last user message + agent reply summary. Keep it short
/// (≤ 4KB) since both rounds embed it in the prompt.
#[allow(clippy::too_many_arguments)]
pub async fn maybe_refine_skill(
    db: &Arc<MemoryDB>,
    provider: &dyn LLMProvider,
    model: &str,
    workspace_skills: &Path,
    skill_name: &str,
    transcript: &str,
    config: &crate::config::SkillRefineConfig,
) -> Result<Option<RefineOutcome>> {
    let Some(path) = skill_file_path(workspace_skills, skill_name) else {
        debug!("refine: skill '{skill_name}' not found on disk");
        return Ok(None);
    };
    let body_before =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let bytes_before = body_before.len();

    let recent_changelog = read_recent_changelog(&path);
    let assessment = round_one_assessment(
        provider,
        model,
        skill_name,
        &body_before,
        transcript,
        &recent_changelog,
        config,
    )
    .await?;
    if !assessment.should_patch {
        debug!(
            "refine: '{skill_name}' should_patch=false confidence={}",
            assessment.confidence
        );
        return Ok(Some(RefineOutcome {
            accepted: false,
            confidence: assessment.confidence,
            reason: assessment.reason,
            bytes_before,
            bytes_after: bytes_before,
            version_after: "n/a".to_string(),
        }));
    }
    if assessment.confidence < config.confidence_threshold {
        debug!(
            "refine: '{skill_name}' confidence {} < threshold {}",
            assessment.confidence, config.confidence_threshold
        );
        return Ok(Some(RefineOutcome {
            accepted: false,
            confidence: assessment.confidence,
            reason: assessment.reason,
            bytes_before,
            bytes_after: bytes_before,
            version_after: "n/a".to_string(),
        }));
    }

    let new_body = round_two_patch(
        provider,
        model,
        skill_name,
        &body_before,
        transcript,
        &assessment.reason,
        config,
    )
    .await?;
    // Skip no-op patches: round-2 occasionally returns the original
    // body unchanged. Writing the same bytes pollutes the audit
    // trail (changelog row + version bump for nothing).
    if new_body.body == body_before {
        info!("refine: round-2 returned unchanged body, skipping patch for '{skill_name}'");
        return Ok(Some(RefineOutcome {
            accepted: false,
            confidence: assessment.confidence,
            reason: format!("noop: {}", assessment.reason),
            bytes_before,
            bytes_after: bytes_before,
            version_after: "n/a".to_string(),
        }));
    }
    let bytes_after = new_body.body.len();
    let n = db.count_skill_refinements(skill_name).unwrap_or(0);
    let version_after = format!("1.{}.0", n + 1);
    apply_patch(&path, &new_body.body, &assessment.reason, &version_after)?;
    let _ = db.insert_skill_refinement(&SkillRefinementRecord {
        skill_name: skill_name.to_string(),
        confidence: f64::from(assessment.confidence),
        reason: assessment.reason.clone(),
        bytes_before: bytes_before as i64,
        bytes_after: bytes_after as i64,
        version_after: version_after.clone(),
        created_at_ms: chrono::Utc::now().timestamp_millis(),
    });
    info!(
        "refine: patched '{skill_name}' v{version_after} ({} → {} bytes, conf {})",
        bytes_before, bytes_after, assessment.confidence
    );
    Ok(Some(RefineOutcome {
        accepted: true,
        confidence: assessment.confidence,
        reason: assessment.reason,
        bytes_before,
        bytes_after,
        version_after,
    }))
}

async fn round_one_assessment(
    provider: &dyn LLMProvider,
    model: &str,
    skill_name: &str,
    body: &str,
    transcript: &str,
    recent_changelog: &str,
    config: &crate::config::SkillRefineConfig,
) -> Result<RoundOneResponse> {
    let body_excerpt = excerpt(body, 4_000);
    let transcript_excerpt = excerpt(transcript, 2_000);
    let changelog_excerpt = excerpt(recent_changelog, 1_000);
    let prompt = format!(
        "You are reviewing a skill file used by an agent. Decide whether the skill body could be \
         tightened or expanded based on what just happened. Respond with valid JSON only.\n\n\
         Skill name: {skill_name}\n\n\
         Recent changes already addressed (do NOT propose patches that duplicate these):\n```\n{changelog_excerpt}\n```\n\n\
         Skill body:\n```\n{body_excerpt}\n```\n\n\
         Just-completed session transcript:\n```\n{transcript_excerpt}\n```\n\n\
         JSON schema (return only this object, no commentary):\n\
         {{\"should_patch\": bool, \"confidence\": number 0..1, \"reason\": string ≤200 chars}}\n"
    );
    let req = ChatRequest {
        model: Some(model.to_string()),
        messages: vec![Message::user(prompt)],
        temperature: Some(0.0),
        max_tokens: config.max_tokens.min(400),
        ..Default::default()
    };
    // Round-1 occasionally emits malformed JSON (commentary, fenced
    // block, trailing text). Retry once with the same prompt before
    // giving up — the second attempt usually parses.
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..2 {
        let resp = provider
            .chat(&req)
            .await
            .context("refine round 1 LLM call")?;
        let text = resp.content.unwrap_or_default();
        match parse_json::<RoundOneResponse>(&text) {
            Ok(parsed) => return Ok(parsed),
            Err(e) => {
                debug!(
                    "refine round 1 parse failed on attempt {}: {e}",
                    attempt + 1
                );
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("refine round 1 parse failed twice")))
}

async fn round_two_patch(
    provider: &dyn LLMProvider,
    model: &str,
    skill_name: &str,
    body: &str,
    transcript: &str,
    reason: &str,
    config: &crate::config::SkillRefineConfig,
) -> Result<RoundTwoResponse> {
    let body_excerpt = excerpt(body, 4_000);
    let transcript_excerpt = excerpt(transcript, 2_000);
    let prompt = format!(
        "Rewrite the skill body to incorporate the improvement noted below. Keep the same intent; \
         only change what the reason calls out. Respond with valid JSON only.\n\n\
         Skill name: {skill_name}\n\n\
         Reason for patch: {reason}\n\n\
         Current skill body:\n```\n{body_excerpt}\n```\n\n\
         Just-completed session transcript:\n```\n{transcript_excerpt}\n```\n\n\
         JSON schema (return only this object): {{\"body\": string (the new full skill body)}}\n"
    );
    let req = ChatRequest {
        model: Some(model.to_string()),
        messages: vec![Message::user(prompt)],
        temperature: Some(0.2),
        max_tokens: config.max_tokens,
        ..Default::default()
    };
    let resp = provider
        .chat(&req)
        .await
        .context("refine round 2 LLM call")?;
    let text = resp.content.unwrap_or_default();
    parse_json::<RoundTwoResponse>(&text).context("refine round 2 parse")
}

/// Lenient JSON extraction: tolerates leading/trailing prose and
/// markdown code fences. Picks the first balanced `{...}` block.
fn parse_json<T: for<'a> Deserialize<'a>>(text: &str) -> Result<T> {
    let stripped = strip_code_fence(text.trim());
    if let Ok(v) = serde_json::from_str::<T>(stripped) {
        return Ok(v);
    }
    if let (Some(start), Some(end)) = (stripped.find('{'), stripped.rfind('}'))
        && end > start
    {
        let candidate = &stripped[start..=end];
        return Ok(serde_json::from_str::<T>(candidate)?);
    }
    Err(anyhow::anyhow!("no JSON object found in response"))
}

fn strip_code_fence(s: &str) -> &str {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```json") {
        return rest.trim_end_matches("```").trim();
    }
    if let Some(rest) = t.strip_prefix("```") {
        return rest.trim_end_matches("```").trim();
    }
    t
}

fn excerpt(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Read the last 8 KB of the sidecar changelog so the assessment
/// prompt can avoid re-proposing already-addressed patches.
fn read_recent_changelog(skill_path: &Path) -> String {
    let path = changelog_path(skill_path);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return String::new();
    };
    excerpt(&raw, 8_000)
}

/// Atomically rewrite `skill_path` with `new_body` and append a
/// dated CHANGELOG entry next to it. The temp+rename guards against
/// crashes mid-write.
fn apply_patch(skill_path: &Path, new_body: &str, reason: &str, version: &str) -> Result<()> {
    let parent = skill_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("skill path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}-tmp",
        skill_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("skill")
    ));
    std::fs::write(&tmp, new_body)?;
    std::fs::rename(&tmp, skill_path)?;

    let cl_path = changelog_path(skill_path);
    let entry = format!(
        "## v{version} — {date}\n\n{reason}\n\n",
        version = version,
        date = chrono::Utc::now().format("%Y-%m-%d"),
    );
    let existing = std::fs::read_to_string(&cl_path).unwrap_or_default();
    let new_changelog = if existing.is_empty() {
        format!("# Changelog\n\n{entry}")
    } else {
        format!("{existing}{entry}")
    };
    if let Err(e) = std::fs::write(&cl_path, new_changelog) {
        warn!("refine: failed to update {}: {e}", cl_path.display());
    }
    Ok(())
}
