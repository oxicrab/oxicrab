use crate::cron::types::{CronJob, CronSchedule};
use regex::Regex;
use std::collections::HashMap;
use tracing::warn;

/// Matches inbound messages against event-triggered cron jobs.
pub struct EventMatcher {
    matchers: Vec<(String, Regex, Option<String>, CronJob)>,
    /// Local tracking of when each job last fired, so cooldowns work across
    /// multiple `check_message` calls without rebuilding the matcher.
    last_fired: HashMap<String, i64>,
}

impl EventMatcher {
    /// Build from a list of cron jobs, filtering for `CronSchedule::Event` variants.
    /// Invalid regex patterns are skipped with a warning.
    pub fn from_jobs(jobs: &[CronJob]) -> Self {
        let mut matchers = Vec::new();
        for job in jobs {
            if !job.enabled {
                continue;
            }
            if let CronSchedule::Event {
                pattern: Some(pat),
                channel,
            } = &job.schedule
            {
                match Regex::new(pat) {
                    Ok(re) => {
                        matchers.push((job.id.clone(), re, channel.clone(), job.clone()));
                    }
                    Err(e) => {
                        warn!(
                            "Event cron job '{}' has invalid regex '{}': {}",
                            job.id, pat, e
                        );
                    }
                }
            }
        }
        // Initialize last_fired from job state snapshots
        let last_fired: HashMap<String, i64> = matchers
            .iter()
            .filter_map(|(id, _, _, job)| job.state.last_fired_at_ms.map(|ms| (id.clone(), ms)))
            .collect();

        Self {
            matchers,
            last_fired,
        }
    }

    /// Check a message against all event matchers.
    /// Returns jobs that should fire, respecting channel filter and cooldown.
    /// Updates local fired timestamps so cooldowns work across calls.
    pub fn check_message(&mut self, content: &str, channel: &str, now_ms: i64) -> Vec<CronJob> {
        let mut matched = Vec::new();
        for (id, regex, channel_filter, job) in &self.matchers {
            // Channel filter: skip if job is restricted to a different channel
            if let Some(required_channel) = channel_filter
                && required_channel != channel
            {
                continue;
            }

            // Regex match
            if !regex.is_match(content) {
                continue;
            }

            // Cooldown check: use local tracking (updated after each fire)
            if let Some(cooldown) = job.cooldown_secs
                && let Some(&last_fired) = self.last_fired.get(id)
            {
                let elapsed_secs = (now_ms - last_fired) / 1000;
                if elapsed_secs < cooldown as i64 {
                    continue;
                }
            }

            // Update local fired timestamp
            self.last_fired.insert(id.clone(), now_ms);
            matched.push(job.clone());
        }
        matched
    }

    /// Returns true if there are no event matchers.
    pub fn is_empty(&self) -> bool {
        self.matchers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cron::types::{CronJobState, CronPayload, CronTarget};

    fn make_event_job(id: &str, pattern: &str, channel: Option<&str>) -> CronJob {
        CronJob {
            id: id.to_string(),
            name: format!("Event {}", id),
            enabled: true,
            schedule: CronSchedule::Event {
                pattern: Some(pattern.to_string()),
                channel: channel.map(str::to_string),
            },
            payload: CronPayload {
                kind: "agent_turn".to_string(),
                message: "triggered".to_string(),
                agent_echo: false,
                targets: vec![CronTarget {
                    channel: "telegram".to_string(),
                    to: "user1".to_string(),
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
    fn test_basic_match() {
        let jobs = vec![make_event_job("e1", r"(?i)deploy", None)];
        let mut em = EventMatcher::from_jobs(&jobs);
        let hits = em.check_message("please deploy to prod", "slack", 1000);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "e1");
    }

    #[test]
    fn test_no_match() {
        let jobs = vec![make_event_job("e1", r"(?i)deploy", None)];
        let mut em = EventMatcher::from_jobs(&jobs);
        let hits = em.check_message("hello world", "slack", 1000);
        assert!(hits.is_empty());
    }

    #[test]
    fn test_channel_filter() {
        let jobs = vec![make_event_job("e1", r"deploy", Some("slack"))];
        let mut matcher = EventMatcher::from_jobs(&jobs);

        // Should match on slack
        assert_eq!(matcher.check_message("deploy now", "slack", 1000).len(), 1);
        // Should not match on discord
        assert!(
            matcher
                .check_message("deploy now", "discord", 1000)
                .is_empty()
        );
    }

    #[test]
    fn test_cooldown() {
        let mut job = make_event_job("e1", r"deploy", None);
        job.cooldown_secs = Some(60);
        job.state.last_fired_at_ms = Some(950_000); // fired 50s ago at now_ms=1000000

        let mut matcher = EventMatcher::from_jobs(&[job]);
        // Within cooldown (50s < 60s)
        assert!(
            matcher
                .check_message("deploy now", "slack", 1_000_000)
                .is_empty()
        );
        // After cooldown (70s > 60s)
        assert_eq!(
            matcher
                .check_message("deploy now", "slack", 1_020_000)
                .len(),
            1
        );
    }

    #[test]
    fn test_cooldown_tracks_across_calls() {
        let mut job = make_event_job("e1", r"deploy", None);
        job.cooldown_secs = Some(60);

        let mut matcher = EventMatcher::from_jobs(&[job]);
        // First call fires (no previous last_fired)
        assert_eq!(
            matcher
                .check_message("deploy now", "slack", 1_000_000)
                .len(),
            1
        );
        // Second call within cooldown should NOT fire
        assert!(
            matcher
                .check_message("deploy now", "slack", 1_030_000)
                .is_empty()
        );
        // Third call after cooldown should fire again
        assert_eq!(
            matcher
                .check_message("deploy now", "slack", 1_061_000)
                .len(),
            1
        );
    }

    #[test]
    fn test_disabled_job_ignored() {
        let mut job = make_event_job("e1", r"deploy", None);
        job.enabled = false;
        let matcher = EventMatcher::from_jobs(&[job]);
        assert!(matcher.is_empty());
    }

    #[test]
    fn test_invalid_regex_skipped() {
        let jobs = vec![
            make_event_job("e1", r"[invalid", None),
            make_event_job("e2", r"valid", None),
        ];
        let matcher = EventMatcher::from_jobs(&jobs);
        // Only the valid one should be registered
        assert_eq!(matcher.matchers.len(), 1);
        assert_eq!(matcher.matchers[0].0, "e2");
    }
}
