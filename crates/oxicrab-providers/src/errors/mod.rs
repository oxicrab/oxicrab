use oxicrab_core::errors::OxicrabError;
use serde_json::Value;
use tracing::{error, warn};

/// Common error handling utilities for LLM providers.
///
/// Free functions for standardized error handling across all providers.
fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504 | 529)
}

pub struct ProviderErrorHandler;

impl ProviderErrorHandler {
    /// Parse API error response and return a typed error.
    pub fn parse_api_error(status: u16, error_text: &str) -> OxicrabError {
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

            if error_type == "not_found_error" && error_msg.contains("model:") {
                let model_name = error_msg.replace("model: ", "").trim().to_string();
                return OxicrabError::Provider {
                    message: format!(
                        "Model '{model_name}' not found. This model may be deprecated or incorrect.\n\
                            Please update your config file (~/.oxicrab/config.toml) to use a valid model:\n\
                            - claude-sonnet-4-6 (recommended)\n\
                            - claude-haiku-4-5-20251001 (fastest)\n\
                            - claude-opus-4-6 (most capable)\n\
                            \n\
                            Or remove the 'model' field from your config to use the default."
                    ),
                    retryable: false,
                };
            }

            let retryable = is_retryable_status(status);
            let safe_msg = &error_msg[..error_msg.floor_char_boundary(500)];
            return OxicrabError::Provider {
                message: format!("API error ({error_type}): {safe_msg}"),
                retryable,
            };
        }

        let retryable = is_retryable_status(status);
        let safe_text = &error_text[..error_text.floor_char_boundary(500)];
        OxicrabError::Provider {
            message: format!("API error ({status}): {safe_text}"),
            retryable,
        }
    }

    /// Log a provider error consistently.
    pub fn log_and_handle_error(e: &anyhow::Error, provider_name: &str, operation: &str) {
        error!(
            "{} provider error during {}: {}",
            provider_name, operation, e
        );
    }

    /// Handle rate limiting errors.
    pub fn handle_rate_limit(status: u16, retry_after: Option<u64>) -> OxicrabError {
        if let Some(seconds) = retry_after {
            warn!("Rate limit hit. Retry after {} seconds", seconds);
        } else {
            warn!("Rate limit hit (status: {})", status);
        }
        OxicrabError::RateLimit { retry_after }
    }

    /// Handle authentication errors.
    pub fn handle_auth_error(status: u16, error_text: &str) -> OxicrabError {
        warn!("Authentication error (status: {}): {}", status, error_text);
        OxicrabError::Auth(format!(
            "Authentication failed. Please check your API key or credentials. Error: {error_text}"
        ))
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

        if status == 429 || status == 529 {
            Self::log_and_handle_error(&anyhow::anyhow!("Rate limit exceeded"), provider, "chat");
            return Err(Self::handle_rate_limit(status.as_u16(), retry_after).into());
        }

        if status == 401 || status == 403 {
            Self::log_and_handle_error(&anyhow::anyhow!("Authentication failed"), provider, "chat");
            return Err(Self::handle_auth_error(status.as_u16(), &error_text).into());
        }

        Self::log_and_handle_error(&anyhow::anyhow!("API error"), provider, "chat");
        Err(Self::parse_api_error(status.as_u16(), &error_text).into())
    }

    /// Check an HTTP response for errors (rate limit, auth, generic API errors).
    /// Returns the response body as JSON on success, or a typed error on failure.
    pub async fn check_response(
        resp: reqwest::Response,
        provider: &str,
    ) -> Result<Value, anyhow::Error> {
        let resp = Self::check_http_status(resp, provider).await?;

        let json: Value = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse {provider} API response: {e}"))?;

        if let Some(error_val) = json.get("error") {
            let wrapper = serde_json::json!({"error": error_val});
            let error_text = wrapper.to_string();
            Self::log_and_handle_error(&anyhow::anyhow!("API error in response"), provider, "chat");
            return Err(Self::parse_api_error(200, &error_text).into());
        }

        Ok(json)
    }
}

#[cfg(test)]
mod tests;
