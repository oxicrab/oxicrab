#![allow(dead_code)]

use thiserror::Error;

/// Typed error hierarchy for nanobot.
///
/// Use at module boundaries (provider calls, tool execution, config validation, sessions).
/// Internal/leaf functions can continue using `anyhow::Result` — the `Internal` variant
/// allows seamless conversion via the `?` operator.
#[derive(Debug, Error)]
pub enum NanobotError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Provider error: {message}")]
    Provider { message: String, retryable: bool },

    #[error("Rate limit exceeded")]
    RateLimit { retry_after: Option<u64> },

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Tool error: {tool}: {message}")]
    Tool { tool: String, message: String },

    #[error("Session error: {0}")]
    Session(String),

    #[error("Channel error: {channel}: {message}")]
    Channel { channel: String, message: String },

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

/// Convenience alias for results using NanobotError.
pub type NanobotResult<T> = std::result::Result<T, NanobotError>;

impl NanobotError {
    /// Whether this error is retryable (rate limits, transient provider errors).
    pub fn is_retryable(&self) -> bool {
        match self {
            NanobotError::RateLimit { .. } => true,
            NanobotError::Provider { retryable, .. } => *retryable,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_error_display() {
        let err = NanobotError::Config("bad value".into());
        assert_eq!(err.to_string(), "Configuration error: bad value");
    }

    #[test]
    fn provider_error_display() {
        let err = NanobotError::Provider {
            message: "timeout".into(),
            retryable: true,
        };
        assert_eq!(err.to_string(), "Provider error: timeout");
        assert!(err.is_retryable());
    }

    #[test]
    fn rate_limit_retryable() {
        let err = NanobotError::RateLimit {
            retry_after: Some(30),
        };
        assert!(err.is_retryable());
    }

    #[test]
    fn auth_error_not_retryable() {
        let err = NanobotError::Auth("invalid key".into());
        assert!(!err.is_retryable());
    }

    #[test]
    fn tool_error_display() {
        let err = NanobotError::Tool {
            tool: "web_search".into(),
            message: "API down".into(),
        };
        assert_eq!(err.to_string(), "Tool error: web_search: API down");
    }

    #[test]
    fn internal_from_anyhow() {
        let anyhow_err = anyhow::anyhow!("something broke");
        let err: NanobotError = anyhow_err.into();
        assert!(matches!(err, NanobotError::Internal(_)));
        assert!(!err.is_retryable());
    }
}
