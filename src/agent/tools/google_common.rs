use crate::auth::google::GoogleCredentials;
use anyhow::Result;
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// Shared Google API client that handles authentication and HTTP requests.
/// Reuses a single `reqwest::Client` for connection pooling.
pub struct GoogleApiClient {
    credentials: Arc<Mutex<GoogleCredentials>>,
    client: Client,
    base_url: String,
}

impl GoogleApiClient {
    pub fn new(credentials: GoogleCredentials, base_url: &str) -> Self {
        Self {
            credentials: Arc::new(Mutex::new(credentials)),
            client: crate::utils::http::default_http_client(),
            base_url: base_url.to_string(),
        }
    }

    pub async fn get_access_token(&self) -> Result<String> {
        let mut creds = self.credentials.lock().await;
        if !creds.is_valid() {
            creds.refresh().await?;
        }
        Ok(creds.get_access_token().to_string())
    }

    pub async fn call(&self, endpoint: &str, method: &str, body: Option<Value>) -> Result<Value> {
        let token = self.get_access_token().await?;
        let url = format!("{}/{}", self.base_url, endpoint);

        let response = self
            .send_request(&url, method, &token, body.as_ref())
            .await?;

        // On 401, force refresh (server rejected the token) and retry once
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            info!("Google API returned 401, forcing token refresh and retrying");
            let new_token = {
                let mut creds = self.credentials.lock().await;
                creds.refresh().await?;
                creds.get_access_token().to_string()
            };
            let retry_response = self
                .send_request(&url, method, &new_token, body.as_ref())
                .await?;
            return Self::parse_response(retry_response).await;
        }

        Self::parse_response(response).await
    }

    /// Parse a Google API response, handling empty bodies (e.g. 204 No Content from DELETE).
    async fn parse_response(response: reqwest::Response) -> Result<Value> {
        let status = response.status();
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        // Read body before checking status so error details are preserved
        let text = response.text().await?;
        if !status.is_success() {
            let safe_text: String = text
                .lines()
                .filter(|line| {
                    let lower = line.to_lowercase();
                    !lower.contains("access_token")
                        && !lower.contains("refresh_token")
                        && !lower.contains("bearer")
                        && !lower.contains("client_secret")
                })
                .collect::<Vec<_>>()
                .join("\n")
                .chars()
                .take(500)
                .collect();
            anyhow::bail!("Google API error ({status}): {safe_text}");
        }
        if text.is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_str(&text)?)
    }

    async fn send_request(
        &self,
        url: &str,
        method: &str,
        token: &str,
        body: Option<&Value>,
    ) -> Result<reqwest::Response> {
        let mut request = match method {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            "PATCH" => self.client.patch(url),
            "DELETE" => self.client.delete(url),
            _ => return Err(anyhow::anyhow!("Unsupported HTTP method: {method}")),
        };

        request = request.header("Authorization", format!("Bearer {token}"));

        if let Some(body) = body {
            request = request.json(body);
        }

        Ok(request.send().await?)
    }
}
