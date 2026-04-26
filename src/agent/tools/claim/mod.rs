//! `claim` tool — exposes the structured claims store to the LLM.
//!
//! Five actions:
//!
//! - `add` — assert a claim with text + confidence + optional evidence
//! - `list` — list claims, optionally filtered by status
//! - `get` — fetch one claim by id (with evidence)
//! - `lint` — run all three lint passes (contradictions, stale, orphan)
//!   and return a markdown report
//! - `update_status` — accept / retract / mark contradicted

use crate::agent::memory::memory_db::{ClaimStatus, EvidencePointer, MemoryDB};
use async_trait::async_trait;
use oxicrab_core::actions;
use oxicrab_core::tools::base::{
    ExecutionContext, Tool, ToolCapabilities, ToolCategory, ToolConcurrency, ToolResult,
};
use serde_json::{Value, json};
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// Default confidence when the LLM doesn't supply one. Mid-band — the
/// claim is open for re-observation but not asserted as fact.
const DEFAULT_CONFIDENCE: f32 = 0.6;
/// Bar for the stale-low-confidence lint pass.
const LINT_STALE_CONFIDENCE_MAX: f32 = 0.4;
/// Last-seen window (days) for the stale-low-confidence pass.
const LINT_STALE_CUTOFF_DAYS: u32 = 90;
/// Token-overlap threshold for the contradiction pass.
const LINT_CONTRADICTION_MIN_SHARED_TOKENS: usize = 2;

pub struct ClaimTool {
    db: Arc<MemoryDB>,
}

impl ClaimTool {
    pub fn new(db: Arc<MemoryDB>) -> Self {
        Self { db }
    }

    fn parse_evidence(value: Option<&Value>) -> Vec<EvidencePointer> {
        let Some(v) = value else { return Vec::new() };
        let Some(arr) = v.as_array() else {
            return Vec::new();
        };
        arr.iter()
            .filter_map(|item| {
                let kind = item.get("kind").and_then(Value::as_str)?.to_string();
                let value = item.get("value").and_then(Value::as_str)?.to_string();
                Some(EvidencePointer { kind, value })
            })
            .collect()
    }

    fn render_lint_report(
        contradictions: &[crate::agent::memory::memory_db::ContradictionPair],
        stale: &[crate::agent::memory::memory_db::Claim],
        orphans: &[crate::agent::memory::memory_db::Claim],
        counts: &[(ClaimStatus, u32)],
    ) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "# claim_lint report\n");
        let _ = writeln!(out, "## Counts");
        for (status, n) in counts {
            let _ = writeln!(out, "- {}: {}", status.as_str(), n);
        }
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "## Potential contradictions ({} pair(s))",
            contradictions.len()
        );
        if contradictions.is_empty() {
            let _ = writeln!(out, "_None._\n");
        } else {
            for p in contradictions {
                let _ = writeln!(
                    out,
                    "- #{}/#{}: \"{}\" vs \"{}\" (conf {:.2}/{:.2})",
                    p.a_id, p.b_id, p.a_text, p.b_text, p.a_confidence, p.b_confidence
                );
            }
            let _ = writeln!(out);
        }
        let _ = writeln!(
            out,
            "## Stale low-confidence (≤ {LINT_STALE_CONFIDENCE_MAX}, > {LINT_STALE_CUTOFF_DAYS}d) ({})",
            stale.len()
        );
        if stale.is_empty() {
            let _ = writeln!(out, "_None._\n");
        } else {
            for c in stale {
                let _ = writeln!(
                    out,
                    "- #{}: \"{}\" (conf {:.2})",
                    c.id, c.text, c.confidence
                );
            }
            let _ = writeln!(out);
        }
        let _ = writeln!(out, "## Orphans (no evidence) ({})", orphans.len());
        if orphans.is_empty() {
            let _ = writeln!(out, "_None._");
        } else {
            for c in orphans {
                let _ = writeln!(out, "- #{}: \"{}\"", c.id, c.text);
            }
        }
        out
    }
}

#[async_trait]
impl Tool for ClaimTool {
    fn name(&self) -> &'static str {
        "claim"
    }

    fn description(&self) -> &'static str {
        "Manage structured claims about the user, project, or world. Each claim has text, \
         confidence (0.0-1.0), status (open/accepted/retracted/contradicted), and optional \
         evidence pointers. Use this when you need to record a fact you'll re-quote later — \
         the structure prevents low-confidence hedges from rounding to fact, and the lint \
         action surfaces contradictions before they accumulate."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "get", "lint", "update_status"],
                    "description": "Which claim operation to perform."
                },
                "text": {
                    "type": "string",
                    "description": "Required for add: the claim's text."
                },
                "confidence": {
                    "type": "number",
                    "minimum": 0.0,
                    "maximum": 1.0,
                    "description": "Confidence on 0..1. Hedges (\"I think\", \"maybe\") should land ≤ 0.5; explicit assertions ≥ 0.8. Defaults to 0.6."
                },
                "evidence": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "kind": {"type": "string", "description": "message | file | tool | other"},
                            "value": {"type": "string", "description": "Pointer (e.g. 'telegram:msg-1234', '/path/to/file.md')"}
                        },
                        "required": ["kind", "value"]
                    },
                    "description": "Optional list of evidence pointers."
                },
                "id": {
                    "type": "integer",
                    "description": "Required for get / update_status."
                },
                "status": {
                    "type": "string",
                    "enum": ["open", "accepted", "retracted", "contradicted"],
                    "description": "Required for update_status."
                },
                "filter_status": {
                    "type": "string",
                    "enum": ["open", "accepted", "retracted", "contradicted"],
                    "description": "Optional filter for the list action."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 200,
                    "description": "Optional limit for list (default 50)."
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            actions: actions![
                add,
                list: ro,
                get,
                lint: ro,
                update_status
            ],
            category: ToolCategory::Productivity,
            concurrency: ToolConcurrency::SideEffect,
            ..Default::default()
        }
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        let Some(action) = params.get("action").and_then(Value::as_str) else {
            return Ok(ToolResult::error("claim: missing 'action'".to_string()));
        };

        match action {
            "add" => {
                let Some(text) = params.get("text").and_then(Value::as_str) else {
                    return Ok(ToolResult::error("claim add: missing 'text'".to_string()));
                };
                let confidence = params
                    .get("confidence")
                    .and_then(serde_json::Value::as_f64)
                    .map_or(DEFAULT_CONFIDENCE, |c| c as f32);
                let evidence = Self::parse_evidence(params.get("evidence"));
                match self
                    .db
                    .insert_claim(text, confidence, ClaimStatus::Open, &evidence)
                {
                    Ok(id) => Ok(ToolResult::new(format!(
                        "Recorded claim #{id} (confidence {:.2}, {} evidence pointer(s))",
                        confidence,
                        evidence.len()
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("claim add failed: {e}"))),
                }
            }
            "list" => {
                let filter = params
                    .get("filter_status")
                    .and_then(Value::as_str)
                    .and_then(ClaimStatus::parse);
                let limit = params
                    .get("limit")
                    .and_then(serde_json::Value::as_u64)
                    .map_or(50, |n| (n as usize).clamp(1, 200));
                match self.db.list_claims(filter, limit) {
                    Ok(claims) if claims.is_empty() => {
                        Ok(ToolResult::new("No claims recorded.".to_string()))
                    }
                    Ok(claims) => {
                        use std::fmt::Write as _;
                        let mut out = format!("{} claim(s):\n", claims.len());
                        for c in claims {
                            let _ = writeln!(
                                out,
                                "- #{} [{}] ({:.2}) {}",
                                c.id,
                                c.status.as_str(),
                                c.confidence,
                                c.text
                            );
                        }
                        Ok(ToolResult::new(out))
                    }
                    Err(e) => Ok(ToolResult::error(format!("claim list failed: {e}"))),
                }
            }
            "get" => {
                let Some(id) = params.get("id").and_then(serde_json::Value::as_i64) else {
                    return Ok(ToolResult::error("claim get: missing 'id'".to_string()));
                };
                match self.db.get_claim(id) {
                    Ok(Some(c)) => {
                        use std::fmt::Write as _;
                        let mut out = format!(
                            "Claim #{}\nStatus: {}\nConfidence: {:.2}\nText: {}\n",
                            c.id,
                            c.status.as_str(),
                            c.confidence,
                            c.text
                        );
                        if c.evidence.is_empty() {
                            out.push_str("Evidence: (none)\n");
                        } else {
                            out.push_str("Evidence:\n");
                            for e in &c.evidence {
                                let _ = writeln!(out, "  - {}: {}", e.kind, e.value);
                            }
                        }
                        // Bump last_seen_ms — re-observation signal.
                        let _ = self.db.touch_claim_last_seen(id);
                        Ok(ToolResult::new(out))
                    }
                    Ok(None) => Ok(ToolResult::error(format!("claim #{id} not found"))),
                    Err(e) => Ok(ToolResult::error(format!("claim get failed: {e}"))),
                }
            }
            "lint" => {
                let contradictions = self
                    .db
                    .find_contradiction_pairs(LINT_CONTRADICTION_MIN_SHARED_TOKENS)
                    .unwrap_or_default();
                let stale = self
                    .db
                    .find_stale_low_confidence_claims(
                        LINT_STALE_CONFIDENCE_MAX,
                        LINT_STALE_CUTOFF_DAYS,
                    )
                    .unwrap_or_default();
                let orphans = self.db.find_orphan_claims().unwrap_or_default();
                let counts = self.db.count_claims_by_status().unwrap_or_default();
                Ok(ToolResult::new(Self::render_lint_report(
                    &contradictions,
                    &stale,
                    &orphans,
                    &counts,
                )))
            }
            "update_status" => {
                let Some(id) = params.get("id").and_then(serde_json::Value::as_i64) else {
                    return Ok(ToolResult::error(
                        "claim update_status: missing 'id'".to_string(),
                    ));
                };
                let Some(status_raw) = params.get("status").and_then(Value::as_str) else {
                    return Ok(ToolResult::error(
                        "claim update_status: missing 'status'".to_string(),
                    ));
                };
                let Some(status) = ClaimStatus::parse(status_raw) else {
                    return Ok(ToolResult::error(format!(
                        "claim update_status: unknown status '{status_raw}'. \
                         Use open / accepted / retracted / contradicted."
                    )));
                };
                match self.db.update_claim_status(id, status) {
                    Ok(true) => Ok(ToolResult::new(format!(
                        "Claim #{id} marked {}.",
                        status.as_str()
                    ))),
                    Ok(false) => Ok(ToolResult::error(format!("claim #{id} not found"))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "claim update_status failed: {e}"
                    ))),
                }
            }
            other => Ok(ToolResult::error(format!(
                "claim: unknown action '{other}'"
            ))),
        }
    }
}
