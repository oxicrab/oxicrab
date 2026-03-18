//! Google OAuth credentials with token refresh.

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Maximum token lifetime (1 hour) — ignore longer `expires_in` to prevent stale tokens.
const MAX_TOKEN_LIFETIME_SECS: u64 = 3600;
/// Refresh 60 seconds before expiry to avoid mid-request expiration.
const TOKEN_EXPIRY_BUFFER_SECS: u64 = 60;

#[derive(Clone, Serialize, Deserialize)]
pub struct GoogleCredentials {
    pub token: String,
    pub refresh_token: Option<String>,
    pub token_uri: String,
    pub client_id: String,
    pub client_secret: String,
    pub scopes: Vec<String>,
    pub expiry: Option<u64>,
}

impl std::fmt::Debug for GoogleCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleCredentials")
            .field("token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("token_uri", &self.token_uri)
            .field("client_id", &self.client_id)
            .field("client_secret", &"[REDACTED]")
            .field("scopes", &self.scopes)
            .field("expiry", &self.expiry)
            .finish()
    }
}

impl GoogleCredentials {
    pub fn is_valid(&self) -> bool {
        if let Some(expiry) = self.expiry {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .is_ok_and(|d| d.as_secs() < expiry)
        } else {
            false
        }
    }

    pub async fn refresh(&mut self) -> Result<()> {
        let refresh_token = self
            .refresh_token
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No refresh token available"))?;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());
        let mut params = HashMap::new();
        params.insert("refresh_token", refresh_token.clone());
        params.insert("client_id", self.client_id.clone());
        params.insert("client_secret", self.client_secret.clone());
        params.insert("grant_type", "refresh_token".to_string());

        let response = client.post(&self.token_uri).form(&params).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let error_code = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(anyhow::anyhow!("token refresh failed: {error_code}"));
        }

        let token_data: serde_json::Value = response.json().await?;

        if let Some(_error) = token_data.get("error") {
            let error_desc = token_data
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(anyhow::anyhow!("Token refresh failed: {error_desc}"));
        }

        self.token = token_data["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing access_token"))?
            .to_string();

        if let Some(refresh_token) = token_data.get("refresh_token").and_then(|v| v.as_str()) {
            self.refresh_token = Some(refresh_token.to_string());
        }

        if let Some(expires_in) = token_data
            .get("expires_in")
            .and_then(serde_json::Value::as_u64)
            && let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH)
        {
            let capped = expires_in.min(MAX_TOKEN_LIFETIME_SECS);
            self.expiry = Some(
                duration
                    .as_secs()
                    .saturating_add(capped.saturating_sub(TOKEN_EXPIRY_BUFFER_SECS)),
            );
        }

        Ok(())
    }

    pub fn get_access_token(&self) -> &str {
        &self.token
    }
}
