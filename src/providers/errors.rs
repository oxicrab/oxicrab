use anyhow::Result;
use serde_json::Value;
use tracing::{error, warn};

/// Common error handling utilities for LLM providers
/// 
/// This module provides standardized error handling patterns for LLM providers.
/// Functions are designed to be used as static methods.
pub struct ProviderErrorHandler;

impl ProviderErrorHandler {
    /// Parse API error response and return a user-friendly error message
    pub fn parse_api_error(status: u16, error_text: &str) -> Result<()> {
        // Try to parse error JSON if possible to provide better error messages
        if let Ok(error_json) = serde_json::from_str::<Value>(error_text) {
            if let Some(error) = error_json.get("error") {
                let error_type = error
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let error_msg = error
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");

                // Provide helpful message for model not found errors
                if error_type == "not_found_error" && error_msg.contains("model:") {
                    let model_name = error_msg.replace("model: ", "").trim().to_string();
                    return Err(anyhow::anyhow!(
                        "Model '{}' not found. This model may be deprecated or incorrect.\n\
                        Please update your config file (~/.nanobot/config.json) to use a valid model:\n\
                        - claude-sonnet-4-5-20250929 (recommended)\n\
                        - claude-haiku-4-5-20251001 (fastest)\n\
                        - claude-opus-4-5-20251101 (most capable)\n\
                        \n\
                        Or remove the 'model' field from your config to use the default.",
                        model_name
                    ));
                }

                return Err(anyhow::anyhow!(
                    "API error ({}): {}",
                    error_type,
                    error_msg
                ));
            }
        }

        Err(anyhow::anyhow!(
            "API error ({}): {}",
            status,
            error_text
        ))
    }

    /// Log and handle provider errors consistently
    pub fn log_and_handle_error(e: &anyhow::Error, provider_name: &str, operation: &str) {
        error!(
            "{} provider error during {}: {}",
            provider_name,
            operation,
            e
        );
    }

    /// Handle rate limiting errors
    pub fn handle_rate_limit(status: u16, retry_after: Option<u64>) -> Result<()> {
        if let Some(seconds) = retry_after {
            warn!("Rate limit hit. Retry after {} seconds", seconds);
            Err(anyhow::anyhow!(
                "Rate limit exceeded. Retry after {} seconds",
                seconds
            ))
        } else {
            warn!("Rate limit hit (status: {})", status);
            Err(anyhow::anyhow!("Rate limit exceeded (status: {})", status))
        }
    }

    /// Handle authentication errors
    pub fn handle_auth_error(status: u16, error_text: &str) -> Result<()> {
        warn!("Authentication error (status: {}): {}", status, error_text);
        Err(anyhow::anyhow!(
            "Authentication failed. Please check your API key or credentials. Error: {}",
            error_text
        ))
    }
}
