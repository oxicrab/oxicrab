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
            client: Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| Client::new()),
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

        // On 401, force token refresh and retry once
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            info!("Google API returned 401, refreshing token and retrying");
            let new_token = {
                let mut creds = self.credentials.lock().await;
                creds.refresh().await?;
                creds.get_access_token().to_string()
            };
            let retry_response = self
                .send_request(&url, method, &new_token, body.as_ref())
                .await?;
            let data: Value = retry_response.error_for_status()?.json().await?;
            return Ok(data);
        }

        let data: Value = response.error_for_status()?.json().await?;
        Ok(data)
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
            "DELETE" => self.client.delete(url),
            _ => return Err(anyhow::anyhow!("Unsupported HTTP method: {}", method)),
        };

        request = request.header("Authorization", format!("Bearer {}", token));

        if let Some(body) = body {
            request = request.json(body);
        }

        Ok(request.send().await?)
    }
}
