//! Structured claims with confidence + status + evidence.
//!
//! Treats memory entries as **assertions**, not prose:
//!
//! - `text`: what's being claimed
//! - `confidence`: how sure the source was (0.0–1.0). Hedges land low.
//! - `status`: open / accepted / retracted / contradicted
//! - `evidence`: typed pointers back to the conversation, file, or
//!   tool result that supports the claim
//!
//! Together with the `_Source: …_` retrieval citation we already added,
//! this closes the hallucination loop on the ingestion side: low-
//! confidence hedges no longer round to fact, and contradictions
//! surface via `claim_lint` instead of accumulating silently.

use super::MemoryDB;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};

/// Status of a claim. Lifecycle: every claim starts `Open`. Operators
/// (or the agent itself via the claims tool) move them through:
/// - `Accepted`  — corroborated, treat as fact
/// - `Retracted` — explicitly disowned by the source
/// - `Contradicted` — flagged by `claim_lint` against another claim;
///   needs operator/agent reconciliation before it goes back to
///   `Accepted` or `Retracted`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimStatus {
    Open,
    Accepted,
    Retracted,
    Contradicted,
}

impl ClaimStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Accepted => "accepted",
            Self::Retracted => "retracted",
            Self::Contradicted => "contradicted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "accepted" => Some(Self::Accepted),
            "retracted" => Some(Self::Retracted),
            "contradicted" => Some(Self::Contradicted),
            _ => None,
        }
    }
}

/// Where a claim's assertion came from. Gates promotion to durable
/// (`Accepted`) memory: an `AgentInferred` claim — the agent's own guess,
/// not something the user said or a tool observed — must NOT be promoted
/// to `Accepted` until a later user turn confirms it (which upgrades its
/// provenance to `UserConfirmed`). This stops the agent from laundering
/// its own speculation into fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// The user stated it directly. Highest trust.
    UserConfirmed,
    /// A tool result or file/source observation. Concrete but not
    /// user-blessed.
    Observed,
    /// The agent inferred it. Lowest trust; cannot be promoted to
    /// `Accepted` without user confirmation.
    AgentInferred,
}

impl Provenance {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserConfirmed => "user_confirmed",
            Self::Observed => "observed",
            Self::AgentInferred => "agent_inferred",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user_confirmed" => Some(Self::UserConfirmed),
            "observed" => Some(Self::Observed),
            "agent_inferred" => Some(Self::AgentInferred),
            _ => None,
        }
    }
}

/// One typed pointer back to whatever supports a claim. Different kinds
/// of evidence need different schemas, so we store the kind explicitly
/// instead of pretending all sources are URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidencePointer {
    pub kind: String,
    pub value: String,
}

impl EvidencePointer {
    pub fn message(channel: &str, msg_id: &str) -> Self {
        Self {
            kind: "message".to_string(),
            value: format!("{channel}:{msg_id}"),
        }
    }
    pub fn file(path: &str) -> Self {
        Self {
            kind: "file".to_string(),
            value: path.to_string(),
        }
    }
    pub fn tool(name: &str, action: Option<&str>) -> Self {
        Self {
            kind: "tool".to_string(),
            value: action.map_or_else(|| name.to_string(), |a| format!("{name}/{a}")),
        }
    }
}

/// One row from the `claims` table plus its evidence list.
#[derive(Debug, Clone)]
pub struct Claim {
    pub id: i64,
    pub text: String,
    pub confidence: f32,
    pub status: ClaimStatus,
    pub provenance: Provenance,
    pub evidence: Vec<EvidencePointer>,
    pub last_seen_ms: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Outcome of a claim status transition. `BlockedInferred` means the
/// caller tried to promote an `AgentInferred` claim to `Accepted` without
/// user confirmation — the write was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusUpdate {
    Updated,
    NotFound,
    BlockedInferred,
}

/// One pair flagged as a potential contradiction. Heuristic — the
/// caller decides whether to mark either side `contradicted`.
#[derive(Debug, Clone)]
pub struct ContradictionPair {
    pub a_id: i64,
    pub b_id: i64,
    pub a_text: String,
    pub b_text: String,
    pub a_confidence: f32,
    pub b_confidence: f32,
}

impl MemoryDB {
    /// Insert a claim. Returns the new claim id.
    pub fn insert_claim(
        &self,
        text: &str,
        confidence: f32,
        status: ClaimStatus,
        evidence: &[EvidencePointer],
    ) -> Result<i64> {
        self.insert_claim_with_provenance(text, confidence, status, Provenance::Observed, evidence)
    }

    /// Insert a claim with an explicit provenance tier. `insert_claim`
    /// defaults to `Observed`; callers that know the source should use
    /// this so the promotion gate can enforce trust.
    pub fn insert_claim_with_provenance(
        &self,
        text: &str,
        confidence: f32,
        status: ClaimStatus,
        provenance: Provenance,
        evidence: &[EvidencePointer],
    ) -> Result<i64> {
        let now = chrono::Utc::now().timestamp_millis();
        let confidence = confidence.clamp(0.0, 1.0);
        // Enforce the same gate as `update_claim_status`: an agent-inferred
        // claim cannot be born already `Accepted`. Downgrade to `Open` so
        // promotion still requires user confirmation.
        let status = if status == ClaimStatus::Accepted && provenance == Provenance::AgentInferred {
            ClaimStatus::Open
        } else {
            status
        };
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO claims (text, confidence, status, provenance, last_seen_ms,
                                  created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![
                text,
                confidence,
                status.as_str(),
                provenance.as_str(),
                now,
                now
            ],
        )?;
        let id = tx.last_insert_rowid();
        for e in evidence {
            tx.execute(
                "INSERT INTO claim_evidence (claim_id, pointer_kind, pointer_value, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, e.kind, e.value, now],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Update the status of an existing claim (and bump `updated_at_ms`).
    ///
    /// Promotion gate: moving an `AgentInferred` claim to `Accepted` is
    /// refused (`StatusUpdate::BlockedInferred`) until its provenance is
    /// upgraded via [`set_claim_provenance`] — i.e. a later user turn
    /// confirmed it. All other transitions (retract, contradict, or
    /// promoting user-confirmed/observed claims) proceed normally.
    pub fn update_claim_status(&self, id: i64, status: ClaimStatus) -> Result<StatusUpdate> {
        let conn = self.lock_conn()?;
        if status == ClaimStatus::Accepted {
            let provenance: Option<String> = conn
                .query_row("SELECT provenance FROM claims WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?;
            match provenance {
                None => return Ok(StatusUpdate::NotFound),
                Some(p) if Provenance::parse(&p) == Some(Provenance::AgentInferred) => {
                    return Ok(StatusUpdate::BlockedInferred);
                }
                Some(_) => {}
            }
        }
        let n = conn.execute(
            "UPDATE claims SET status = ?1, updated_at_ms = ?2 WHERE id = ?3",
            params![status.as_str(), chrono::Utc::now().timestamp_millis(), id],
        )?;
        Ok(if n > 0 {
            StatusUpdate::Updated
        } else {
            StatusUpdate::NotFound
        })
    }

    /// Set a claim's provenance tier and bump `updated_at_ms`. Used to
    /// record that a later user turn confirmed a previously agent-inferred
    /// claim, which unblocks promotion to `Accepted`.
    pub fn set_claim_provenance(&self, id: i64, provenance: Provenance) -> Result<bool> {
        let conn = self.lock_conn()?;
        let n = conn.execute(
            "UPDATE claims SET provenance = ?1, updated_at_ms = ?2 WHERE id = ?3",
            params![
                provenance.as_str(),
                chrono::Utc::now().timestamp_millis(),
                id
            ],
        )?;
        Ok(n > 0)
    }

    /// Bump `last_seen_ms` to "now" — used when the agent re-observes
    /// the claim during a turn. Keeps the staleness lint accurate.
    pub fn touch_claim_last_seen(&self, id: i64) -> Result<bool> {
        let conn = self.lock_conn()?;
        let n = conn.execute(
            "UPDATE claims SET last_seen_ms = ?1 WHERE id = ?2",
            params![chrono::Utc::now().timestamp_millis(), id],
        )?;
        Ok(n > 0)
    }

    /// Fetch a single claim with its evidence.
    pub fn get_claim(&self, id: i64) -> Result<Option<Claim>> {
        let conn = self.lock_conn()?;
        let mut head = conn.prepare(
            "SELECT id, text, confidence, status, last_seen_ms,
                    created_at_ms, updated_at_ms, provenance
               FROM claims WHERE id = ?1",
        )?;
        let mut rows = head.query([id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let raw_status: String = row.get(3)?;
        let status = ClaimStatus::parse(&raw_status).unwrap_or(ClaimStatus::Open);
        let raw_provenance: String = row.get(7)?;
        let provenance = Provenance::parse(&raw_provenance).unwrap_or(Provenance::Observed);
        let mut claim = Claim {
            id: row.get(0)?,
            text: row.get(1)?,
            confidence: row.get::<_, f64>(2)? as f32,
            status,
            provenance,
            evidence: Vec::new(),
            last_seen_ms: row.get(4)?,
            created_at_ms: row.get(5)?,
            updated_at_ms: row.get(6)?,
        };
        drop(rows);
        drop(head);
        let mut ev_stmt = conn.prepare(
            "SELECT pointer_kind, pointer_value FROM claim_evidence
              WHERE claim_id = ?1 ORDER BY id",
        )?;
        let evs = ev_stmt.query_map([id], |r| {
            Ok(EvidencePointer {
                kind: r.get(0)?,
                value: r.get(1)?,
            })
        })?;
        for e in evs {
            claim.evidence.push(e?);
        }
        Ok(Some(claim))
    }

    /// List claims, optionally filtered by status.
    pub fn list_claims(
        &self,
        status_filter: Option<ClaimStatus>,
        limit: usize,
    ) -> Result<Vec<Claim>> {
        // Collect ids while holding the lock, then DROP it before
        // calling get_claim for each — get_claim re-acquires the
        // mutex and std::sync::Mutex is not re-entrant.
        let ids: Vec<i64> = {
            let conn = self.lock_conn()?;
            let mut stmt = if status_filter.is_some() {
                conn.prepare(
                    "SELECT id FROM claims WHERE status = ?1
                     ORDER BY last_seen_ms DESC LIMIT ?2",
                )?
            } else {
                conn.prepare("SELECT id FROM claims ORDER BY last_seen_ms DESC LIMIT ?1")?
            };
            match status_filter {
                Some(s) => stmt
                    .query_map(params![s.as_str(), limit as i64], |r| r.get(0))?
                    .filter_map(std::result::Result::ok)
                    .collect(),
                None => stmt
                    .query_map(params![limit as i64], |r| r.get(0))?
                    .filter_map(std::result::Result::ok)
                    .collect(),
            }
        };
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(c) = self.get_claim(id)? {
                out.push(c);
            }
        }
        Ok(out)
    }

    /// Find pairs of claims with high text overlap that may be
    /// contradictions. Heuristic only — the caller decides whether
    /// to mark them `Contradicted`. Walks all `Open` and `Accepted`
    /// claims and flags pairs whose lowercased text shares ≥ N tokens
    /// AND whose tokens contain opposite polarity markers.
    pub fn find_contradiction_pairs(
        &self,
        min_shared_tokens: usize,
    ) -> Result<Vec<ContradictionPair>> {
        let claims = self.list_claims(None, 1000)?;
        let active: Vec<_> = claims
            .into_iter()
            .filter(|c| matches!(c.status, ClaimStatus::Open | ClaimStatus::Accepted))
            .collect();
        let mut out = Vec::new();
        for i in 0..active.len() {
            for j in (i + 1)..active.len() {
                let a = &active[i];
                let b = &active[j];
                let shared = shared_token_count(&a.text, &b.text);
                if shared < min_shared_tokens {
                    continue;
                }
                if !has_opposite_polarity(&a.text, &b.text) {
                    continue;
                }
                out.push(ContradictionPair {
                    a_id: a.id,
                    b_id: b.id,
                    a_text: a.text.clone(),
                    b_text: b.text.clone(),
                    a_confidence: a.confidence,
                    b_confidence: b.confidence,
                });
            }
        }
        Ok(out)
    }

    /// Find claims with low confidence that haven't been re-observed
    /// recently — likely stale guesses. `cutoff_days` is the
    /// last-seen window; `max_confidence` is the bar.
    pub fn find_stale_low_confidence_claims(
        &self,
        max_confidence: f32,
        cutoff_days: u32,
    ) -> Result<Vec<Claim>> {
        let cutoff = chrono::Utc::now().timestamp_millis() - i64::from(cutoff_days) * 86_400_000;
        let ids: Vec<i64> = {
            let conn = self.lock_conn()?;
            let mut stmt = conn.prepare(
                "SELECT id FROM claims
                  WHERE confidence <= ?1 AND last_seen_ms < ?2
                    AND status IN ('open', 'accepted')
                  ORDER BY confidence ASC, last_seen_ms ASC",
            )?;
            stmt.query_map(params![max_confidence as f64, cutoff], |r| r.get(0))?
                .filter_map(std::result::Result::ok)
                .collect()
        };
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(c) = self.get_claim(id)? {
                out.push(c);
            }
        }
        Ok(out)
    }

    /// Find claims with no evidence pointers. These were probably
    /// imported via the bulk path and should get a pointer assigned
    /// or be retracted.
    pub fn find_orphan_claims(&self) -> Result<Vec<Claim>> {
        let ids: Vec<i64> = {
            let conn = self.lock_conn()?;
            let mut stmt = conn.prepare(
                "SELECT id FROM claims
                  WHERE id NOT IN (SELECT claim_id FROM claim_evidence)
                    AND status IN ('open', 'accepted')",
            )?;
            stmt.query_map([], |r| r.get(0))?
                .filter_map(std::result::Result::ok)
                .collect()
        };
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(c) = self.get_claim(id)? {
                out.push(c);
            }
        }
        Ok(out)
    }

    /// Count claims by status.
    pub fn count_claims_by_status(&self) -> Result<Vec<(ClaimStatus, u32)>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM claims GROUP BY status")?;
        let rows = stmt.query_map([], |r| {
            let s: String = r.get(0)?;
            let n: i64 = r.get(1)?;
            Ok((s, n))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (s, n) = r?;
            if let Some(status) = ClaimStatus::parse(&s) {
                out.push((status, n as u32));
            }
        }
        Ok(out)
    }

    /// Delete a claim and its evidence.
    pub fn delete_claim(&self, id: i64) -> Result<bool> {
        let conn = self.lock_conn()?;
        let n = conn.execute("DELETE FROM claims WHERE id = ?1", [id])?;
        Ok(n > 0)
    }
}

/// Tokenise both strings, lowercase, drop stop words and tokens shorter
/// than 3 chars, then count the intersection. Cheap baseline for the
/// contradiction detector — cross-references against the polarity
/// check, never alone.
fn shared_token_count(a: &str, b: &str) -> usize {
    let stop_words: &[&str] = &[
        "the", "and", "for", "but", "with", "this", "that", "have", "are", "was", "were", "from",
        "into", "than", "then", "they", "their", "there",
    ];
    let toks_a: std::collections::HashSet<String> = tokenize(a)
        .into_iter()
        .filter(|t| t.len() >= 3 && !stop_words.contains(&t.as_str()))
        .collect();
    let toks_b: std::collections::HashSet<String> = tokenize(b)
        .into_iter()
        .filter(|t| t.len() >= 3 && !stop_words.contains(&t.as_str()))
        .collect();
    toks_a.intersection(&toks_b).count()
}

fn tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Detect explicit polarity markers — negation (`not`, `no`, `never`,
/// `dislike`, `don't`) or contradictory verbs (`prefers` X vs Y).
/// Returns true only when the two strings disagree in obvious ways;
/// the caller still has to confirm. Kept conservative.
fn has_opposite_polarity(a: &str, b: &str) -> bool {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();

    // Simple negation pattern: one contains "not"/"no"/"never"/"don't"
    // / "dislike", the other doesn't.
    let negation_markers = [
        "not ", " no ", "never ", "don't ", "dislike", "doesn't ", "isn't ",
    ];
    let a_neg = negation_markers.iter().any(|m| a_lower.contains(m));
    let b_neg = negation_markers.iter().any(|m| b_lower.contains(m));
    if a_neg != b_neg {
        return true;
    }

    // "prefers X" vs "prefers Y" pattern: same verb, different object.
    // Look for shared "X Y" with the same head verb.
    for verb in ["prefers ", "likes ", "loves ", "uses "] {
        let (Some(ai), Some(bi)) = (a_lower.find(verb), b_lower.find(verb)) else {
            continue;
        };
        let after_a = &a_lower[ai + verb.len()..]
            .split_whitespace()
            .next()
            .unwrap_or("");
        let after_b = &b_lower[bi + verb.len()..]
            .split_whitespace()
            .next()
            .unwrap_or("");
        if !after_a.is_empty() && !after_b.is_empty() && after_a != after_b {
            return true;
        }
    }
    false
}

impl std::hash::Hash for ClaimStatus {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn db() -> (MemoryDB, tempfile::TempDir) {
        let d = tempdir().unwrap();
        (MemoryDB::new(d.path().join("t.db")).unwrap(), d)
    }

    #[test]
    fn insert_and_fetch_claim() {
        let (db, _g) = db();
        let evidence = vec![EvidencePointer::message("telegram", "msg-1234")];
        let id = db
            .insert_claim(
                "User prefers Rust over Python",
                0.85,
                ClaimStatus::Open,
                &evidence,
            )
            .unwrap();
        let c = db.get_claim(id).unwrap().unwrap();
        assert_eq!(c.text, "User prefers Rust over Python");
        assert!((c.confidence - 0.85).abs() < 0.001);
        assert_eq!(c.status, ClaimStatus::Open);
        assert_eq!(c.evidence.len(), 1);
        assert_eq!(c.evidence[0].kind, "message");
        assert_eq!(c.evidence[0].value, "telegram:msg-1234");
    }

    #[test]
    fn confidence_clamps_to_unit_interval() {
        let (db, _g) = db();
        let id = db
            .insert_claim("over the limit", 1.5, ClaimStatus::Open, &[])
            .unwrap();
        let c = db.get_claim(id).unwrap().unwrap();
        assert!((c.confidence - 1.0).abs() < 0.001);
        let id2 = db
            .insert_claim("under the limit", -0.3, ClaimStatus::Open, &[])
            .unwrap();
        let c2 = db.get_claim(id2).unwrap().unwrap();
        assert!((c2.confidence - 0.0).abs() < 0.001);
    }

    #[test]
    fn status_lifecycle() {
        let (db, _g) = db();
        let id = db
            .insert_claim("test", 0.5, ClaimStatus::Open, &[])
            .unwrap();
        db.update_claim_status(id, ClaimStatus::Accepted).unwrap();
        assert_eq!(
            db.get_claim(id).unwrap().unwrap().status,
            ClaimStatus::Accepted
        );
        db.update_claim_status(id, ClaimStatus::Retracted).unwrap();
        assert_eq!(
            db.get_claim(id).unwrap().unwrap().status,
            ClaimStatus::Retracted
        );
    }

    #[test]
    fn list_claims_filters_by_status() {
        let (db, _g) = db();
        let _open = db.insert_claim("a", 0.5, ClaimStatus::Open, &[]).unwrap();
        let _acc = db
            .insert_claim("b", 0.9, ClaimStatus::Accepted, &[])
            .unwrap();
        let _ret = db
            .insert_claim("c", 0.5, ClaimStatus::Retracted, &[])
            .unwrap();
        let open_only = db.list_claims(Some(ClaimStatus::Open), 100).unwrap();
        assert_eq!(open_only.len(), 1);
        assert_eq!(open_only[0].text, "a");
        let all = db.list_claims(None, 100).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn contradiction_detector_flags_negation_pairs() {
        let (db, _g) = db();
        db.insert_claim(
            "User prefers Rust for systems work",
            0.8,
            ClaimStatus::Open,
            &[],
        )
        .unwrap();
        db.insert_claim(
            "User does not prefer Rust for systems work anymore",
            0.7,
            ClaimStatus::Open,
            &[],
        )
        .unwrap();
        let pairs = db.find_contradiction_pairs(2).unwrap();
        assert_eq!(pairs.len(), 1);
    }

    #[test]
    fn contradiction_detector_flags_prefers_x_vs_y() {
        let (db, _g) = db();
        db.insert_claim("User prefers Rust", 0.8, ClaimStatus::Open, &[])
            .unwrap();
        db.insert_claim("User prefers Go", 0.8, ClaimStatus::Open, &[])
            .unwrap();
        let pairs = db.find_contradiction_pairs(1).unwrap();
        assert_eq!(pairs.len(), 1);
    }

    #[test]
    fn contradiction_detector_skips_unrelated_claims() {
        let (db, _g) = db();
        db.insert_claim("User prefers Rust", 0.8, ClaimStatus::Open, &[])
            .unwrap();
        db.insert_claim("Project uses CMake", 0.8, ClaimStatus::Open, &[])
            .unwrap();
        let pairs = db.find_contradiction_pairs(2).unwrap();
        assert!(pairs.is_empty());
    }

    #[test]
    fn stale_low_confidence_finder() {
        let (db, _g) = db();
        let id = db
            .insert_claim("hedged", 0.3, ClaimStatus::Open, &[])
            .unwrap();
        // Backdate last_seen by 100 days.
        let cutoff = chrono::Utc::now().timestamp_millis() - 100 * 86_400_000;
        let conn = db.lock_conn().unwrap();
        conn.execute(
            "UPDATE claims SET last_seen_ms = ?1 WHERE id = ?2",
            params![cutoff, id],
        )
        .unwrap();
        drop(conn);
        let stale = db.find_stale_low_confidence_claims(0.4, 90).unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].id, id);
    }

    #[test]
    fn orphan_finder_only_lists_claims_without_evidence() {
        let (db, _g) = db();
        db.insert_claim("orphan", 0.5, ClaimStatus::Open, &[])
            .unwrap();
        db.insert_claim(
            "with evidence",
            0.5,
            ClaimStatus::Open,
            &[EvidencePointer::file("notes.md")],
        )
        .unwrap();
        let orphans = db.find_orphan_claims().unwrap();
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].text, "orphan");
    }

    #[test]
    fn count_by_status() {
        let (db, _g) = db();
        db.insert_claim("a", 0.5, ClaimStatus::Open, &[]).unwrap();
        db.insert_claim("b", 0.5, ClaimStatus::Open, &[]).unwrap();
        db.insert_claim("c", 0.5, ClaimStatus::Accepted, &[])
            .unwrap();
        let counts: std::collections::HashMap<ClaimStatus, u32> =
            db.count_claims_by_status().unwrap().into_iter().collect();
        assert_eq!(counts.get(&ClaimStatus::Open).copied(), Some(2));
        assert_eq!(counts.get(&ClaimStatus::Accepted).copied(), Some(1));
    }

    #[test]
    fn provenance_str_roundtrip() {
        for p in [
            Provenance::UserConfirmed,
            Provenance::Observed,
            Provenance::AgentInferred,
        ] {
            assert_eq!(Provenance::parse(p.as_str()), Some(p));
        }
        assert_eq!(
            Provenance::parse("user_confirmed"),
            Some(Provenance::UserConfirmed)
        );
        assert_eq!(Provenance::parse("observed"), Some(Provenance::Observed));
        assert_eq!(
            Provenance::parse("agent_inferred"),
            Some(Provenance::AgentInferred)
        );
        assert_eq!(Provenance::parse("nonsense"), None);
    }

    #[test]
    fn default_insert_is_observed() {
        let (db, _g) = db();
        let id = db
            .insert_claim("plain insert", 0.5, ClaimStatus::Open, &[])
            .unwrap();
        assert_eq!(
            db.get_claim(id).unwrap().unwrap().provenance,
            Provenance::Observed
        );
    }

    #[test]
    fn blocks_accept_of_agent_inferred() {
        let (db, _g) = db();
        let id = db
            .insert_claim_with_provenance(
                "agent guessed the user likes tabs",
                0.6,
                ClaimStatus::Open,
                Provenance::AgentInferred,
                &[],
            )
            .unwrap();
        assert_eq!(
            db.update_claim_status(id, ClaimStatus::Accepted).unwrap(),
            StatusUpdate::BlockedInferred
        );
        // The refused promotion must leave the stored status untouched.
        assert_eq!(db.get_claim(id).unwrap().unwrap().status, ClaimStatus::Open);
    }

    #[test]
    fn promotes_observed_and_user_confirmed() {
        let (db, _g) = db();
        let observed = db
            .insert_claim_with_provenance(
                "tool observed the repo uses cargo",
                0.7,
                ClaimStatus::Open,
                Provenance::Observed,
                &[],
            )
            .unwrap();
        let confirmed = db
            .insert_claim_with_provenance(
                "user said they prefer Rust",
                0.9,
                ClaimStatus::Open,
                Provenance::UserConfirmed,
                &[],
            )
            .unwrap();
        assert_eq!(
            db.update_claim_status(observed, ClaimStatus::Accepted)
                .unwrap(),
            StatusUpdate::Updated
        );
        assert_eq!(
            db.update_claim_status(confirmed, ClaimStatus::Accepted)
                .unwrap(),
            StatusUpdate::Updated
        );
        assert_eq!(
            db.get_claim(observed).unwrap().unwrap().status,
            ClaimStatus::Accepted
        );
        assert_eq!(
            db.get_claim(confirmed).unwrap().unwrap().status,
            ClaimStatus::Accepted
        );
    }

    #[test]
    fn confirm_then_promote_path() {
        let (db, _g) = db();
        let id = db
            .insert_claim_with_provenance(
                "agent inferred the user is on macOS",
                0.5,
                ClaimStatus::Open,
                Provenance::AgentInferred,
                &[],
            )
            .unwrap();
        assert_eq!(
            db.update_claim_status(id, ClaimStatus::Accepted).unwrap(),
            StatusUpdate::BlockedInferred
        );
        assert!(
            db.set_claim_provenance(id, Provenance::UserConfirmed)
                .unwrap()
        );
        assert_eq!(
            db.update_claim_status(id, ClaimStatus::Accepted).unwrap(),
            StatusUpdate::Updated
        );
        assert_eq!(
            db.get_claim(id).unwrap().unwrap().status,
            ClaimStatus::Accepted
        );
    }

    #[test]
    fn insert_accepted_inferred_is_downgraded() {
        let (db, _g) = db();
        let id = db
            .insert_claim_with_provenance(
                "agent tried to be born accepted",
                0.8,
                ClaimStatus::Accepted,
                Provenance::AgentInferred,
                &[],
            )
            .unwrap();
        let c = db.get_claim(id).unwrap().unwrap();
        assert_eq!(c.status, ClaimStatus::Open);
        assert_eq!(c.provenance, Provenance::AgentInferred);
    }

    #[test]
    fn retract_of_inferred_is_allowed() {
        let (db, _g) = db();
        let id = db
            .insert_claim_with_provenance(
                "agent inferred something wrong",
                0.4,
                ClaimStatus::Open,
                Provenance::AgentInferred,
                &[],
            )
            .unwrap();
        assert_eq!(
            db.update_claim_status(id, ClaimStatus::Retracted).unwrap(),
            StatusUpdate::Updated
        );
        assert_eq!(
            db.get_claim(id).unwrap().unwrap().status,
            ClaimStatus::Retracted
        );
    }

    #[test]
    fn update_status_missing_id_is_notfound() {
        let (db, _g) = db();
        assert_eq!(
            db.update_claim_status(9999, ClaimStatus::Accepted).unwrap(),
            StatusUpdate::NotFound
        );
    }
}
