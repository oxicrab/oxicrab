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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronPayload {
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub message: String,
    #[serde(default, rename = "agentEcho")]
    pub agent_echo: bool,
    pub channel: Option<String>,
    pub to: Option<String>,
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
    pub channel: Option<String>,
    pub to: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron_schedule_at_roundtrip() {
        let schedule = CronSchedule::At {
            at_ms: Some(1234567890),
        };
        let json = serde_json::to_string(&schedule).unwrap();
        let deserialized: CronSchedule = serde_json::from_str(&json).unwrap();

        match deserialized {
            CronSchedule::At { at_ms } => assert_eq!(at_ms, Some(1234567890)),
            _ => panic!("Expected At variant"),
        }
    }

    #[test]
    fn test_cron_schedule_every_roundtrip() {
        let schedule = CronSchedule::Every {
            every_ms: Some(60000),
        };
        let json = serde_json::to_string(&schedule).unwrap();
        let deserialized: CronSchedule = serde_json::from_str(&json).unwrap();

        match deserialized {
            CronSchedule::Every { every_ms } => assert_eq!(every_ms, Some(60000)),
            _ => panic!("Expected Every variant"),
        }
    }

    #[test]
    fn test_cron_schedule_cron_roundtrip() {
        let schedule = CronSchedule::Cron {
            expr: Some("0 0 * * *".to_string()),
            tz: Some("America/New_York".to_string()),
        };
        let json = serde_json::to_string(&schedule).unwrap();
        let deserialized: CronSchedule = serde_json::from_str(&json).unwrap();

        match deserialized {
            CronSchedule::Cron { expr, tz } => {
                assert_eq!(expr, Some("0 0 * * *".to_string()));
                assert_eq!(tz, Some("America/New_York".to_string()));
            }
            _ => panic!("Expected Cron variant"),
        }
    }

    #[test]
    fn test_cron_store_default_version() {
        let store = CronStore {
            version: default_version(),
            jobs: vec![],
        };
        assert_eq!(store.version, 1);
    }

    #[test]
    fn test_cron_job_full_roundtrip() {
        let job = CronJob {
            id: "test-job-1".to_string(),
            name: "Test Job".to_string(),
            enabled: true,
            schedule: CronSchedule::Every {
                every_ms: Some(3600000),
            },
            payload: CronPayload {
                kind: "agent_turn".to_string(),
                message: "Hello World".to_string(),
                agent_echo: true,
                channel: Some("telegram".to_string()),
                to: Some("user123".to_string()),
            },
            state: CronJobState {
                next_run_at_ms: Some(9999999999),
                last_run_at_ms: Some(8888888888),
                last_status: Some("success".to_string()),
                last_error: None,
            },
            created_at_ms: 1234567890,
            updated_at_ms: 1234567900,
            delete_after_run: false,
        };

        let json = serde_json::to_string(&job).unwrap();
        let deserialized: CronJob = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "test-job-1");
        assert_eq!(deserialized.name, "Test Job");
        assert!(deserialized.enabled);
        assert_eq!(deserialized.payload.kind, "agent_turn");
        assert_eq!(deserialized.payload.message, "Hello World");
        assert!(deserialized.payload.agent_echo);
        assert_eq!(deserialized.payload.channel, Some("telegram".to_string()));
        assert_eq!(deserialized.payload.to, Some("user123".to_string()));
        assert_eq!(deserialized.state.next_run_at_ms, Some(9999999999));
        assert_eq!(deserialized.state.last_run_at_ms, Some(8888888888));
        assert_eq!(deserialized.state.last_status, Some("success".to_string()));
        assert_eq!(deserialized.state.last_error, None);
        assert_eq!(deserialized.created_at_ms, 1234567890);
        assert_eq!(deserialized.updated_at_ms, 1234567900);
        assert!(!deserialized.delete_after_run);
    }

    #[test]
    fn test_cron_schedule_cron_missing_tz() {
        let schedule = CronSchedule::Cron {
            expr: Some("0 0 * * *".to_string()),
            tz: None,
        };
        let json = serde_json::to_string(&schedule).unwrap();
        let deserialized: CronSchedule = serde_json::from_str(&json).unwrap();

        match deserialized {
            CronSchedule::Cron { expr, tz } => {
                assert_eq!(expr, Some("0 0 * * *".to_string()));
                assert_eq!(tz, None);
            }
            _ => panic!("Expected Cron variant"),
        }
    }
}
