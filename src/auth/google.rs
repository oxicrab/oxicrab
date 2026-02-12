use anyhow::{Context, Result};
use oauth2::{
    basic::BasicClient, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, RedirectUrl,
    Scope, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

const DEFAULT_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/gmail.modify",
    "https://www.googleapis.com/auth/gmail.send",
    "https://www.googleapis.com/auth/calendar.events",
    "https://www.googleapis.com/auth/calendar.readonly",
];

const DEFAULT_TOKEN_PATH: &str = ".nanobot/google_tokens.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleCredentials {
    pub token: String,
    pub refresh_token: Option<String>,
    pub token_uri: String,
    pub client_id: String,
    pub client_secret: String,
    pub scopes: Vec<String>,
    pub expiry: Option<u64>, // Unix timestamp
}

impl GoogleCredentials {
    pub fn is_valid(&self) -> bool {
        if let Some(expiry) = self.expiry {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() < expiry)
                .unwrap_or(false)
        } else {
            false // No expiry means we don't know — refresh to be safe
        }
    }

    pub async fn refresh(&mut self) -> Result<()> {
        if self.refresh_token.is_none() {
            return Err(anyhow::anyhow!("No refresh token available"));
        }

        // Use direct HTTP call for refresh since oauth2 crate refresh flow is complex
        let refresh_token = self
            .refresh_token
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No refresh token available"))?;
        let client = reqwest::Client::new();
        let mut params = HashMap::new();
        params.insert("refresh_token", refresh_token.clone());
        params.insert("client_id", self.client_id.clone());
        params.insert("client_secret", self.client_secret.clone());
        params.insert("grant_type", "refresh_token".to_string());

        let response = client.post(&self.token_uri).form(&params).send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Token refresh failed: {}", error_text));
        }

        let token_data: serde_json::Value = response.json().await?;

        if let Some(_error) = token_data.get("error") {
            let error_desc = token_data
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(anyhow::anyhow!("Token refresh failed: {}", error_desc));
        }

        self.token = token_data["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing access_token"))?
            .to_string();

        if let Some(refresh_token) = token_data.get("refresh_token").and_then(|v| v.as_str()) {
            self.refresh_token = Some(refresh_token.to_string());
        }

        if let Some(expires_in) = token_data.get("expires_in").and_then(|v| v.as_u64()) {
            if let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) {
                self.expiry = Some(duration.as_secs() + expires_in);
            }
        }

        Ok(())
    }

    pub fn get_access_token(&self) -> &str {
        &self.token
    }
}

pub async fn get_credentials(
    _client_id: &str,
    _client_secret: &str,
    scopes: Option<&[String]>,
    token_path: Option<&Path>,
) -> Result<GoogleCredentials> {
    let scopes = scopes
        .map(|s| s.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        .unwrap_or_else(|| DEFAULT_SCOPES.to_vec());
    let token_path = token_path.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        dirs::home_dir()
            .map(|h| h.join(DEFAULT_TOKEN_PATH))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_TOKEN_PATH))
    });

    let mut creds = load_credentials(&token_path, &scopes)?;

    if let Some(ref mut c) = creds {
        if c.is_valid() {
            return Ok(c.clone());
        }

        if c.refresh_token.is_some() {
            match c.refresh().await {
                Ok(_) => {
                    save_credentials(c, &token_path)?;
                    return Ok(c.clone());
                }
                Err(e) => {
                    warn!("Failed to refresh Google credentials: {}", e);
                }
            }
        }
    }

    Err(anyhow::anyhow!(
        "No valid Google credentials. Run 'nanobot auth google' to authenticate."
    ))
}

pub async fn run_oauth_flow(
    client_id: &str,
    client_secret: &str,
    scopes: Option<&[String]>,
    token_path: Option<&Path>,
    port: u16,
    headless: bool,
) -> Result<GoogleCredentials> {
    let scopes = scopes
        .map(|s| s.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        .unwrap_or_else(|| DEFAULT_SCOPES.to_vec());
    let token_path = token_path.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        dirs::home_dir()
            .map(|h| h.join(DEFAULT_TOKEN_PATH))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_TOKEN_PATH))
    });

    let client = BasicClient::new(ClientId::new(client_id.to_string()))
        .set_client_secret(ClientSecret::new(client_secret.to_string()))
        .set_auth_uri(AuthUrl::new(
            "https://accounts.google.com/o/oauth2/auth".to_string(),
        )?)
        .set_token_uri(TokenUrl::new(
            "https://oauth2.googleapis.com/token".to_string(),
        )?)
        .set_redirect_uri(RedirectUrl::new(format!("http://localhost:{}", port))?);

    let (auth_url, _csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .add_scopes(scopes.iter().map(|s| Scope::new(s.to_string())))
        .url();

    if headless {
        return run_manual_flow(
            client_id,
            client_secret,
            &scopes,
            &token_path,
            auth_url.clone(),
        )
        .await;
    }

    // Try browser flow to get auth code, fall back to manual if it fails
    let code = match get_code_via_browser(auth_url.clone(), port).await {
        Ok(code) => code,
        Err(e) => {
            warn!("Browser flow failed ({}), falling back to manual flow", e);
            return run_manual_flow(client_id, client_secret, &scopes, &token_path, auth_url).await;
        }
    };

    // Exchange code for token using oauth2 crate
    let http_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("Failed to build HTTP client for token exchange")?;
    let token_result = client
        .exchange_code(AuthorizationCode::new(code))
        .request_async(&http_client)
        .await
        .map_err(|e| anyhow::anyhow!("Token exchange failed: {}", e))?;

    let creds = GoogleCredentials {
        token: token_result.access_token().secret().clone(),
        refresh_token: token_result.refresh_token().map(|rt| rt.secret().clone()),
        token_uri: "https://oauth2.googleapis.com/token".to_string(),
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
        scopes: token_result
            .scopes()
            .map(|s| s.iter().map(|scope| scope.to_string()).collect::<Vec<_>>())
            .unwrap_or_default(),
        expiry: token_result
            .expires_in()
            .map(|d: std::time::Duration| d.as_secs())
            .and_then(|secs| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .ok()
                    .map(|now| now.as_secs() + secs)
            }),
    };

    save_credentials(&creds, &token_path)?;
    info!("Google credentials saved to {}", token_path.display());
    Ok(creds)
}

async fn get_code_via_browser(auth_url: url::Url, port: u16) -> Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // Open browser
    if let Err(e) = open::that(auth_url.as_str()) {
        return Err(anyhow::anyhow!("Failed to open browser: {}", e));
    }

    info!("Waiting for OAuth redirect on port {}...", port);

    // Start local server
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    let (mut stream, _) = listener.accept().await?;

    let mut buffer = [0; 4096];
    let n = stream.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer[..n]);

    // Extract code from request
    let code = extract_code_from_request(&request)?;

    // Send response
    let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
    stream.write_all(response.as_bytes()).await?;

    Ok(code)
}

async fn run_manual_flow(
    client_id: &str,
    client_secret: &str,
    scopes: &[&str],
    token_path: &Path,
    auth_url: url::Url,
) -> Result<GoogleCredentials> {
    println!("\n┌─────────────────────────────────────────────────────┐");
    println!("│  Open this URL in any browser and authorize access: │");
    println!("└─────────────────────────────────────────────────────┘\n");
    println!("{}", auth_url);
    println!(
        "\nAfter authorizing, you will be redirected to a localhost URL.\n\
         It may show an error page — that's OK.\n\
         Copy the FULL URL from your browser's address bar and paste it below.\n\
         (It will look like: http://localhost/?code=4/0A...&scope=...)\n"
    );

    use std::io::{self, Write};
    print!("Paste the redirect URL (or just the code): ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let response_input = input.trim();

    // Extract code
    let code = if response_input.starts_with("http") {
        let url = url::Url::parse(response_input)?;
        url.query_pairs()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.to_string())
            .ok_or_else(|| anyhow::anyhow!("Could not find 'code' parameter in URL"))?
    } else {
        response_input.to_string()
    };

    // Exchange code for token
    let client = reqwest::Client::new();
    let mut params = HashMap::new();
    params.insert("code", code);
    params.insert("client_id", client_id.to_string());
    params.insert("client_secret", client_secret.to_string());
    params.insert("redirect_uri", "http://localhost".to_string());
    params.insert("grant_type", "authorization_code".to_string());

    let response = client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(anyhow::anyhow!("Token exchange failed: {}", error_text));
    }

    let token_data: serde_json::Value = response.json().await?;

    if let Some(_error) = token_data.get("error") {
        let error_desc = token_data
            .get("error_description")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(anyhow::anyhow!("Token exchange failed: {}", error_desc));
    }

    let expiry = token_data
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .and_then(|secs| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|now| now.as_secs() + secs)
        });

    let creds = GoogleCredentials {
        token: token_data["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing access_token"))?
            .to_string(),
        refresh_token: token_data
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        token_uri: "https://oauth2.googleapis.com/token".to_string(),
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        expiry,
    };

    save_credentials(&creds, token_path)?;
    info!("Google credentials saved to {}", token_path.display());

    Ok(creds)
}

fn extract_code_from_request(request: &str) -> Result<String> {
    // Parse HTTP request and extract code from query string
    let lines: Vec<&str> = request.lines().collect();
    if let Some(first_line) = lines.first() {
        if let Some(path_part) = first_line.split_whitespace().nth(1) {
            if let Some(query_part) = path_part.split('?').nth(1) {
                for pair in query_part.split('&') {
                    if let Some((key, value)) = pair.split_once('=') {
                        if key == "code" {
                            return Ok(urlencoding::decode(value)?.to_string());
                        }
                    }
                }
            }
        }
    }
    Err(anyhow::anyhow!(
        "Could not find 'code' parameter in request"
    ))
}

pub fn has_valid_credentials(
    _client_id: &str,
    _client_secret: &str,
    scopes: Option<&[String]>,
    token_path: Option<&Path>,
) -> bool {
    let scopes_vec: Vec<&str> = scopes
        .map(|s| s.iter().map(|s| s.as_str()).collect())
        .unwrap_or_else(|| DEFAULT_SCOPES.to_vec());
    let token_path = token_path.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        dirs::home_dir()
            .map(|h| h.join(DEFAULT_TOKEN_PATH))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_TOKEN_PATH))
    });

    // Check credentials synchronously without creating a nested runtime
    match load_credentials(&token_path, &scopes_vec) {
        Ok(Some(creds)) => creds.is_valid() || creds.refresh_token.is_some(),
        _ => false,
    }
}

fn load_credentials(path: &Path, scopes: &[&str]) -> Result<Option<GoogleCredentials>> {
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(path).context(format!(
        "Failed to read credentials from {}",
        path.display()
    ))?;
    let creds: GoogleCredentials = serde_json::from_str(&content).context(format!(
        "Failed to parse credentials from {}",
        path.display()
    ))?;

    // Verify scopes match
    let required_scopes: std::collections::HashSet<String> =
        scopes.iter().map(|s| s.to_string()).collect();
    let cred_scopes: std::collections::HashSet<String> = creds.scopes.iter().cloned().collect();
    if !required_scopes.is_subset(&cred_scopes) {
        warn!("Credential scopes don't match required scopes");
        return Ok(None);
    }

    Ok(Some(creds))
}

fn save_credentials(creds: &GoogleCredentials, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = serde_json::to_string_pretty(creds)?;
    std::fs::write(path, content)?;

    // Restrict permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(path, perms)?;
    }

    Ok(())
}
