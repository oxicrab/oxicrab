use crate::credentials::GoogleCredentials;
use anyhow::Result;
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::info;

/// Shared Google API client that handles authentication and HTTP requests.
/// Reuses a single `reqwest::Client` for connection pooling.
///
/// Multiple `GoogleApiClient` instances (e.g. Gmail, Calendar, Tasks) should
/// share the same `Arc<Mutex<GoogleCredentials>>` so that a single token
/// refresh serves all tools — avoiding redundant API calls.
pub struct GoogleApiClient {
    credentials: Arc<Mutex<GoogleCredentials>>,
    client: Client,
    base_url: String,
}

impl GoogleApiClient {
    /// Create a new client with shared credentials.
    ///
    /// Prefer this over constructing per-tool credentials — all Google tools
    /// should share one `Arc<Mutex<GoogleCredentials>>`.
    pub fn new(credentials: Arc<Mutex<GoogleCredentials>>, base_url: &str) -> Self {
        Self {
            credentials,
            client: crate::utils::default_http_client(),
            base_url: base_url.to_string(),
        }
    }

    pub async fn get_access_token(&self) -> Result<String> {
        let mut creds = self.credentials.lock().await;
        if !creds.is_valid() {
            creds.refresh(&self.client).await?;
        }
        Ok(creds.get_access_token().to_string())
    }

    pub async fn call(&self, endpoint: &str, method: &str, body: Option<Value>) -> Result<Value> {
        let token = self.get_access_token().await?;
        let url = format!("{}/{}", self.base_url, endpoint);

        let response = self
            .send_request(&url, method, &token, body.as_ref())
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            info!("Google API returned 401, forcing token refresh and retrying");
            let new_token = {
                let mut creds = self.credentials.lock().await;
                creds.refresh(&self.client).await?;
                creds.get_access_token().to_string()
            };
            let retry_response = self
                .send_request(&url, method, &new_token, body.as_ref())
                .await?;
            return Self::parse_response(retry_response).await;
        }

        Self::parse_response(response).await
    }

    async fn parse_response(response: reqwest::Response) -> Result<Value> {
        let status = response.status();
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
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

    #[cfg(test)]
    fn with_base_url(base_url: &str) -> Self {
        let creds = GoogleCredentials {
            token: "test-token".to_string(),
            refresh_token: None,
            token_uri: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            scopes: vec![],
            expiry: Some(u64::MAX),
        };
        Self {
            credentials: Arc::new(Mutex::new(creds)),
            client: Client::new(),
            base_url: base_url.to_string(),
        }
    }

    pub fn shared_credentials(credentials: GoogleCredentials) -> Arc<Mutex<GoogleCredentials>> {
        Arc::new(Mutex::new(credentials))
    }

    /// Paginate a Google API list endpoint, collecting items across pages.
    pub async fn paginate(
        &self,
        base_endpoint: &str,
        items_field: &str,
        max_pages: usize,
        max_items: Option<usize>,
    ) -> Result<Vec<Value>> {
        let separator = if base_endpoint.contains('?') {
            '&'
        } else {
            '?'
        };
        let mut all_items: Vec<Value> = Vec::new();
        let mut page_token: Option<String> = None;

        for _ in 0..max_pages {
            let endpoint = match &page_token {
                Some(token) => format!(
                    "{base_endpoint}{separator}pageToken={}",
                    urlencoding::encode(token)
                ),
                None => base_endpoint.to_string(),
            };

            let data = self.call(&endpoint, "GET", None).await?;

            if let Some(items) = data[items_field].as_array() {
                all_items.extend(items.iter().cloned());
            }

            if let Some(cap) = max_items
                && all_items.len() >= cap
            {
                all_items.truncate(cap);
                break;
            }

            match data["nextPageToken"].as_str() {
                Some(token) if !token.is_empty() => {
                    page_token = Some(token.to_string());
                }
                _ => break,
            }
        }

        Ok(all_items)
    }

    async fn send_request(
        &self,
        url: &str,
        method: &str,
        token: &str,
        body: Option<&Value>,
    ) -> Result<reqwest::Response> {
        let http_method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| anyhow::anyhow!("invalid HTTP method: {method}"))?;
        let mut request = self
            .client
            .request(http_method, url)
            .header("Authorization", format!("Bearer {token}"));

        if let Some(body) = body {
            request = request.json(body);
        }

        Ok(request.timeout(Duration::from_secs(15)).send().await?)
    }
}

#[cfg(test)]
mod tests;
