use crate::utils::{DmCheckResult, check_dm_access, check_group_access, format_pairing_reply};
use anyhow::{Context, Result};
use async_trait::async_trait;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use base64::Engine;
use hmac::{Hmac, Mac};
use oxicrab_core::bus::events::{InboundMessage, OutboundMessage};
use oxicrab_core::channels::base::{BaseChannel, split_message};
use oxicrab_core::config::schema::TwilioConfig;
use sha1::Sha1;
use std::collections::HashMap;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Maximum size for downloaded MMS media (20 MB).
const MAX_MMS_DOWNLOAD: usize = 20 * 1024 * 1024;

type HmacSha1 = Hmac<Sha1>;

pub struct TwilioChannel {
    config: TwilioConfig,
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
    server_handle: Option<tokio::task::JoinHandle<()>>,
    client: reqwest::Client,
}

impl TwilioChannel {
    pub fn new(config: TwilioConfig, inbound_tx: Arc<mpsc::Sender<InboundMessage>>) -> Self {
        Self {
            config,
            inbound_tx,
            shutdown_tx: None,
            server_handle: None,
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

#[derive(Clone)]
struct WebhookState {
    auth_token: Arc<str>,
    webhook_url: String,
    phone_number: String,
    allow_from: oxicrab_core::config::schema::DenyByDefaultList,
    allow_groups: oxicrab_core::config::schema::DenyByDefaultList,
    dm_policy: oxicrab_core::config::schema::DmPolicy,
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
    client: reqwest::Client,
    account_sid: String,
}

fn validate_twilio_signature(
    auth_token: &str,
    signature: &str,
    url: &str,
    params: &HashMap<String, String>,
) -> bool {
    // Build the data string: URL + sorted params (key+value concatenated)
    let mut data = url.to_string();
    let mut sorted_keys: Vec<&String> = params.keys().collect();
    sorted_keys.sort();
    for key in sorted_keys {
        data.push_str(key);
        data.push_str(&params[key]);
    }

    let Ok(mut mac) = HmacSha1::new_from_slice(auth_token.as_bytes()) else {
        return false;
    };
    mac.update(data.as_bytes());
    let result = mac.finalize();
    let expected = base64::engine::general_purpose::STANDARD.encode(result.into_bytes());

    expected.as_bytes().ct_eq(signature.as_bytes()).into()
}

/// Download an MMS media file and save to ~/.oxicrab/media/.
async fn download_mms_media(
    client: &reqwest::Client,
    account_sid: &str,
    auth_token: &str,
    media_url: &str,
    content_type: &str,
    index: u32,
    message_sid: &str,
) -> Option<String> {
    let ext = match content_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/ogg" => "ogg",
        "application/pdf" => "pdf",
        _ => "bin",
    };

    let media_dir = match crate::media_utils::media_dir() {
        Ok(d) => d,
        Err(e) => {
            warn!("failed to create media directory: {}", e);
            return None;
        }
    };

    let safe_sid = crate::media_utils::safe_filename(message_sid);
    let file_path = media_dir.join(format!("twilio_{safe_sid}_{index}.{ext}"));

    // Twilio MMS media URLs require authentication
    let resp = match client
        .get(media_url)
        .basic_auth(account_sid, Some(auth_token))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("failed to download MMS media: {}", e);
            return None;
        }
    };

    if !resp.status().is_success() {
        warn!("MMS media download failed ({})", resp.status());
        return None;
    }

    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            warn!("failed to read MMS media bytes: {}", e);
            return None;
        }
    };

    if bytes.len() > MAX_MMS_DOWNLOAD {
        warn!("MMS media too large ({} bytes), skipping", bytes.len());
        return None;
    }

    if let Err(e) = tokio::fs::write(&file_path, &bytes).await {
        warn!("failed to write MMS media file: {}", e);
        return None;
    }

    Some(file_path.to_string_lossy().to_string())
}

async fn webhook_handler(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    body: String,
) -> axum::response::Response {
    // Skip signature validation if webhook_url is empty (warn at startup, not per-request)
    let validate_signature = !state.webhook_url.is_empty();

    // Extract signature header
    if validate_signature {
        let Some(signature) = headers
            .get("X-Twilio-Signature")
            .and_then(|v| v.to_str().ok())
        else {
            warn!("twilio webhook: missing X-Twilio-Signature header");
            return StatusCode::FORBIDDEN.into_response();
        };
        let signature = signature.to_string();

        // Parse form-encoded body for validation
        let params: HashMap<String, String> = form_urlencoded::parse(body.as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

        // Validates against the configured webhook_url, not the inbound request URL.
        // This is standard for Twilio integrations behind reverse proxies — the URL
        // must match what Twilio was configured to call. If validation fails, check
        // that the webhookUrl config matches the URL configured in Twilio's console.
        if !validate_twilio_signature(&state.auth_token, &signature, &state.webhook_url, &params) {
            warn!("twilio webhook: invalid signature");
            return StatusCode::FORBIDDEN.into_response();
        }
    }

    // Parse form-encoded body
    let params: HashMap<String, String> = form_urlencoded::parse(body.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    // Detect format: SMS webhook has "From"/"To"/"MessageSid",
    // Conversations webhook has "EventType"/"Author"/"ConversationSid"
    let (sender, chat_id, mut body_text, message_sid) = if params.contains_key("MessageSid") {
        // SMS webhook format
        let from = params.get("From").map_or("", String::as_str);
        let body = params.get("Body").map_or("", String::as_str);
        let msg_sid = params
            .get("MessageSid")
            .map_or("", String::as_str)
            .to_string();
        debug!("twilio webhook: SMS from={}, body_len={}", from, body.len());
        // Use sender phone number as chat_id so sessions group by person
        (
            from.to_string(),
            from.to_string(),
            body.to_string(),
            msg_sid,
        )
    } else if params.get("EventType").map_or("", String::as_str) == "onMessageAdded" {
        // Conversations webhook format
        let author = params.get("Author").map_or("", String::as_str);
        let conv_sid = params.get("ConversationSid").map_or("", String::as_str);
        let body = params.get("Body").map_or("", String::as_str);
        let msg_sid = params
            .get("MessageSid")
            .map_or("", String::as_str)
            .to_string();
        debug!(
            "twilio webhook: conversation event author={}, sid={}",
            author, conv_sid
        );
        // Skip our own messages
        if author == "oxicrab" {
            debug!("twilio webhook: ignoring own message");
            return StatusCode::OK.into_response();
        }
        (
            author.to_string(),
            conv_sid.to_string(),
            body.to_string(),
            msg_sid,
        )
    } else {
        let event_type = params.get("EventType").map_or("", String::as_str);
        debug!("twilio webhook: ignoring event type: {}", event_type);
        return StatusCode::OK.into_response();
    };

    // Skip messages from our own number
    if sender == state.phone_number {
        debug!("twilio webhook: ignoring own message");
        return StatusCode::OK.into_response();
    }

    // Conversations messages (ConversationSid) are group messages — skip DM access check.
    // SMS messages (MessageSid) are always 1:1 so DM access applies.
    let is_group = params.contains_key("ConversationSid");

    // Check group access for Conversations
    if is_group && !check_group_access(&chat_id, &state.allow_groups) {
        debug!(
            "twilio webhook: conversation {} not in allowGroups",
            chat_id
        );
        return StatusCode::OK.into_response();
    }

    // Check access based on dmPolicy (skip for group messages, consistent with other channels)
    if !is_group {
        match check_dm_access(&sender, &state.allow_from, "twilio", &state.dm_policy) {
            DmCheckResult::Allowed => {}
            DmCheckResult::PairingRequired { code } => {
                let reply = format_pairing_reply("twilio", &sender, &code);
                // Return TwiML response so Twilio sends the pairing code as an SMS reply
                let escaped = html_escape::encode_text(&reply);
                let twiml = format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Response><Message>{escaped}</Message></Response>"
                );
                return (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "text/xml")],
                    twiml,
                )
                    .into_response();
            }
            DmCheckResult::Denied => {
                debug!("twilio webhook: sender not allowed: {}", sender);
                return StatusCode::OK.into_response();
            }
        }
    }

    // Handle MMS media attachments
    let num_media = params
        .get("NumMedia")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);

    for i in 0..num_media {
        let url_key = format!("MediaUrl{i}");
        let type_key = format!("MediaContentType{i}");
        if let Some(media_url) = params.get(&url_key) {
            let content_type = params
                .get(&type_key)
                .map_or("application/octet-stream", String::as_str);

            if let Some(path) = download_mms_media(
                &state.client,
                &state.account_sid,
                &state.auth_token,
                media_url,
                content_type,
                i,
                &message_sid,
            )
            .await
            {
                let tag = if content_type.starts_with("image/") {
                    format!("\n[image: {path}]")
                } else {
                    format!("\n[document: {path}]")
                };
                body_text.push_str(&tag);
            }
        }
    }

    if body_text.is_empty() {
        debug!("twilio webhook: empty message body");
        return StatusCode::OK.into_response();
    }

    let message = InboundMessage::builder("twilio", sender, chat_id, body_text)
        .is_group(is_group)
        .build();

    if let Err(e) = state.inbound_tx.send(message).await {
        error!("twilio webhook: failed to send inbound message: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::OK.into_response()
}

/// Send a Twilio API request with retry for 429 and 5xx errors.
async fn send_with_retry(
    client: &reqwest::Client,
    url: reqwest::Url,
    account_sid: &str,
    auth_token: &str,
    form_params: &[(&str, &str)],
) -> Result<()> {
    for attempt in 0u32..3 {
        let resp = client
            .post(url.clone())
            .basic_auth(account_sid, Some(auth_token))
            .form(form_params)
            .send()
            .await?;

        let status = resp.status();

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(2);
            warn!("twilio rate limited, retrying after {retry_after}s");
            tokio::time::sleep(std::time::Duration::from_secs(retry_after)).await;
            continue;
        }

        if status.is_server_error() && attempt < 2 {
            warn!("twilio server error ({}), retrying", status);
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        }

        if !status.is_success() {
            // Don't include raw error body — may contain account SIDs and request URLs
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let error_msg = body["message"].as_str().unwrap_or("unknown error");
            return Err(anyhow::anyhow!("twilio API error ({status}): {error_msg}"));
        }

        return Ok(());
    }

    Err(anyhow::anyhow!(
        "twilio API request failed after 3 attempts"
    ))
}

#[async_trait]
impl BaseChannel for TwilioChannel {
    fn name(&self) -> &'static str {
        "twilio"
    }

    async fn start(&mut self) -> Result<()> {
        if self.shutdown_tx.is_some() {
            return Ok(());
        }

        // Warn if webhook_url is empty — signature validation will be skipped
        if self.config.webhook_url.is_empty() {
            warn!("twilio webhookUrl is empty — webhook signature validation is disabled");
        }

        // Validate webhook URL before using it for signature verification
        if !self.config.webhook_url.is_empty() {
            if let Ok(parsed) = url::Url::parse(&self.config.webhook_url) {
                if parsed.scheme() != "https"
                    && !parsed
                        .host_str()
                        .is_some_and(|h| h == "localhost" || h.starts_with("127."))
                {
                    warn!(
                        "twilio webhook_url uses {} (not HTTPS) — signature validation may fail in production",
                        parsed.scheme()
                    );
                }
            } else {
                warn!(
                    "twilio webhook_url is not a valid URL: {}",
                    self.config.webhook_url
                );
            }
        }

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

        let state = WebhookState {
            auth_token: Arc::from(self.config.auth_token.as_str()),
            webhook_url: self.config.webhook_url.clone(),
            phone_number: self.config.phone_number.clone(),
            allow_from: self.config.allow_from.clone(),
            allow_groups: self.config.allow_groups.clone(),
            dm_policy: self.config.dm_policy.clone(),
            inbound_tx: self.inbound_tx.clone(),
            client: self.client.clone(),
            account_sid: self.config.account_sid.clone(),
        };

        let webhook_path = self.config.webhook_path.clone();
        let webhook_port = self.config.webhook_port;
        let webhook_host = self.config.webhook_host.clone();

        let app = Router::new()
            .route(&webhook_path, post(webhook_handler))
            .layer(axum::extract::DefaultBodyLimit::max(1_048_576))
            .with_state(state);

        let bind_addr: std::net::IpAddr = webhook_host
            .parse()
            .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        let addr = std::net::SocketAddr::from((bind_addr, webhook_port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        info!(
            "twilio webhook server listening on {}:{}{}",
            webhook_host, webhook_port, webhook_path
        );

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.changed().await;
                })
                .await
            {
                error!("twilio webhook server error: {}", e);
            }
        });

        self.shutdown_tx = Some(shutdown_tx);
        self.server_handle = Some(handle);
        info!("twilio channel started");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }

        if let Some(handle) = self.server_handle.take() {
            handle.abort();
        }
        info!("twilio channel stopped");
        Ok(())
    }

    async fn is_healthy(&self) -> bool {
        if let Some(ref handle) = self.server_handle {
            !handle.is_finished()
        } else {
            // Twilio hasn't started yet, or was stopped
            self.shutdown_tx.is_none()
        }
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        if !msg.media.is_empty() {
            warn!(
                "twilio: outbound media not yet supported, {} file(s) skipped",
                msg.media.len()
            );
        }

        let chunks = split_message(&msg.content, 1600);

        for chunk in chunks {
            if msg.chat_id.starts_with('+') {
                // SMS API: chat_id is a phone number
                // SECURITY: credentials transmitted exclusively over HTTPS (hardcoded scheme)
                let url = reqwest::Url::parse(&format!(
                    "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
                    urlencoding::encode(&self.config.account_sid)
                ))
                .context("invalid Twilio SMS API URL")?;
                send_with_retry(
                    &self.client,
                    url,
                    &self.config.account_sid,
                    &self.config.auth_token,
                    &[
                        ("Body", chunk.as_str()),
                        ("To", &msg.chat_id),
                        ("From", &self.config.phone_number),
                    ],
                )
                .await?;
            } else {
                // Conversations API: chat_id is a ConversationSid
                // SECURITY: credentials transmitted exclusively over HTTPS (hardcoded scheme)
                let url = reqwest::Url::parse(&format!(
                    "https://conversations.twilio.com/v1/Conversations/{}/Messages",
                    urlencoding::encode(&msg.chat_id)
                ))
                .context("invalid Twilio Conversations API URL")?;
                send_with_retry(
                    &self.client,
                    url,
                    &self.config.account_sid,
                    &self.config.auth_token,
                    &[("Body", chunk.as_str()), ("Author", "oxicrab")],
                )
                .await?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
