use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum CronSchedule {
    #[serde(rename = "at")]
    At {
        #[serde(rename = "atMs")]
        at_ms: Option<i64>,
    },
    #[serde(rename = "every")]
    Every {
        #[serde(rename = "everyMs")]
        every_ms: Option<i64>,
    },
    #[serde(rename = "cron")]
    Cron {
        expr: Option<String>,
        tz: Option<String>,
    },
    /// Fires when an inbound message matches the regex pattern.
    #[serde(rename = "event")]
    Event {
        /// Regex pattern to match against message content.
        pattern: Option<String>,
        /// Optional channel filter (only fire for messages from this channel).
        channel: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronTarget {
    pub channel: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronPayload {
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub message: String,
    #[serde(default, rename = "agentEcho")]
    pub agent_echo: bool,
    #[serde(default)]
    pub targets: Vec<CronTarget>,
}

fn default_kind() -> String {
    "agent_turn".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CronJobState {
    #[serde(rename = "nextRunAtMs")]
    pub next_run_at_ms: Option<i64>,
    #[serde(rename = "lastRunAtMs")]
    pub last_run_at_ms: Option<i64>,
    #[serde(rename = "lastStatus")]
    pub last_status: Option<String>,
    #[serde(rename = "lastError")]
    pub last_error: Option<String>,
    #[serde(rename = "runCount", default)]
    pub run_count: u32,
    /// Timestamp of last event-triggered firing (for cooldown enforcement).
    #[serde(
        rename = "lastFiredAtMs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_fired_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub schedule: CronSchedule,
    pub payload: CronPayload,
    #[serde(default)]
    pub state: CronJobState,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
    /// If true, disable the job after its first execution.
    /// Disabled jobs are eventually pruned from the store.
    #[serde(rename = "deleteAfterRun", default)]
    pub delete_after_run: bool,
    #[serde(
        rename = "expiresAtMs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub expires_at_ms: Option<i64>,
    #[serde(rename = "maxRuns", default, skip_serializing_if = "Option::is_none")]
    pub max_runs: Option<u32>,
    /// Minimum seconds between event-triggered firings.
    #[serde(
        rename = "cooldownSecs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub cooldown_secs: Option<u64>,
    /// Maximum concurrent executions for event-triggered jobs.
    /// Reserved for future use — not currently enforced by the scheduler.
    #[serde(
        rename = "maxConcurrent",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub max_concurrent: Option<u32>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronStore {
    #[serde(default = "default_version")]
    pub version: i32,
    #[serde(default)]
    pub jobs: Vec<CronJob>,
}

fn default_version() -> i32 {
    1
}

/// Parameters for updating an existing cron job.
#[derive(Debug, Default)]
pub struct UpdateJobParams {
    pub name: Option<String>,
    pub message: Option<String>,
    pub schedule: Option<CronSchedule>,
    pub agent_echo: Option<bool>,
    pub targets: Option<Vec<CronTarget>>,
}

#[cfg(test)]
mod tests;
