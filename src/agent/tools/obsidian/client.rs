use anyhow::Result;
use reqwest::Client;
use std::time::Duration;
use tracing::warn;

/// REST API client for the Obsidian Local REST API plugin.
pub struct ObsidianApiClient {
    base_url: String,
    api_key: String,
    client: Client,
}

impl ObsidianApiClient {
    pub fn new(base_url: &str, api_key: &str, timeout_secs: u64) -> Self {
        // Obsidian Local REST API uses a self-signed HTTPS certificate on localhost
        let is_localhost = base_url.contains("://127.0.0.1")
            || base_url.contains("://localhost")
            || base_url.contains("://[::1]");
        if !is_localhost {
            warn!(
                "obsidian API URL is not localhost — TLS certificate validation will be enforced"
            );
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .pool_max_idle_per_host(4)
            .danger_accept_invalid_certs(is_localhost)
            .build()
            .unwrap_or_default();

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            client,
        }
    }

    #[cfg(test)]
    pub fn with_base_url(base_url: String, api_key: &str) -> Self {
        Self {
            client: Client::builder().build().unwrap_or_default(),
            base_url,
            api_key: api_key.to_string(),
        }
    }

    /// Check if the Obsidian REST API is reachable.
    pub async fn is_reachable(&self) -> bool {
        let url = format!("{}/vault/", self.base_url);
        self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    /// List all files in the vault, recursing into subdirectories.
    pub async fn list_files(&self) -> Result<Vec<String>> {
        let mut all_files = Vec::new();
        let mut dirs_to_visit = vec![String::new()]; // start with root

        while let Some(dir) = dirs_to_visit.pop() {
            let url = format!("{}/vault/{}", self.base_url, urlencoding::encode(&dir));
            let resp = self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Accept", "application/json")
                .send()
                .await?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                warn!("Obsidian API {} listing '{}': {}", status, dir, body);
                continue;
            }

            let body: serde_json::Value = resp.json().await?;
            if let Some(files) = body["files"].as_array() {
                for entry in files {
                    if let Some(name) = entry.as_str() {
                        if name.ends_with('/') {
                            // Subdirectory — queue for recursive listing
                            let full_path = if dir.is_empty() {
                                name.to_string()
                            } else {
                                format!("{}{}", dir, name)
                            };
                            dirs_to_visit.push(full_path);
                        } else {
                            // File — add with full path
                            let full_path = if dir.is_empty() {
                                name.to_string()
                            } else {
                                format!("{}{}", dir, name)
                            };
                            all_files.push(full_path);
                        }
                    }
                }
            }
        }

        Ok(all_files)
    }

    /// Read a file's content from the vault.
    pub async fn read_file(&self, path: &str) -> Result<String> {
        let encoded = urlencoding::encode(path);
        let url = format!("{}/vault/{}", self.base_url, encoded);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "text/markdown")
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Obsidian API {} reading '{}': {}", status, path, body);
        }

        Ok(resp.text().await?)
    }

    /// Create or overwrite a file in the vault.
    pub async fn write_file(&self, path: &str, content: &str) -> Result<()> {
        let encoded = urlencoding::encode(path);
        let url = format!("{}/vault/{}", self.base_url, encoded);
        let resp = self
            .client
            .put(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "text/markdown")
            .body(content.to_string())
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Obsidian API {} writing '{}': {}", status, path, body);
        }

        Ok(())
    }

    /// Append content to a file in the vault.
    pub async fn append_file(&self, path: &str, content: &str) -> Result<()> {
        let encoded = urlencoding::encode(path);
        let url = format!("{}/vault/{}", self.base_url, encoded);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "text/markdown")
            .body(content.to_string())
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Obsidian API {} appending '{}': {}", status, path, body);
        }

        Ok(())
    }
}
