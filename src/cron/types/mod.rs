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
