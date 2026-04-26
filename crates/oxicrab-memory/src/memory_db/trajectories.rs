use super::MemoryDB;
use anyhow::Result;
use rusqlite::params;

/// One event in the agent's tool loop. Recorded only when
/// `agents.defaults.trajectory.enabled = true`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrajectoryEventType {
    ToolCall,
    ToolResult,
    TurnEnd,
}

impl TrajectoryEventType {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::TurnEnd => "turn_end",
        }
    }
}

/// One row in `trajectory_events`. `tool_name`/`action`/`latency_ms`/`is_error`
/// are only populated for tool events.
#[derive(Debug, Clone)]
pub struct TrajectoryEvent {
    pub session_id: String,
    pub turn_index: u32,
    pub event_type: TrajectoryEventType,
    pub tool_name: Option<String>,
    pub action: Option<String>,
    pub latency_ms: Option<i64>,
    pub is_error: Option<bool>,
    pub created_at_ms: i64,
}

/// A repeated `tool[/action]` sequence detected across turns. The
/// `fingerprint` is a tab-joined `tool/action` list and is used as a
/// stable identity for coverage checks and dedup.
#[derive(Debug, Clone)]
pub struct RepeatedSequence {
    pub fingerprint: String,
    pub steps: Vec<String>,
    pub occurrences: u32,
    pub example_session_id: String,
}

/// LLM-generated summary of an old trajectory session. Old raw events are
/// deleted after compression; `fingerprint` keeps the pattern signal.
#[derive(Debug, Clone)]
pub struct TrajectorySummary {
    pub session_id: String,
    pub summary: String,
    pub fingerprint: Option<String>,
    pub occurrences: u32,
    pub candidate_name: Option<String>,
    pub candidate_desc: Option<String>,
    pub candidate_conf: Option<f64>,
    pub created_at_ms: i64,
}

impl MemoryDB {
    /// Append one trajectory event. Single-row insert — callers in the
    /// hot tool loop should keep this off the critical path (spawn_blocking
    /// when the surrounding context is async).
    pub fn insert_trajectory_event(&self, ev: &TrajectoryEvent) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO trajectory_events (
                session_id, turn_index, event_type, tool_name, action,
                latency_ms, is_error, created_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                ev.session_id,
                ev.turn_index as i64,
                ev.event_type.as_str(),
                ev.tool_name,
                ev.action,
                ev.latency_ms,
                ev.is_error.map(i64::from),
                ev.created_at_ms,
            ],
        )?;
        Ok(())
    }

    /// Count rows in `trajectory_events`. Test/observability helper.
    pub fn count_trajectory_events(&self) -> Result<u64> {
        let conn = self.lock_conn()?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM trajectory_events", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    /// Find tool sequences (≥ `min_len` consecutive `tool_call` events
    /// per turn) that repeat across turns. Returns sequences sorted by
    /// occurrence count, descending; trims to `max_steps` steps.
    pub fn find_repeated_tool_sequences(
        &self,
        min_occurrences: u32,
        min_len: usize,
        max_steps: usize,
    ) -> Result<Vec<RepeatedSequence>> {
        if min_len == 0 || max_steps == 0 {
            return Ok(Vec::new());
        }
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT session_id, turn_index, tool_name, action
               FROM trajectory_events
              WHERE event_type = 'tool_call'
              ORDER BY session_id, turn_index, id",
        )?;
        let mut rows = stmt.query([])?;

        // Bucket steps per (session, turn) preserving order.
        let mut current: Option<(String, u32)> = None;
        let mut current_steps: Vec<String> = Vec::new();
        let mut all_turns: Vec<(String, Vec<String>)> = Vec::new();
        while let Some(row) = rows.next()? {
            let session_id: String = row.get(0)?;
            let turn_index: i64 = row.get(1)?;
            let turn_index = turn_index.max(0) as u32;
            let tool: Option<String> = row.get(2)?;
            let action: Option<String> = row.get(3)?;
            let step = match (tool.as_deref(), action.as_deref()) {
                (Some(t), Some(a)) if !a.is_empty() => format!("{t}/{a}"),
                (Some(t), _) => t.to_string(),
                _ => continue,
            };
            let key = (session_id.clone(), turn_index);
            if Some(&key) != current.as_ref() {
                if let Some((sid, _)) = current.take()
                    && current_steps.len() >= min_len
                {
                    let mut steps = std::mem::take(&mut current_steps);
                    steps.truncate(max_steps);
                    all_turns.push((sid, steps));
                }
                current_steps.clear();
                current = Some(key);
            }
            current_steps.push(step);
        }
        if let Some((sid, _)) = current.take()
            && current_steps.len() >= min_len
        {
            let mut steps = current_steps;
            steps.truncate(max_steps);
            all_turns.push((sid, steps));
        }

        let mut counts: std::collections::HashMap<String, (Vec<String>, u32, String)> =
            std::collections::HashMap::new();
        for (sid, steps) in all_turns {
            let fingerprint = steps.join("\t");
            counts
                .entry(fingerprint)
                .and_modify(|(_, n, _)| *n += 1)
                .or_insert_with(|| (steps, 1, sid));
        }
        let mut out: Vec<RepeatedSequence> = counts
            .into_iter()
            .filter(|(_, (_, n, _))| *n >= min_occurrences)
            .map(|(fp, (steps, n, sid))| RepeatedSequence {
                fingerprint: fp,
                steps,
                occurrences: n,
                example_session_id: sid,
            })
            .collect();
        out.sort_by(|a, b| {
            b.occurrences
                .cmp(&a.occurrences)
                .then_with(|| a.fingerprint.cmp(&b.fingerprint))
        });
        Ok(out)
    }

    /// Return distinct session ids for which every event predates `cutoff_ms`.
    /// Used by trajectory compression to find batches of dormant sessions.
    pub fn trajectory_sessions_older_than(&self, cutoff_ms: i64) -> Result<Vec<String>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT session_id
               FROM trajectory_events
              GROUP BY session_id
             HAVING MAX(created_at_ms) < ?1",
        )?;
        let sids: Vec<String> = stmt
            .query_map([cutoff_ms], |row| row.get::<_, String>(0))?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(sids)
    }

    /// Read all events for one session as a chronologically-ordered list.
    /// Used by compression to feed an LLM and to render NL renderings.
    pub fn read_trajectory_session(&self, session_id: &str) -> Result<Vec<TrajectoryEvent>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT session_id, turn_index, event_type, tool_name, action,
                    latency_ms, is_error, created_at_ms
               FROM trajectory_events
              WHERE session_id = ?1
              ORDER BY id",
        )?;
        let evs = stmt
            .query_map([session_id], |row| {
                let event_type: String = row.get(2)?;
                let event_type = match event_type.as_str() {
                    "tool_call" => TrajectoryEventType::ToolCall,
                    "tool_result" => TrajectoryEventType::ToolResult,
                    _ => TrajectoryEventType::TurnEnd,
                };
                let is_error: Option<i64> = row.get(6)?;
                Ok(TrajectoryEvent {
                    session_id: row.get(0)?,
                    turn_index: (row.get::<_, i64>(1)?).max(0) as u32,
                    event_type,
                    tool_name: row.get(3)?,
                    action: row.get(4)?,
                    latency_ms: row.get(5)?,
                    is_error: is_error.map(|x| x != 0),
                    created_at_ms: row.get(7)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(evs)
    }

    /// Persist a compressed trajectory summary and remove the raw events
    /// for the same session. Idempotent on `session_id` (REPLACE).
    pub fn save_trajectory_summary(&self, s: &TrajectorySummary) -> Result<()> {
        // Cap the LLM-generated summary so a verbose model can't
        // multiply 10 KB summaries across thousands of sessions and
        // OOM the DB. 8 KB is well above the prompt template's
        // expected output and below the BLOB performance cliff.
        const MAX_SUMMARY_BYTES: usize = 8 * 1024;
        let summary = if s.summary.len() > MAX_SUMMARY_BYTES {
            let mut cut = MAX_SUMMARY_BYTES;
            while cut > 0 && !s.summary.is_char_boundary(cut) {
                cut -= 1;
            }
            format!("{}…", &s.summary[..cut])
        } else {
            s.summary.clone()
        };
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO trajectory_summaries (
                session_id, summary, fingerprint, occurrences,
                candidate_name, candidate_desc, candidate_conf, created_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                s.session_id,
                summary,
                s.fingerprint,
                s.occurrences as i64,
                s.candidate_name,
                s.candidate_desc,
                s.candidate_conf,
                s.created_at_ms,
            ],
        )?;
        conn.execute(
            "DELETE FROM trajectory_events WHERE session_id = ?1",
            params![s.session_id],
        )?;
        Ok(())
    }

    /// Count summary rows. Test/observability helper.
    pub fn count_trajectory_summaries(&self) -> Result<u64> {
        let conn = self.lock_conn()?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM trajectory_summaries", [], |r| {
            r.get(0)
        })?;
        Ok(n as u64)
    }
}

/// One row in `skill_refinements` — written by the auto-refine pipeline
/// after a confidence-gated patch lands.
#[derive(Debug, Clone)]
pub struct SkillRefinementRecord {
    pub skill_name: String,
    pub confidence: f64,
    pub reason: String,
    pub bytes_before: i64,
    pub bytes_after: i64,
    pub version_after: String,
    pub created_at_ms: i64,
}

impl MemoryDB {
    /// Append one refinement record.
    pub fn insert_skill_refinement(&self, rec: &SkillRefinementRecord) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO skill_refinements (
                skill_name, confidence, reason, bytes_before, bytes_after,
                version_after, created_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                rec.skill_name,
                rec.confidence,
                rec.reason,
                rec.bytes_before,
                rec.bytes_after,
                rec.version_after,
                rec.created_at_ms,
            ],
        )?;
        Ok(())
    }

    /// Count refinements for one skill. Tests use this to verify the
    /// auto-refine pipeline didn't double-fire.
    pub fn count_skill_refinements(&self, skill_name: &str) -> Result<u64> {
        let conn = self.lock_conn()?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM skill_refinements WHERE skill_name = ?1",
            [skill_name],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn db() -> (MemoryDB, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let db = MemoryDB::new(dir.path().join("t.db")).unwrap();
        (db, dir)
    }

    fn ev(
        session: &str,
        turn: u32,
        kind: TrajectoryEventType,
        tool: Option<&str>,
    ) -> TrajectoryEvent {
        TrajectoryEvent {
            session_id: session.to_string(),
            turn_index: turn,
            event_type: kind,
            tool_name: tool.map(str::to_string),
            action: None,
            latency_ms: None,
            is_error: None,
            created_at_ms: chrono::Utc::now().timestamp_millis(),
        }
    }

    #[test]
    fn insert_and_count() {
        let (db, _g) = db();
        assert_eq!(db.count_trajectory_events().unwrap(), 0);
        db.insert_trajectory_event(&ev("s1", 1, TrajectoryEventType::ToolCall, Some("a")))
            .unwrap();
        db.insert_trajectory_event(&ev("s1", 1, TrajectoryEventType::ToolResult, Some("a")))
            .unwrap();
        assert_eq!(db.count_trajectory_events().unwrap(), 2);
    }

    #[test]
    fn finds_repeated_sequences_across_turns() {
        let (db, _g) = db();
        // Three turns: A→B, A→B, A→C — A→B should appear twice.
        for turn in [1, 2] {
            db.insert_trajectory_event(&ev("s1", turn, TrajectoryEventType::ToolCall, Some("a")))
                .unwrap();
            db.insert_trajectory_event(&ev("s1", turn, TrajectoryEventType::ToolCall, Some("b")))
                .unwrap();
        }
        db.insert_trajectory_event(&ev("s1", 3, TrajectoryEventType::ToolCall, Some("a")))
            .unwrap();
        db.insert_trajectory_event(&ev("s1", 3, TrajectoryEventType::ToolCall, Some("c")))
            .unwrap();

        let seqs = db.find_repeated_tool_sequences(2, 2, 8).unwrap();
        assert_eq!(seqs.len(), 1, "only A→B should meet threshold");
        assert_eq!(seqs[0].steps, vec!["a", "b"]);
        assert_eq!(seqs[0].occurrences, 2);
    }

    #[test]
    fn ignores_short_sequences() {
        let (db, _g) = db();
        db.insert_trajectory_event(&ev("s1", 1, TrajectoryEventType::ToolCall, Some("a")))
            .unwrap();
        db.insert_trajectory_event(&ev("s1", 2, TrajectoryEventType::ToolCall, Some("a")))
            .unwrap();
        let seqs = db.find_repeated_tool_sequences(2, 2, 8).unwrap();
        assert!(
            seqs.is_empty(),
            "single-step turns must be filtered by min_len"
        );
    }

    #[test]
    fn save_summary_deletes_raw_events() {
        let (db, _g) = db();
        db.insert_trajectory_event(&ev("s1", 1, TrajectoryEventType::ToolCall, Some("a")))
            .unwrap();
        db.insert_trajectory_event(&ev("s2", 1, TrajectoryEventType::ToolCall, Some("a")))
            .unwrap();
        db.save_trajectory_summary(&TrajectorySummary {
            session_id: "s1".to_string(),
            summary: "did stuff".to_string(),
            fingerprint: Some("a".to_string()),
            occurrences: 1,
            candidate_name: None,
            candidate_desc: None,
            candidate_conf: None,
            created_at_ms: chrono::Utc::now().timestamp_millis(),
        })
        .unwrap();
        assert_eq!(db.count_trajectory_summaries().unwrap(), 1);
        // Only s2's event should remain.
        let remaining = db.read_trajectory_session("s1").unwrap();
        assert!(remaining.is_empty());
        let s2 = db.read_trajectory_session("s2").unwrap();
        assert_eq!(s2.len(), 1);
    }

    #[test]
    fn sessions_older_than_filters_correctly() {
        let (db, _g) = db();
        let now = chrono::Utc::now().timestamp_millis();
        let mut old_ev = ev("old", 1, TrajectoryEventType::ToolCall, Some("a"));
        old_ev.created_at_ms = now - 10_000;
        let mut new_ev = ev("new", 1, TrajectoryEventType::ToolCall, Some("a"));
        new_ev.created_at_ms = now;
        db.insert_trajectory_event(&old_ev).unwrap();
        db.insert_trajectory_event(&new_ev).unwrap();
        let old = db.trajectory_sessions_older_than(now - 5_000).unwrap();
        assert_eq!(old, vec!["old".to_string()]);
    }

    #[test]
    fn skill_refinements_round_trip() {
        let (db, _g) = db();
        db.insert_skill_refinement(&SkillRefinementRecord {
            skill_name: "demo".to_string(),
            confidence: 0.85,
            reason: "tightened wording".to_string(),
            bytes_before: 100,
            bytes_after: 120,
            version_after: "1.1.0".to_string(),
            created_at_ms: chrono::Utc::now().timestamp_millis(),
        })
        .unwrap();
        assert_eq!(db.count_skill_refinements("demo").unwrap(), 1);
        assert_eq!(db.count_skill_refinements("other").unwrap(), 0);
    }
}
