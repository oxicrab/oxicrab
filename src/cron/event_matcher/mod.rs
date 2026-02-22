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
    /// Returns jobs that should fire, respecting channel filter, cooldown,
    /// expiry (`expires_at_ms`), and run limits (`max_runs`).
    /// Updates local fired timestamps so cooldowns work across calls.
    pub fn check_message(&mut self, content: &str, channel: &str, now_ms: i64) -> Vec<CronJob> {
        let mut matched = Vec::new();
        for (id, regex, channel_filter, job) in &mut self.matchers {
            // Expiry check: skip if past expires_at
            if job.expires_at_ms.is_some_and(|exp| exp <= now_ms) {
                continue;
            }

            // Max runs check: skip if exhausted
            if job.max_runs.is_some_and(|max| job.state.run_count >= max) {
                continue;
            }

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
                && let Some(&last_fired) = self.last_fired.get(id.as_str())
            {
                let elapsed_secs = (now_ms - last_fired) / 1000;
                if elapsed_secs < cooldown as i64 {
                    continue;
                }
            }

            // Update local fired timestamp and run count
            self.last_fired.insert(id.clone(), now_ms);
            job.state.last_fired_at_ms = Some(now_ms);
            job.state.run_count = job.state.run_count.saturating_add(1);
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
mod tests;
