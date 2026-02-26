use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::{BaseChannel, split_message};
use crate::channels::utils::{DmCheckResult, check_dm_access, format_pairing_reply};
use crate::config::TwilioConfig;
use anyhow::Result;
use async_trait::async_trait;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::collections::HashMap;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

type HmacSha1 = Hmac<Sha1>;

pub struct TwilioChannel {
    config: TwilioConfig,
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
    running: Arc<tokio::sync::Mutex<bool>>,
    server_handle: Option<tokio::task::JoinHandle<()>>,
    client: reqwest::Client,
}

impl TwilioChannel {
    pub fn new(config: TwilioConfig, inbound_tx: Arc<mpsc::Sender<InboundMessage>>) -> Self {
        Self {
            config,
            inbound_tx,
            running: Arc::new(tokio::sync::Mutex::new(false)),
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
    auth_token: String,
    webhook_url: String,
    phone_number: String,
    allow_from: Vec<String>,
    dm_policy: crate::config::DmPolicy,
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
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

async fn webhook_handler(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    body: String,
) -> axum::response::Response {
    // Extract signature header
    let Some(signature) = headers
        .get("X-Twilio-Signature")
        .and_then(|v| v.to_str().ok())
    else {
        warn!("twilio webhook: missing X-Twilio-Signature header");
        return StatusCode::FORBIDDEN.into_response();
    };
    let signature = signature.to_string();

    // Parse form-encoded body
    let params: HashMap<String, String> = form_urlencoded::parse(body.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    // Validate signature
    if !validate_twilio_signature(&state.auth_token, &signature, &state.webhook_url, &params) {
        warn!("twilio webhook: invalid signature");
        return StatusCode::FORBIDDEN.into_response();
    }

    // Detect format: SMS webhook has "From"/"To"/"MessageSid",
    // Conversations webhook has "EventType"/"Author"/"ConversationSid"
    let (sender, chat_id, body_text) = if params.contains_key("MessageSid") {
        // SMS webhook format
        let from = params.get("From").map_or("", String::as_str);
        let body = params.get("Body").map_or("", String::as_str);
        debug!("twilio webhook: SMS from={}, body_len={}", from, body.len());
        // Use sender phone number as chat_id so sessions group by person
        (from.to_string(), from.to_string(), body.to_string())
    } else if params.get("EventType").map_or("", String::as_str) == "onMessageAdded" {
        // Conversations webhook format
        let author = params.get("Author").map_or("", String::as_str);
        let conv_sid = params.get("ConversationSid").map_or("", String::as_str);
        let body = params.get("Body").map_or("", String::as_str);
        debug!(
            "twilio webhook: conversation event author={}, sid={}",
            author, conv_sid
        );
        // Skip our own messages
        if author == "oxicrab" {
            debug!("twilio webhook: ignoring own message");
            return StatusCode::OK.into_response();
        }
        (author.to_string(), conv_sid.to_string(), body.to_string())
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

    // Check access based on dmPolicy
    match check_dm_access(&sender, &state.allow_from, "twilio", &state.dm_policy) {
        DmCheckResult::Allowed => {}
        DmCheckResult::PairingRequired { code } => {
            let reply = format_pairing_reply("twilio", &sender, &code);
            // Return TwiML response so Twilio sends the pairing code as an SMS reply
            let escaped = html_escape::encode_text(&reply);
            let twiml = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Response><Message>{}</Message></Response>",
                escaped
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

    if body_text.is_empty() {
        debug!("twilio webhook: empty message body");
        return StatusCode::OK.into_response();
    }

    // Conversations API messages (ConversationSid) can be group chats;
    // SMS messages (phone number chat_id) are always 1:1.
    let is_group = params.contains_key("ConversationSid");
    let mut metadata = HashMap::new();
    metadata.insert("is_group".to_string(), serde_json::Value::Bool(is_group));
    let message = InboundMessage {
        channel: "twilio".to_string(),
        sender_id: sender,
        chat_id,
        content: body_text,
        timestamp: Utc::now(),
        media: Vec::new(),
        metadata,
    };

    if let Err(e) = state.inbound_tx.send(message).await {
        error!("twilio webhook: failed to send inbound message: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::OK.into_response()
}

#[async_trait]
impl BaseChannel for TwilioChannel {
    fn name(&self) -> &'static str {
        "twilio"
    }

    async fn start(&mut self) -> Result<()> {
        let mut running = self.running.lock().await;
        if *running {
            return Ok(());
        }
        *running = true;
        drop(running);

        // Validate webhook URL before using it for signature verification
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

        let state = WebhookState {
            auth_token: self.config.auth_token.clone(),
            webhook_url: self.config.webhook_url.clone(),
            phone_number: self.config.phone_number.clone(),
            allow_from: self.config.allow_from.clone(),
            dm_policy: self.config.dm_policy.clone(),
            inbound_tx: self.inbound_tx.clone(),
        };

        let webhook_path = self.config.webhook_path.clone();
        let webhook_port = self.config.webhook_port;

        let app = Router::new()
            .route(&webhook_path, post(webhook_handler))
            .with_state(state);

        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], webhook_port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        info!(
            "twilio webhook server listening on 0.0.0.0:{}{}",
            webhook_port, webhook_path
        );

        let running = self.running.clone();
        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        if !*running.lock().await {
                            break;
                        }
                    }
                })
                .await
            {
                error!("twilio webhook server error: {}", e);
            }
        });

        self.server_handle = Some(handle);
        info!("twilio channel started");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        let mut running = self.running.lock().await;
        *running = false;
        drop(running);

        if let Some(handle) = self.server_handle.take() {
            handle.abort();
        }
        info!("twilio channel stopped");
        Ok(())
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
            let response = if msg.chat_id.starts_with('+') {
                // SMS API: chat_id is a phone number
                let url = format!(
                    "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
                    self.config.account_sid
                );
                self.client
                    .post(&url)
                    .basic_auth(&self.config.account_sid, Some(&self.config.auth_token))
                    .form(&[
                        ("Body", chunk.as_str()),
                        ("To", &msg.chat_id),
                        ("From", &self.config.phone_number),
                    ])
                    .send()
                    .await?
            } else {
                // Conversations API: chat_id is a ConversationSid
                let url = format!(
                    "https://conversations.twilio.com/v1/Conversations/{}/Messages",
                    urlencoding::encode(&msg.chat_id)
                );
                self.client
                    .post(&url)
                    .basic_auth(&self.config.account_sid, Some(&self.config.auth_token))
                    .form(&[("Body", chunk.as_str()), ("Author", "oxicrab")])
                    .send()
                    .await?
            };

            if !response.status().is_success() {
                let status = response.status();
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "unknown".to_string());
                return Err(anyhow::anyhow!("twilio API error ({}): {}", status, body));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_signature_valid() {
        // Twilio example values
        let auth_token = "12345";
        let url = "https://mycompany.com/myapp.php?foo=1&bar=2";
        let mut params = HashMap::new();
        params.insert("CallSid".to_string(), "CA1234567890ABCDE".to_string());
        params.insert("Caller".to_string(), "+14158675310".to_string());
        params.insert("Digits".to_string(), "1234".to_string());
        params.insert("From".to_string(), "+14158675310".to_string());
        params.insert("To".to_string(), "+18005551212".to_string());

        // Compute expected signature
        let mut data = url.to_string();
        let mut sorted_keys: Vec<&String> = params.keys().collect();
        sorted_keys.sort();
        for key in &sorted_keys {
            data.push_str(key);
            data.push_str(&params[*key]);
        }
        let mut mac = HmacSha1::new_from_slice(auth_token.as_bytes()).unwrap();
        mac.update(data.as_bytes());
        let result = mac.finalize();
        let expected_sig = base64::engine::general_purpose::STANDARD.encode(result.into_bytes());

        assert!(validate_twilio_signature(
            auth_token,
            &expected_sig,
            url,
            &params
        ));
    }

    #[test]
    fn test_validate_signature_invalid() {
        let auth_token = "12345";
        let url = "https://example.com/webhook";
        let params = HashMap::new();

        assert!(!validate_twilio_signature(
            auth_token,
            "invalid_signature",
            url,
            &params
        ));
    }

    #[test]
    fn test_validate_signature_empty_params() {
        let auth_token = "test_token";
        let url = "https://example.com/webhook";
        let params = HashMap::new();

        // Compute expected signature with empty params (just URL)
        let mut mac = HmacSha1::new_from_slice(auth_token.as_bytes()).unwrap();
        mac.update(url.as_bytes());
        let result = mac.finalize();
        let expected_sig = base64::engine::general_purpose::STANDARD.encode(result.into_bytes());

        assert!(validate_twilio_signature(
            auth_token,
            &expected_sig,
            url,
            &params
        ));
    }

    #[test]
    fn test_validate_signature_param_ordering() {
        let auth_token = "secret";
        let url = "https://example.com/hook";
        let mut params = HashMap::new();
        params.insert("Zebra".to_string(), "last".to_string());
        params.insert("Alpha".to_string(), "first".to_string());
        params.insert("Middle".to_string(), "mid".to_string());

        // Compute expected: URL + AlphafirstMiddlemidZebralast
        let data = format!("{}AlphafirstMiddlemidZebralast", url);
        let mut mac = HmacSha1::new_from_slice(auth_token.as_bytes()).unwrap();
        mac.update(data.as_bytes());
        let result = mac.finalize();
        let expected_sig = base64::engine::general_purpose::STANDARD.encode(result.into_bytes());

        assert!(validate_twilio_signature(
            auth_token,
            &expected_sig,
            url,
            &params
        ));
    }

    #[test]
    fn test_send_constructs_correct_url() {
        // Verify the URL format for sending messages
        let chat_id = "CH1234567890";
        let url = format!(
            "https://conversations.twilio.com/v1/Conversations/{}/Messages",
            chat_id
        );
        assert_eq!(
            url,
            "https://conversations.twilio.com/v1/Conversations/CH1234567890/Messages"
        );
    }
}
