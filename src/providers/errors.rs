use crate::errors::OxicrabError;
use crate::providers::base::ProviderMetrics;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tracing::{error, warn};

/// Common error handling utilities for LLM providers
///
/// This module provides standardized error handling patterns for LLM providers.
/// Functions are designed to be used as static methods.
pub struct ProviderErrorHandler;

impl ProviderErrorHandler {
    /// Parse API error response and return a typed error
    pub fn parse_api_error(status: u16, error_text: &str) -> Result<(), OxicrabError> {
        // Try to parse error JSON if possible to provide better error messages
        if let Ok(error_json) = serde_json::from_str::<Value>(error_text)
            && let Some(err) = error_json.get("error")
        {
            let error_type = err
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let error_msg = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");

            // Provide helpful message for model not found errors
            if error_type == "not_found_error" && error_msg.contains("model:") {
                let model_name = error_msg.replace("model: ", "").trim().to_string();
                return Err(OxicrabError::Provider {
                    message: format!(
                        "Model '{}' not found. This model may be deprecated or incorrect.\n\
                            Please update your config file (~/.oxicrab/config.json) to use a valid model:\n\
                            - claude-sonnet-4-5-20250929 (recommended)\n\
                            - claude-haiku-4-5-20251001 (fastest)\n\
                            - claude-opus-4-5-20251101 (most capable)\n\
                            \n\
                            Or remove the 'model' field from your config to use the default.",
                        model_name
                    ),
                    retryable: false,
                });
            }

            let retryable = status == 500 || status == 502 || status == 503;
            return Err(OxicrabError::Provider {
                message: format!("API error ({}): {}", error_type, error_msg),
                retryable,
            });
        }

        let retryable = status == 500 || status == 502 || status == 503;
        Err(OxicrabError::Provider {
            message: format!("API error ({}): {}", status, error_text),
            retryable,
        })
    }

    /// Log and handle provider errors consistently
    pub fn log_and_handle_error(e: &anyhow::Error, provider_name: &str, operation: &str) {
        error!(
            "{} provider error during {}: {}",
            provider_name, operation, e
        );
    }

    /// Handle rate limiting errors
    pub fn handle_rate_limit(status: u16, retry_after: Option<u64>) -> Result<(), OxicrabError> {
        if let Some(seconds) = retry_after {
            warn!("Rate limit hit. Retry after {} seconds", seconds);
        } else {
            warn!("Rate limit hit (status: {})", status);
        }
        Err(OxicrabError::RateLimit { retry_after })
    }

    /// Handle authentication errors
    pub fn handle_auth_error(status: u16, error_text: &str) -> Result<(), OxicrabError> {
        warn!("Authentication error (status: {}): {}", status, error_text);
        Err(OxicrabError::Auth(format!(
            "Authentication failed. Please check your API key or credentials. Error: {}",
            error_text
        )))
    }

    /// Check HTTP status and return a typed error if the response is not successful.
    /// On error, consumes the response body to extract error details.
    /// On success, returns the response unchanged for further processing.
    pub async fn check_http_status(
        resp: reqwest::Response,
        provider: &str,
    ) -> Result<reqwest::Response, anyhow::Error> {
        if resp.status().is_success() {
            return Ok(resp);
        }

        let status = resp.status();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        let error_text = resp
            .text()
            .await
            .unwrap_or_else(|_| "unknown error".to_string());

        if status == 429 {
            Self::log_and_handle_error(&anyhow::anyhow!("Rate limit exceeded"), provider, "chat");
            return Err(Self::handle_rate_limit(status.as_u16(), retry_after)
                .unwrap_err()
                .into());
        }

        if status == 401 || status == 403 {
            Self::log_and_handle_error(&anyhow::anyhow!("Authentication failed"), provider, "chat");
            return Err(Self::handle_auth_error(status.as_u16(), &error_text)
                .unwrap_err()
                .into());
        }

        Self::log_and_handle_error(&anyhow::anyhow!("API error"), provider, "chat");
        Err(Self::parse_api_error(status.as_u16(), &error_text)
            .unwrap_err()
            .into())
    }

    /// Check an HTTP response for errors (rate limit, auth, generic API errors).
    /// Returns the response body as JSON on success, or a typed error on failure.
    pub async fn check_response(
        resp: reqwest::Response,
        provider: &str,
        metrics: &Arc<Mutex<ProviderMetrics>>,
    ) -> Result<Value, anyhow::Error> {
        let resp = Self::check_http_status(resp, provider).await?;

        let json: Value = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse {} API response: {}", provider, e))?;

        // Check for API-level errors in the JSON body
        if let Some(error_val) = json.get("error") {
            if let Ok(mut m) = metrics.lock() {
                m.error_count += 1;
            }
            let error_text =
                serde_json::to_string(error_val).unwrap_or_else(|_| "Unknown error".to_string());
            Self::log_and_handle_error(&anyhow::anyhow!("API error in response"), provider, "chat");
            return Err(Self::parse_api_error(200, &error_text).unwrap_err().into());
        }

        Ok(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::OxicrabError;

    #[test]
    fn test_parse_api_error_with_json_body() {
        let error_json = r#"{"error": {"type": "invalid_request", "message": "bad request"}}"#;
        let result = ProviderErrorHandler::parse_api_error(400, error_json);
        let err = result.unwrap_err();
        match err {
            OxicrabError::Provider { message, retryable } => {
                assert!(message.contains("invalid_request"));
                assert!(message.contains("bad request"));
                assert!(!retryable);
            }
            _ => panic!("expected Provider error, got {:?}", err),
        }
    }

    #[test]
    fn test_parse_api_error_retryable_500() {
        let error_json = r#"{"error": {"type": "server_error", "message": "internal"}}"#;
        let result = ProviderErrorHandler::parse_api_error(500, error_json);
        let err = result.unwrap_err();
        match err {
            OxicrabError::Provider { retryable, .. } => assert!(retryable),
            _ => panic!("expected Provider error"),
        }
    }

    #[test]
    fn test_parse_api_error_retryable_502() {
        let error_json = r#"{"error": {"type": "overloaded", "message": "busy"}}"#;
        let result = ProviderErrorHandler::parse_api_error(502, error_json);
        let err = result.unwrap_err();
        match err {
            OxicrabError::Provider { retryable, .. } => assert!(retryable),
            _ => panic!("expected Provider error"),
        }
    }

    #[test]
    fn test_parse_api_error_retryable_503() {
        let error_json = r#"{"error": {"type": "overloaded", "message": "busy"}}"#;
        let result = ProviderErrorHandler::parse_api_error(503, error_json);
        let err = result.unwrap_err();
        match err {
            OxicrabError::Provider { retryable, .. } => assert!(retryable),
            _ => panic!("expected Provider error"),
        }
    }

    #[test]
    fn test_parse_api_error_not_retryable_400() {
        let error_json = r#"{"error": {"type": "bad_request", "message": "invalid"}}"#;
        let result = ProviderErrorHandler::parse_api_error(400, error_json);
        let err = result.unwrap_err();
        match err {
            OxicrabError::Provider { retryable, .. } => assert!(!retryable),
            _ => panic!("expected Provider error"),
        }
    }

    #[test]
    fn test_parse_api_error_non_json_body() {
        let result = ProviderErrorHandler::parse_api_error(500, "plain text error");
        let err = result.unwrap_err();
        match err {
            OxicrabError::Provider { message, retryable } => {
                assert!(message.contains("500"));
                assert!(message.contains("plain text error"));
                assert!(retryable);
            }
            _ => panic!("expected Provider error"),
        }
    }

    #[test]
    fn test_parse_api_error_model_not_found() {
        let error_json =
            r#"{"error": {"type": "not_found_error", "message": "model: claude-old"}}"#;
        let result = ProviderErrorHandler::parse_api_error(404, error_json);
        let err = result.unwrap_err();
        match err {
            OxicrabError::Provider { message, retryable } => {
                assert!(message.contains("not found"));
                assert!(message.contains("claude-sonnet-4-5-20250929"));
                assert!(!retryable);
            }
            _ => panic!("expected Provider error"),
        }
    }

    #[test]
    fn test_handle_rate_limit_with_retry_after() {
        let result = ProviderErrorHandler::handle_rate_limit(429, Some(30));
        let err = result.unwrap_err();
        match err {
            OxicrabError::RateLimit { retry_after } => {
                assert_eq!(retry_after, Some(30));
            }
            _ => panic!("expected RateLimit error"),
        }
    }

    #[test]
    fn test_handle_rate_limit_without_retry_after() {
        let result = ProviderErrorHandler::handle_rate_limit(429, None);
        let err = result.unwrap_err();
        match err {
            OxicrabError::RateLimit { retry_after } => {
                assert_eq!(retry_after, None);
            }
            _ => panic!("expected RateLimit error"),
        }
    }

    #[test]
    fn test_handle_auth_error() {
        let result = ProviderErrorHandler::handle_auth_error(401, "invalid token");
        let err = result.unwrap_err();
        match err {
            OxicrabError::Auth(msg) => {
                assert!(msg.contains("invalid token"));
                assert!(msg.contains("Authentication failed"));
            }
            _ => panic!("expected Auth error"),
        }
    }
}
