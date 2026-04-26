//! `skill_propose` tool — exposes the staged-skill helpers to the LLM.
//!
//! Action-based tool with four actions:
//! * `propose` — write a candidate skill body to
//!   `<workspace>/skills/staged/<name>.md`. Staged skills do not load
//!   into context. Returns a result that suggests `promote`/`reject`
//!   buttons so an operator can act with one click.
//! * `list_staged` — return current staged proposals.
//! * `promote` — move a staged skill into the active per-skill dir.
//! * `reject` — discard a staged skill without promoting.
//!
//! All four actions are non-read-only and routed through the operator
//! approval workflow when `agents.defaults.approval` covers them.

use crate::agent::memory::MemoryStore;
use crate::agent::skills::index::SkillIndex;
use crate::agent::skills::propose;
use async_trait::async_trait;
use oxicrab_core::actions;
use oxicrab_core::tools::base::{
    ExecutionContext, Tool, ToolCapabilities, ToolCategory, ToolConcurrency, ToolResult,
};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

pub struct SkillProposeTool {
    workspace_skills: PathBuf,
    /// Optional embedding-indexed skill retrieval; when present,
    /// `promote` calls `SkillIndex::index_one` so the freshly-promoted
    /// skill is discoverable on the next turn without waiting for the
    /// next full rebuild.
    skill_index: Option<Arc<SkillIndex>>,
    /// Memory store for embedding access in incremental indexing.
    memory: Option<Arc<MemoryStore>>,
    /// Redacts secrets from proposed skill bodies before they hit
    /// disk. A skill markdown file written from LLM output may echo
    /// a key the model saw earlier in the turn.
    leak_detector: Option<Arc<crate::safety::LeakDetector>>,
}

impl SkillProposeTool {
    pub fn new(workspace_skills: PathBuf) -> Self {
        Self {
            workspace_skills,
            skill_index: None,
            memory: None,
            leak_detector: None,
        }
    }

    #[must_use]
    pub fn with_leak_detector(mut self, detector: Arc<crate::safety::LeakDetector>) -> Self {
        self.leak_detector = Some(detector);
        self
    }

    #[must_use]
    pub fn with_index(mut self, index: Arc<SkillIndex>, memory: Arc<MemoryStore>) -> Self {
        self.skill_index = Some(index);
        self.memory = Some(memory);
        self
    }
}

fn require_str<'a>(params: &'a Value, field: &str) -> Result<&'a str, ToolResult> {
    params
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolResult::error(format!("missing required parameter '{field}'")))
}

fn promote_button(name: &str, style: &str) -> Value {
    json!({
        "id": format!("skill_promote_{name}"),
        "label": format!("Promote {name}"),
        "style": style,
        "context": serde_json::to_string(&json!({
            "tool": "skill_propose",
            "params": {"action": "promote", "name": name}
        })).unwrap_or_default(),
    })
}

fn reject_button(name: &str) -> Value {
    json!({
        "id": format!("skill_reject_{name}"),
        "label": format!("Reject {name}"),
        "style": "danger",
        "context": serde_json::to_string(&json!({
            "tool": "skill_propose",
            "params": {"action": "reject", "name": name}
        })).unwrap_or_default(),
    })
}

#[async_trait]
impl Tool for SkillProposeTool {
    fn name(&self) -> &'static str {
        "skill_propose"
    }

    fn description(&self) -> &'static str {
        "Propose, list, promote, or reject candidate skill files. Proposed skills land in a \
         staged directory and are NOT loaded into context until an operator promotes them. \
         The propose action attaches Promote/Reject buttons so an operator can act with one \
         click. promote/reject are mutating and route through the operator approval workflow."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["propose", "list_staged", "promote", "reject"],
                    "description": "The skill_propose action."
                },
                "name": {
                    "type": "string",
                    "description": "Skill name (alphanumeric + '_-', 1-64 chars). Required for propose/promote/reject."
                },
                "body": {
                    "type": "string",
                    "description": "Markdown body for the skill file (required for propose). May include YAML frontmatter."
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            actions: actions![propose, list_staged: ro, promote, reject],
            category: ToolCategory::Productivity,
            concurrency: ToolConcurrency::SideEffect,
            ..Default::default()
        }
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        let action = match require_str(&params, "action") {
            Ok(a) => a,
            Err(r) => return Ok(r),
        };

        match action {
            "propose" => {
                let name = match require_str(&params, "name") {
                    Ok(n) => n,
                    Err(r) => return Ok(r),
                };
                let body = match require_str(&params, "body") {
                    Ok(b) => b,
                    Err(r) => return Ok(r),
                };
                let body_redacted = self
                    .leak_detector
                    .as_deref()
                    .map_or_else(|| body.to_string(), |d| d.redact(body));
                match propose::propose_skill(&self.workspace_skills, name, &body_redacted) {
                    Ok(path) => {
                        let buttons = vec![promote_button(name, "primary"), reject_button(name)];
                        Ok(ToolResult::new(format!(
                            "Staged '{name}' at {}. Awaiting operator approval to promote.",
                            path.display()
                        ))
                        .with_buttons(buttons))
                    }
                    Err(e) => Ok(ToolResult::error(format!(
                        "skill_propose: failed to stage '{name}': {e}"
                    ))),
                }
            }
            "list_staged" => {
                let entries = propose::list_staged_with_metadata(&self.workspace_skills);
                if entries.is_empty() {
                    return Ok(ToolResult::new("No staged skill proposals.".to_string()));
                }
                let mut lines = vec![format!("{} staged skill(s):", entries.len())];
                for s in &entries {
                    lines.push(format!(
                        "- {} ({} bytes): {}",
                        s.name,
                        s.bytes,
                        if s.description.is_empty() {
                            "(no description)"
                        } else {
                            s.description.as_str()
                        }
                    ));
                }
                let buttons: Vec<Value> = entries
                    .iter()
                    .take(2)
                    .flat_map(|s| [promote_button(&s.name, "primary"), reject_button(&s.name)])
                    .collect();
                Ok(ToolResult::new(lines.join("\n")).with_buttons(buttons))
            }
            "promote" => {
                let name = match require_str(&params, "name") {
                    Ok(n) => n,
                    Err(r) => return Ok(r),
                };
                match propose::promote_staged_skill(&self.workspace_skills, name) {
                    Ok(path) => {
                        // Best-effort incremental index — make the skill
                        // discoverable on the next turn instead of waiting
                        // for the next full rebuild. Silent on failure.
                        let indexed_now = {
                            #[cfg(feature = "embeddings")]
                            {
                                let mut flag = false;
                                if let (Some(idx), Some(mem)) = (&self.skill_index, &self.memory)
                                    && let Some(svc) = mem.embedding_service()
                                {
                                    match idx.index_one(svc, &path) {
                                        Ok(n) if n > 0 => flag = true,
                                        Ok(_) => {}
                                        Err(e) => {
                                            // Promoted skill is on disk but
                                            // not yet searchable — embedding
                                            // service failure rather than a
                                            // benign "no change" return.
                                            tracing::warn!(
                                                "skill_propose: post-promote index_one failed for '{name}': {e}"
                                            );
                                        }
                                    }
                                }
                                flag
                            }
                            #[cfg(not(feature = "embeddings"))]
                            false
                        };
                        let suffix = if indexed_now {
                            " (indexed)".to_string()
                        } else {
                            " — index will pick it up on next rebuild".to_string()
                        };
                        Ok(ToolResult::new(format!(
                            "Promoted '{name}' to {}.{suffix}",
                            path.display()
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!(
                        "skill_propose: failed to promote '{name}': {e}"
                    ))),
                }
            }
            "reject" => {
                let name = match require_str(&params, "name") {
                    Ok(n) => n,
                    Err(r) => return Ok(r),
                };
                match propose::reject_staged_skill(&self.workspace_skills, name) {
                    Ok(()) => Ok(ToolResult::new(format!("Rejected staged '{name}'."))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "skill_propose: failed to reject '{name}': {e}"
                    ))),
                }
            }
            other => Ok(ToolResult::error(format!(
                "skill_propose: unknown action '{other}'"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxicrab_core::tools::base::ExecutionContext;
    use serde_json::json;

    fn ctx() -> ExecutionContext {
        ExecutionContext::default()
    }

    #[tokio::test]
    async fn propose_stages_and_lists() {
        let dir = tempfile::tempdir().unwrap();
        let tool = SkillProposeTool::new(dir.path().to_path_buf());

        let r = tool
            .execute(
                json!({"action": "propose", "name": "demo", "body": "---\nname: demo\ndescription: A demo skill\n---\n\nbody"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!r.is_error, "propose returned error: {}", r.content);
        // Verify suggested_buttons attached.
        let meta = r.metadata.as_ref().unwrap();
        assert!(meta.contains_key("suggested_buttons"));

        let r = tool
            .execute(json!({"action": "list_staged"}), &ctx())
            .await
            .unwrap();
        assert!(!r.is_error);
        assert!(r.content.contains("demo"));
        assert!(r.content.contains("A demo skill"));
    }

    #[tokio::test]
    async fn promote_and_reject_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let tool = SkillProposeTool::new(dir.path().to_path_buf());

        // Stage two.
        for name in ["one", "two"] {
            tool.execute(
                json!({"action": "propose", "name": name, "body": "body"}),
                &ctx(),
            )
            .await
            .unwrap();
        }
        // Promote one.
        let r = tool
            .execute(json!({"action": "promote", "name": "one"}), &ctx())
            .await
            .unwrap();
        assert!(!r.is_error, "promote failed: {}", r.content);
        // Reject the other.
        let r = tool
            .execute(json!({"action": "reject", "name": "two"}), &ctx())
            .await
            .unwrap();
        assert!(!r.is_error, "reject failed: {}", r.content);
        // List should now be empty.
        let r = tool
            .execute(json!({"action": "list_staged"}), &ctx())
            .await
            .unwrap();
        assert!(r.content.contains("No staged"));
    }

    #[tokio::test]
    async fn missing_action_errors() {
        let dir = tempfile::tempdir().unwrap();
        let tool = SkillProposeTool::new(dir.path().to_path_buf());
        let r = tool.execute(json!({}), &ctx()).await.unwrap();
        assert!(r.is_error);
    }

    #[tokio::test]
    async fn unknown_action_errors() {
        let dir = tempfile::tempdir().unwrap();
        let tool = SkillProposeTool::new(dir.path().to_path_buf());
        let r = tool
            .execute(json!({"action": "wat"}), &ctx())
            .await
            .unwrap();
        assert!(r.is_error);
    }
}
