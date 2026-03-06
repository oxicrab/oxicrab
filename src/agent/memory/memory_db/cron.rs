use super::MemoryDB;
use crate::cron::types::{
    CronJob, CronJobState, CronPayload, CronSchedule, CronTarget, UpdateJobParams,
};
use anyhow::Result;
use rusqlite::params;
use std::collections::HashMap;

fn schedule_type_str(schedule: &CronSchedule) -> &'static str {
    match schedule {
        CronSchedule::At { .. } => "at",
        CronSchedule::Every { .. } => "every",
        CronSchedule::Cron { .. } => "cron",
        CronSchedule::Event { .. } => "event",
    }
}

fn schedule_from_row(
    schedule_type: &str,
    at_ms: Option<i64>,
    every_ms: Option<i64>,
    cron_expr: Option<String>,
    cron_tz: Option<String>,
    event_pattern: Option<String>,
    event_channel: Option<String>,
) -> Result<CronSchedule> {
    match schedule_type {
        "at" => Ok(CronSchedule::At { at_ms }),
        "every" => Ok(CronSchedule::Every { every_ms }),
        "cron" => Ok(CronSchedule::Cron {
            expr: cron_expr,
            tz: cron_tz,
        }),
        "event" => Ok(CronSchedule::Event {
            pattern: event_pattern,
            channel: event_channel,
        }),
        other => anyhow::bail!("unknown schedule_type: {other}"),
    }
}

struct ScheduleColumns<'a> {
    at_ms: Option<i64>,
    every_ms: Option<i64>,
    cron_expr: Option<&'a str>,
    cron_tz: Option<&'a str>,
    event_pattern: Option<&'a str>,
    event_channel: Option<&'a str>,
}

fn schedule_columns(schedule: &CronSchedule) -> ScheduleColumns<'_> {
    match schedule {
        CronSchedule::At { at_ms } => ScheduleColumns {
            at_ms: *at_ms,
            every_ms: None,
            cron_expr: None,
            cron_tz: None,
            event_pattern: None,
            event_channel: None,
        },
        CronSchedule::Every { every_ms } => ScheduleColumns {
            at_ms: None,
            every_ms: *every_ms,
            cron_expr: None,
            cron_tz: None,
            event_pattern: None,
            event_channel: None,
        },
        CronSchedule::Cron { expr, tz } => ScheduleColumns {
            at_ms: None,
            every_ms: None,
            cron_expr: expr.as_deref(),
            cron_tz: tz.as_deref(),
            event_pattern: None,
            event_channel: None,
        },
        CronSchedule::Event { pattern, channel } => ScheduleColumns {
            at_ms: None,
            every_ms: None,
            cron_expr: None,
            cron_tz: None,
            event_pattern: pattern.as_deref(),
            event_channel: channel.as_deref(),
        },
    }
}

impl MemoryDB {
    pub fn insert_cron_job(&self, job: &CronJob) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let stype = schedule_type_str(&job.schedule);
        let cols = schedule_columns(&job.schedule);

        conn.execute_batch("BEGIN")?;
        let result = (|| -> Result<()> {
            conn.execute(
                "INSERT INTO cron_jobs (
                    id, name, enabled, schedule_type,
                    at_ms, every_ms, cron_expr, cron_tz, event_pattern, event_channel,
                    payload_kind, payload_message, agent_echo,
                    next_run_at_ms, last_run_at_ms, last_status, last_error,
                    run_count, last_fired_at_ms,
                    created_at_ms, updated_at_ms, delete_after_run,
                    expires_at_ms, max_runs, cooldown_secs, max_concurrent
                ) VALUES (
                    ?1, ?2, ?3, ?4,
                    ?5, ?6, ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13,
                    ?14, ?15, ?16, ?17,
                    ?18, ?19,
                    ?20, ?21, ?22,
                    ?23, ?24, ?25, ?26
                )",
                params![
                    job.id,
                    job.name,
                    job.enabled,
                    stype,
                    cols.at_ms,
                    cols.every_ms,
                    cols.cron_expr,
                    cols.cron_tz,
                    cols.event_pattern,
                    cols.event_channel,
                    job.payload.kind,
                    job.payload.message,
                    job.payload.agent_echo,
                    job.state.next_run_at_ms,
                    job.state.last_run_at_ms,
                    job.state.last_status,
                    job.state.last_error,
                    job.state.run_count,
                    job.state.last_fired_at_ms,
                    job.created_at_ms,
                    job.updated_at_ms,
                    job.delete_after_run,
                    job.expires_at_ms,
                    job.max_runs,
                    job.cooldown_secs.map(|v| v as i64),
                    job.max_concurrent,
                ],
            )?;

            for target in &job.payload.targets {
                conn.execute(
                    "INSERT INTO cron_job_targets (job_id, channel, target)
                     VALUES (?1, ?2, ?3)",
                    params![job.id, target.channel, target.to],
                )?;
            }
            Ok(())
        })();

        if result.is_ok() {
            conn.execute_batch("COMMIT")?;
        } else {
            let _ = conn.execute_batch("ROLLBACK");
        }
        result
    }

    pub fn delete_cron_job(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let deleted = conn.execute("DELETE FROM cron_jobs WHERE id = ?1", params![id])?;
        Ok(deleted > 0)
    }

    pub fn list_cron_jobs(&self, include_disabled: bool) -> Result<Vec<CronJob>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;

        let sql = if include_disabled {
            "SELECT id, name, enabled, schedule_type,
                    at_ms, every_ms, cron_expr, cron_tz, event_pattern, event_channel,
                    payload_kind, payload_message, agent_echo,
                    next_run_at_ms, last_run_at_ms, last_status, last_error,
                    run_count, last_fired_at_ms,
                    created_at_ms, updated_at_ms, delete_after_run,
                    expires_at_ms, max_runs, cooldown_secs, max_concurrent
             FROM cron_jobs ORDER BY created_at_ms"
        } else {
            "SELECT id, name, enabled, schedule_type,
                    at_ms, every_ms, cron_expr, cron_tz, event_pattern, event_channel,
                    payload_kind, payload_message, agent_echo,
                    next_run_at_ms, last_run_at_ms, last_status, last_error,
                    run_count, last_fired_at_ms,
                    created_at_ms, updated_at_ms, delete_after_run,
                    expires_at_ms, max_runs, cooldown_secs, max_concurrent
             FROM cron_jobs WHERE enabled = 1 ORDER BY created_at_ms"
        };

        // Load targets only for jobs matching the filter
        let target_sql = if include_disabled {
            "SELECT job_id, channel, target FROM cron_job_targets ORDER BY rowid"
        } else {
            "SELECT t.job_id, t.channel, t.target FROM cron_job_targets t
             INNER JOIN cron_jobs j ON j.id = t.job_id
             WHERE j.enabled = 1
             ORDER BY t.rowid"
        };
        let mut target_map: HashMap<String, Vec<CronTarget>> = HashMap::new();
        {
            let mut stmt = conn.prepare(target_sql)?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    CronTarget {
                        channel: row.get(1)?,
                        to: row.get(2)?,
                    },
                ))
            })?;
            for row in rows {
                let (job_id, target) = row?;
                target_map.entry(job_id).or_default().push(target);
            }
        }

        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(CronJobRow {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get(2)?,
                schedule_type: row.get(3)?,
                at_ms: row.get(4)?,
                every_ms: row.get(5)?,
                cron_expr: row.get(6)?,
                cron_tz: row.get(7)?,
                event_pattern: row.get(8)?,
                event_channel: row.get(9)?,
                payload_kind: row.get(10)?,
                payload_message: row.get(11)?,
                agent_echo: row.get(12)?,
                next_run_at_ms: row.get(13)?,
                last_run_at_ms: row.get(14)?,
                last_status: row.get(15)?,
                last_error: row.get(16)?,
                run_count: row.get(17)?,
                last_fired_at_ms: row.get(18)?,
                created_at_ms: row.get(19)?,
                updated_at_ms: row.get(20)?,
                delete_after_run: row.get(21)?,
                expires_at_ms: row.get(22)?,
                max_runs: row.get(23)?,
                cooldown_secs: row.get(24)?,
                max_concurrent: row.get(25)?,
            })
        })?;

        let mut jobs = Vec::new();
        for row in rows {
            let r = row?;
            let schedule = schedule_from_row(
                &r.schedule_type,
                r.at_ms,
                r.every_ms,
                r.cron_expr,
                r.cron_tz,
                r.event_pattern,
                r.event_channel,
            )
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
            let targets = target_map.remove(&r.id).unwrap_or_default();
            jobs.push(CronJob {
                id: r.id,
                name: r.name,
                enabled: r.enabled,
                schedule,
                payload: CronPayload {
                    kind: r.payload_kind,
                    message: r.payload_message,
                    agent_echo: r.agent_echo,
                    targets,
                },
                state: CronJobState {
                    next_run_at_ms: r.next_run_at_ms,
                    last_run_at_ms: r.last_run_at_ms,
                    last_status: r.last_status,
                    last_error: r.last_error,
                    run_count: r.run_count,
                    last_fired_at_ms: r.last_fired_at_ms,
                },
                created_at_ms: r.created_at_ms,
                updated_at_ms: r.updated_at_ms,
                delete_after_run: r.delete_after_run,
                expires_at_ms: r.expires_at_ms,
                max_runs: r.max_runs,
                cooldown_secs: r.cooldown_secs.map(|v| v as u64),
                max_concurrent: r.max_concurrent,
            });
        }

        Ok(jobs)
    }

    pub fn get_cron_job(&self, id: &str) -> Result<Option<CronJob>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;

        let mut stmt = conn.prepare(
            "SELECT id, name, enabled, schedule_type,
                    at_ms, every_ms, cron_expr, cron_tz, event_pattern, event_channel,
                    payload_kind, payload_message, agent_echo,
                    next_run_at_ms, last_run_at_ms, last_status, last_error,
                    run_count, last_fired_at_ms,
                    created_at_ms, updated_at_ms, delete_after_run,
                    expires_at_ms, max_runs, cooldown_secs, max_concurrent
             FROM cron_jobs WHERE id = ?1",
        )?;

        let mut rows = stmt.query(params![id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };

        let schedule_type: String = row.get(3)?;
        let schedule = schedule_from_row(
            &schedule_type,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
            row.get(8)?,
            row.get(9)?,
        )?;

        let job_id: String = row.get(0)?;

        // Load targets for this job
        let mut target_stmt = conn.prepare(
            "SELECT channel, target FROM cron_job_targets WHERE job_id = ?1 ORDER BY rowid",
        )?;
        let targets = target_stmt
            .query_map(params![job_id], |r| {
                Ok(CronTarget {
                    channel: r.get(0)?,
                    to: r.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let cooldown_secs: Option<i64> = row.get(24)?;
        Ok(Some(CronJob {
            id: job_id,
            name: row.get(1)?,
            enabled: row.get(2)?,
            schedule,
            payload: CronPayload {
                kind: row.get(10)?,
                message: row.get(11)?,
                agent_echo: row.get(12)?,
                targets,
            },
            state: CronJobState {
                next_run_at_ms: row.get(13)?,
                last_run_at_ms: row.get(14)?,
                last_status: row.get(15)?,
                last_error: row.get(16)?,
                run_count: row.get(17)?,
                last_fired_at_ms: row.get(18)?,
            },
            created_at_ms: row.get(19)?,
            updated_at_ms: row.get(20)?,
            delete_after_run: row.get(21)?,
            expires_at_ms: row.get(22)?,
            max_runs: row.get(23)?,
            cooldown_secs: cooldown_secs.map(|v| v as u64),
            max_concurrent: row.get(25)?,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_cron_job_state(
        &self,
        id: &str,
        last_status: Option<&str>,
        last_error: Option<&str>,
        run_count: u32,
        next_run_at_ms: Option<i64>,
        last_run_at_ms: Option<i64>,
        last_fired_at_ms: Option<i64>,
        updated_at_ms: i64,
    ) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let updated = conn.execute(
            "UPDATE cron_jobs SET
                last_status = ?1, last_error = ?2, run_count = ?3,
                next_run_at_ms = ?4, last_run_at_ms = ?5, last_fired_at_ms = ?6,
                updated_at_ms = ?7
             WHERE id = ?8",
            params![
                last_status,
                last_error,
                run_count,
                next_run_at_ms,
                last_run_at_ms,
                last_fired_at_ms,
                updated_at_ms,
                id,
            ],
        )?;
        Ok(updated > 0)
    }

    /// Atomically mark a job as firing: increment `run_count` via SQL (no
    /// read-modify-write race), set status to running, and update timestamps.
    pub fn fire_cron_job(
        &self,
        id: &str,
        next_run_at_ms: Option<i64>,
        last_run_at_ms: i64,
        updated_at_ms: i64,
    ) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let updated = conn.execute(
            "UPDATE cron_jobs SET
                last_status = 'running', last_error = NULL,
                run_count = run_count + 1,
                next_run_at_ms = ?1, last_run_at_ms = ?2, updated_at_ms = ?3
             WHERE id = ?4",
            params![next_run_at_ms, last_run_at_ms, updated_at_ms, id],
        )?;
        Ok(updated > 0)
    }

    pub fn update_cron_job_enabled(
        &self,
        id: &str,
        enabled: bool,
        next_run_at_ms: Option<i64>,
        updated_at_ms: i64,
    ) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let updated = conn.execute(
            "UPDATE cron_jobs SET enabled = ?1, next_run_at_ms = ?2, updated_at_ms = ?3
             WHERE id = ?4",
            params![enabled, next_run_at_ms, updated_at_ms, id],
        )?;
        Ok(updated > 0)
    }

    /// Update a cron job's fields. Only `Some` fields are updated.
    /// When `next_run_at_ms` is `Some`, it is also set (used when schedule changes
    /// require recomputing the next run time in the same write).
    pub fn update_cron_job(
        &self,
        id: &str,
        params_upd: &UpdateJobParams,
        next_run_at_ms: Option<Option<i64>>,
        updated_at_ms: i64,
    ) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;

        // Build dynamic SET clause
        let mut set_clauses = vec!["updated_at_ms = ?1".to_string()];
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(updated_at_ms)];

        if let Some(name) = &params_upd.name {
            param_values.push(Box::new(name.clone()));
            set_clauses.push(format!("name = ?{}", param_values.len()));
        }

        if let Some(message) = &params_upd.message {
            param_values.push(Box::new(message.clone()));
            set_clauses.push(format!("payload_message = ?{}", param_values.len()));
        }

        if let Some(agent_echo) = params_upd.agent_echo {
            param_values.push(Box::new(agent_echo));
            set_clauses.push(format!("agent_echo = ?{}", param_values.len()));
        }

        if let Some(schedule) = &params_upd.schedule {
            let stype = schedule_type_str(schedule);
            let cols = schedule_columns(schedule);

            param_values.push(Box::new(stype.to_string()));
            set_clauses.push(format!("schedule_type = ?{}", param_values.len()));

            param_values.push(Box::new(cols.at_ms));
            set_clauses.push(format!("at_ms = ?{}", param_values.len()));

            param_values.push(Box::new(cols.every_ms));
            set_clauses.push(format!("every_ms = ?{}", param_values.len()));

            param_values.push(Box::new(cols.cron_expr.map(ToString::to_string)));
            set_clauses.push(format!("cron_expr = ?{}", param_values.len()));

            param_values.push(Box::new(cols.cron_tz.map(ToString::to_string)));
            set_clauses.push(format!("cron_tz = ?{}", param_values.len()));

            param_values.push(Box::new(cols.event_pattern.map(ToString::to_string)));
            set_clauses.push(format!("event_pattern = ?{}", param_values.len()));

            param_values.push(Box::new(cols.event_channel.map(ToString::to_string)));
            set_clauses.push(format!("event_channel = ?{}", param_values.len()));
        }

        if let Some(next_run) = next_run_at_ms {
            param_values.push(Box::new(next_run));
            set_clauses.push(format!("next_run_at_ms = ?{}", param_values.len()));
        }

        // Add id as last parameter
        param_values.push(Box::new(id.to_string()));
        let id_idx = param_values.len();

        let sql = format!(
            "UPDATE cron_jobs SET {} WHERE id = ?{}",
            set_clauses.join(", "),
            id_idx
        );

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(AsRef::as_ref).collect();

        // Wrap UPDATE + target replacement in a single transaction
        conn.execute_batch("BEGIN")?;
        let result = (|| -> Result<bool> {
            let updated = conn.execute(&sql, params_ref.as_slice())?;

            if let Some(targets) = &params_upd.targets {
                conn.execute(
                    "DELETE FROM cron_job_targets WHERE job_id = ?1",
                    params![id],
                )?;
                for target in targets {
                    conn.execute(
                        "INSERT INTO cron_job_targets (job_id, channel, target)
                         VALUES (?1, ?2, ?3)",
                        params![id, target.channel, target.to],
                    )?;
                }
            }

            Ok(updated > 0)
        })();

        if result.is_ok() {
            conn.execute_batch("COMMIT")?;
        } else {
            let _ = conn.execute_batch("ROLLBACK");
        }
        result
    }

    /// Update only the completion status and error of a cron job.
    /// Does not touch `run_count`, `next_run_at_ms`, or other state fields,
    /// avoiding a read-modify-write race with the polling loop.
    pub fn update_cron_job_status(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "UPDATE cron_jobs SET last_status = ?1, last_error = ?2 WHERE id = ?3",
            params![status, error, id],
        )?;
        Ok(())
    }

    pub fn count_cron_jobs_by_name(&self, name: &str) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM cron_jobs WHERE LOWER(name) = LOWER(?1)",
            params![name],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Find the next available suffix number for a job name.
    /// Returns `None` if the base name is available. Returns `Some(n)` where
    /// the caller should use `"{name} ({n})"`.
    pub fn next_cron_job_name_suffix(&self, name: &str) -> Result<Option<u32>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let base_lower = name.to_lowercase();

        // Check if the base name is taken
        let base_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM cron_jobs WHERE LOWER(name) = LOWER(?1)",
            params![name],
            |row| row.get(0),
        )?;
        if base_count == 0 {
            return Ok(None);
        }

        // Fetch all existing names that match the base or the "{base} (N)" pattern.
        // Escape SQL LIKE wildcards in the base name to prevent false matches.
        let escaped = base_lower
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("{escaped}%");
        let mut stmt = conn
            .prepare("SELECT LOWER(name) FROM cron_jobs WHERE LOWER(name) LIKE ?1 ESCAPE '\\'")?;
        let existing: std::collections::HashSet<String> = stmt
            .query_map(params![pattern], |row| row.get(0))?
            .filter_map(Result::ok)
            .collect();

        for n in 2..10_002u32 {
            let candidate = format!("{base_lower} ({n})");
            if !existing.contains(&candidate) {
                return Ok(Some(n));
            }
        }
        anyhow::bail!("unable to find unique name suffix after 10000 attempts")
    }

    pub fn prune_disabled_cron_jobs(&self, cutoff_ms: i64) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let deleted = conn.execute(
            "DELETE FROM cron_jobs WHERE enabled = 0 AND updated_at_ms < ?1",
            params![cutoff_ms],
        )?;
        Ok(deleted)
    }

    pub fn recover_running_cron_jobs(&self) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let updated = conn.execute(
            "UPDATE cron_jobs SET last_status = 'interrupted',
                last_error = 'process restarted while job was running'
             WHERE last_status = 'running'",
            [],
        )?;
        Ok(updated)
    }
}

/// Internal row struct for mapping query results.
struct CronJobRow {
    id: String,
    name: String,
    enabled: bool,
    schedule_type: String,
    at_ms: Option<i64>,
    every_ms: Option<i64>,
    cron_expr: Option<String>,
    cron_tz: Option<String>,
    event_pattern: Option<String>,
    event_channel: Option<String>,
    payload_kind: String,
    payload_message: String,
    agent_echo: bool,
    next_run_at_ms: Option<i64>,
    last_run_at_ms: Option<i64>,
    last_status: Option<String>,
    last_error: Option<String>,
    run_count: u32,
    last_fired_at_ms: Option<i64>,
    created_at_ms: i64,
    updated_at_ms: i64,
    delete_after_run: bool,
    expires_at_ms: Option<i64>,
    max_runs: Option<u32>,
    cooldown_secs: Option<i64>,
    max_concurrent: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::super::MemoryDB;
    use crate::cron::types::{
        CronJob, CronJobState, CronPayload, CronSchedule, CronTarget, UpdateJobParams,
    };

    fn make_test_job(id: &str, name: &str, schedule: CronSchedule) -> CronJob {
        CronJob {
            id: id.to_string(),
            name: name.to_string(),
            enabled: true,
            schedule,
            payload: CronPayload {
                kind: "agent_turn".to_string(),
                message: "hello world".to_string(),
                agent_echo: true,
                targets: vec![CronTarget {
                    channel: "slack".to_string(),
                    to: "C123".to_string(),
                }],
            },
            state: CronJobState::default(),
            created_at_ms: 1000,
            updated_at_ms: 1000,
            delete_after_run: false,
            expires_at_ms: None,
            max_runs: None,
            cooldown_secs: None,
            max_concurrent: None,
        }
    }

    #[test]
    fn test_insert_and_list_cron_job() {
        let db = MemoryDB::new(":memory:").unwrap();
        let job = make_test_job(
            "job-1",
            "daily check",
            CronSchedule::Cron {
                expr: Some("0 9 * * *".to_string()),
                tz: Some("UTC".to_string()),
            },
        );

        db.insert_cron_job(&job).unwrap();

        let jobs = db.list_cron_jobs(true).unwrap();
        assert_eq!(jobs.len(), 1);

        let got = &jobs[0];
        assert_eq!(got.id, "job-1");
        assert_eq!(got.name, "daily check");
        assert!(got.enabled);
        assert_eq!(got.payload.kind, "agent_turn");
        assert_eq!(got.payload.message, "hello world");
        assert!(got.payload.agent_echo);
        assert_eq!(got.payload.targets.len(), 1);
        assert_eq!(got.payload.targets[0].channel, "slack");
        assert_eq!(got.payload.targets[0].to, "C123");
        assert_eq!(got.created_at_ms, 1000);
        assert_eq!(got.updated_at_ms, 1000);

        if let CronSchedule::Cron { expr, tz } = &got.schedule {
            assert_eq!(expr.as_deref(), Some("0 9 * * *"));
            assert_eq!(tz.as_deref(), Some("UTC"));
        } else {
            panic!("expected Cron schedule");
        }
    }

    #[test]
    fn test_delete_cron_job() {
        let db = MemoryDB::new(":memory:").unwrap();
        let job = make_test_job(
            "job-del",
            "to delete",
            CronSchedule::Every {
                every_ms: Some(60000),
            },
        );
        db.insert_cron_job(&job).unwrap();

        assert!(db.delete_cron_job("job-del").unwrap());
        assert!(!db.delete_cron_job("job-del").unwrap());

        let jobs = db.list_cron_jobs(true).unwrap();
        assert!(jobs.is_empty());
    }

    #[test]
    fn test_get_cron_job() {
        let db = MemoryDB::new(":memory:").unwrap();
        let job = make_test_job(
            "job-get",
            "get me",
            CronSchedule::At {
                at_ms: Some(999_999),
            },
        );
        db.insert_cron_job(&job).unwrap();

        let got = db.get_cron_job("job-get").unwrap().unwrap();
        assert_eq!(got.id, "job-get");
        assert_eq!(got.name, "get me");
        assert_eq!(got.payload.targets.len(), 1);

        if let CronSchedule::At { at_ms } = &got.schedule {
            assert_eq!(*at_ms, Some(999_999));
        } else {
            panic!("expected At schedule");
        }

        // Nonexistent returns None
        assert!(db.get_cron_job("no-such-job").unwrap().is_none());
    }

    #[test]
    fn test_list_excludes_disabled() {
        let db = MemoryDB::new(":memory:").unwrap();

        let mut enabled_job = make_test_job(
            "job-e",
            "enabled",
            CronSchedule::Every {
                every_ms: Some(5000),
            },
        );
        enabled_job.enabled = true;
        db.insert_cron_job(&enabled_job).unwrap();

        let mut disabled_job = make_test_job(
            "job-d",
            "disabled",
            CronSchedule::Every {
                every_ms: Some(5000),
            },
        );
        disabled_job.enabled = false;
        db.insert_cron_job(&disabled_job).unwrap();

        // include_disabled=false should only return enabled
        let jobs = db.list_cron_jobs(false).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "job-e");

        // include_disabled=true should return both
        let all = db.list_cron_jobs(true).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_update_cron_job_state() {
        let db = MemoryDB::new(":memory:").unwrap();
        let job = make_test_job(
            "job-state",
            "state test",
            CronSchedule::Every {
                every_ms: Some(1000),
            },
        );
        db.insert_cron_job(&job).unwrap();

        let ok = db
            .update_cron_job_state(
                "job-state",
                Some("success"),
                None,
                5,
                Some(2000),
                Some(1500),
                Some(1500),
                2000,
            )
            .unwrap();
        assert!(ok);

        let got = db.get_cron_job("job-state").unwrap().unwrap();
        assert_eq!(got.state.last_status.as_deref(), Some("success"));
        assert!(got.state.last_error.is_none());
        assert_eq!(got.state.run_count, 5);
        assert_eq!(got.state.next_run_at_ms, Some(2000));
        assert_eq!(got.state.last_run_at_ms, Some(1500));
        assert_eq!(got.state.last_fired_at_ms, Some(1500));
        assert_eq!(got.updated_at_ms, 2000);
    }

    #[test]
    fn test_update_cron_job_enabled() {
        let db = MemoryDB::new(":memory:").unwrap();
        let job = make_test_job(
            "job-en",
            "enable test",
            CronSchedule::Every {
                every_ms: Some(1000),
            },
        );
        db.insert_cron_job(&job).unwrap();

        let ok = db
            .update_cron_job_enabled("job-en", false, None, 3000)
            .unwrap();
        assert!(ok);

        let got = db.get_cron_job("job-en").unwrap().unwrap();
        assert!(!got.enabled);
        assert!(got.state.next_run_at_ms.is_none());
        assert_eq!(got.updated_at_ms, 3000);
    }

    #[test]
    fn test_update_cron_job_partial() {
        let db = MemoryDB::new(":memory:").unwrap();
        let job = make_test_job(
            "job-partial",
            "old name",
            CronSchedule::Every {
                every_ms: Some(1000),
            },
        );
        db.insert_cron_job(&job).unwrap();

        let params = UpdateJobParams {
            name: Some("new name".to_string()),
            ..Default::default()
        };
        let ok = db
            .update_cron_job("job-partial", &params, None, 5000)
            .unwrap();
        assert!(ok);

        let got = db.get_cron_job("job-partial").unwrap().unwrap();
        assert_eq!(got.name, "new name");
        // Message should be unchanged
        assert_eq!(got.payload.message, "hello world");
        assert_eq!(got.updated_at_ms, 5000);
    }

    #[test]
    fn test_update_cron_job_targets() {
        let db = MemoryDB::new(":memory:").unwrap();
        let job = make_test_job(
            "job-targets",
            "targets test",
            CronSchedule::Every {
                every_ms: Some(1000),
            },
        );
        db.insert_cron_job(&job).unwrap();

        // Verify original targets
        let got = db.get_cron_job("job-targets").unwrap().unwrap();
        assert_eq!(got.payload.targets.len(), 1);
        assert_eq!(got.payload.targets[0].channel, "slack");

        // Update targets
        let params = UpdateJobParams {
            targets: Some(vec![
                CronTarget {
                    channel: "telegram".to_string(),
                    to: "12345".to_string(),
                },
                CronTarget {
                    channel: "discord".to_string(),
                    to: "67890".to_string(),
                },
            ]),
            ..Default::default()
        };
        db.update_cron_job("job-targets", &params, None, 6000)
            .unwrap();

        let got = db.get_cron_job("job-targets").unwrap().unwrap();
        assert_eq!(got.payload.targets.len(), 2);

        let channels: Vec<&str> = got
            .payload
            .targets
            .iter()
            .map(|t| t.channel.as_str())
            .collect();
        assert!(channels.contains(&"telegram"));
        assert!(channels.contains(&"discord"));
    }

    #[test]
    fn test_count_cron_jobs_by_name() {
        let db = MemoryDB::new(":memory:").unwrap();

        let job1 = make_test_job(
            "job-c1",
            "Daily Report",
            CronSchedule::Every {
                every_ms: Some(1000),
            },
        );
        let job2 = make_test_job(
            "job-c2",
            "daily report",
            CronSchedule::Every {
                every_ms: Some(2000),
            },
        );
        let job3 = make_test_job(
            "job-c3",
            "other job",
            CronSchedule::Every {
                every_ms: Some(3000),
            },
        );
        db.insert_cron_job(&job1).unwrap();
        db.insert_cron_job(&job2).unwrap();
        db.insert_cron_job(&job3).unwrap();

        assert_eq!(db.count_cron_jobs_by_name("daily report").unwrap(), 2);
        assert_eq!(db.count_cron_jobs_by_name("DAILY REPORT").unwrap(), 2);
        assert_eq!(db.count_cron_jobs_by_name("other job").unwrap(), 1);
        assert_eq!(db.count_cron_jobs_by_name("nonexistent").unwrap(), 0);
    }

    #[test]
    fn test_prune_disabled_cron_jobs() {
        let db = MemoryDB::new(":memory:").unwrap();

        // Old disabled job (should be pruned)
        let mut old = make_test_job(
            "old-disabled",
            "old",
            CronSchedule::Every {
                every_ms: Some(1000),
            },
        );
        old.enabled = false;
        old.updated_at_ms = 100;
        db.insert_cron_job(&old).unwrap();

        // Recent disabled job (should survive)
        let mut recent = make_test_job(
            "recent-disabled",
            "recent",
            CronSchedule::Every {
                every_ms: Some(1000),
            },
        );
        recent.enabled = false;
        recent.updated_at_ms = 9000;
        db.insert_cron_job(&recent).unwrap();

        // Enabled job (should survive)
        let mut enabled = make_test_job(
            "enabled-job",
            "enabled",
            CronSchedule::Every {
                every_ms: Some(1000),
            },
        );
        enabled.updated_at_ms = 100;
        db.insert_cron_job(&enabled).unwrap();

        let pruned = db.prune_disabled_cron_jobs(5000).unwrap();
        assert_eq!(pruned, 1);

        let all = db.list_cron_jobs(true).unwrap();
        assert_eq!(all.len(), 2);
        let ids: Vec<&str> = all.iter().map(|j| j.id.as_str()).collect();
        assert!(ids.contains(&"recent-disabled"));
        assert!(ids.contains(&"enabled-job"));
    }

    #[test]
    fn test_recover_running_cron_jobs() {
        let db = MemoryDB::new(":memory:").unwrap();

        let mut running = make_test_job(
            "job-run",
            "running",
            CronSchedule::Every {
                every_ms: Some(1000),
            },
        );
        running.state.last_status = Some("running".to_string());
        db.insert_cron_job(&running).unwrap();

        let mut ok = make_test_job(
            "job-ok",
            "ok",
            CronSchedule::Every {
                every_ms: Some(1000),
            },
        );
        ok.state.last_status = Some("success".to_string());
        db.insert_cron_job(&ok).unwrap();

        let recovered = db.recover_running_cron_jobs().unwrap();
        assert_eq!(recovered, 1);

        let got = db.get_cron_job("job-run").unwrap().unwrap();
        assert_eq!(got.state.last_status.as_deref(), Some("interrupted"));
        assert_eq!(
            got.state.last_error.as_deref(),
            Some("process restarted while job was running")
        );

        // The "success" job should be unchanged
        let ok_got = db.get_cron_job("job-ok").unwrap().unwrap();
        assert_eq!(ok_got.state.last_status.as_deref(), Some("success"));
    }
}
