use crate::media_utils::get_oxicrab_home;
use crate::utils::{
    DmCheckResult, check_allowed_sender, check_dm_access, check_group_access,
    exponential_backoff_delay, format_pairing_reply,
};
use anyhow::Result;
use async_trait::async_trait;
use oxicrab_core::bus::events::meta;
use oxicrab_core::bus::events::{InboundMessage, OutboundMessage};
use oxicrab_core::channels::base::BaseChannel;
use oxicrab_core::config::schema::WhatsAppConfig;
use serde_json::Value;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use whatsapp_rust::TokioRuntime;
use whatsapp_rust::proto_helpers::MessageExt;

/// WhatsApp-specific metadata key for the original message timestamp (millis).
const META_WHATSAPP_TIMESTAMP: &str = "whatsapp_timestamp";
/// WhatsApp-specific metadata key for the provider message ID.
const META_MESSAGE_ID: &str = "message_id";

/// Normalize a JID by stripping the device suffix and ensuring
/// a domain part is present.
///
/// Examples:
/// - `"15037348571:20@s.whatsapp.net"` -> `"15037348571@s.whatsapp.net"`
/// - `"15037348571@s.whatsapp.net"`    -> `"15037348571@s.whatsapp.net"`
/// - `"15037348571"`                   -> `"15037348571@s.whatsapp.net"`
/// - `"120363123456789@g.us"`          -> `"120363123456789@g.us"`
fn normalize_jid(jid: &str) -> String {
    if jid.contains('@') {
        let (user_part, domain) = jid.split_once('@').unwrap_or((jid, "s.whatsapp.net"));
        // Strip device suffix (e.g. ":20")
        let user = user_part.split(':').next().unwrap_or(user_part);
        format!("{user}@{domain}")
    } else {
        // Bare phone number — strip device suffix and add default domain
        let user = jid.split(':').next().unwrap_or(jid);
        format!("{user}@s.whatsapp.net")
    }
}

pub struct WhatsAppChannel {
    config: WhatsAppConfig,
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
    bot_handle: Option<tokio::task::JoinHandle<()>>,
    running: Arc<tokio::sync::Mutex<bool>>,
    session_path: PathBuf,
    client: Arc<tokio::sync::Mutex<Option<Arc<whatsapp_rust::client::Client>>>>,
    message_queue: Arc<tokio::sync::Mutex<VecDeque<OutboundMessage>>>,
}

impl WhatsAppChannel {
    pub fn new(config: WhatsAppConfig, inbound_tx: Arc<mpsc::Sender<InboundMessage>>) -> Self {
        // Determine session path for WhatsApp session storage
        let session_path = get_oxicrab_home().map_or_else(
            |_| PathBuf::from(".oxicrab/whatsapp"),
            |home| home.join("whatsapp"),
        );

        Self {
            config,
            inbound_tx,
            bot_handle: None,
            running: Arc::new(tokio::sync::Mutex::new(false)),
            session_path,
            client: Arc::new(tokio::sync::Mutex::new(None)),
            message_queue: Arc::new(tokio::sync::Mutex::new(VecDeque::new())),
        }
    }
}

#[async_trait]
impl BaseChannel for WhatsAppChannel {
    fn name(&self) -> &'static str {
        "whatsapp"
    }

    #[allow(clippy::too_many_lines)]
    async fn start(&mut self) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        info!("Initializing WhatsApp channel...");

        // Ensure session directory exists
        if let Some(parent) = self.session_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let session_db = self.session_path.join("whatsapp.db");
        let session_db_str = session_db.to_string_lossy().to_string();

        // Ensure database directory exists and set restrictive permissions
        if let Some(parent) = session_db.parent() {
            std::fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
            }
        }

        debug!("WhatsApp session database path: {}", session_db_str);

        let inbound_tx = self.inbound_tx.clone();
        let running = self.running.clone();
        let config_allow = self.config.allow_from.clone();
        let config_allow_groups = self.config.allow_groups.clone();
        let dm_policy = self.config.dm_policy.clone();
        let client_for_storage = self.client.clone();

        *self.running.lock().await = true;

        let bot_task = tokio::spawn(async move {
            let mut reconnect_attempt = 0u32;
            loop {
                if !*running.lock().await {
                    break;
                }

                // Create SQLite backend for session storage
                debug!("Creating WhatsApp SQLite backend at: {}", session_db_str);
                let backend = match whatsapp_rust::store::SqliteStore::new(&session_db_str).await {
                    Ok(b) => Arc::new(b),
                    Err(e) => {
                        error!("Failed to create WhatsApp backend: {}", e);
                        let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                        reconnect_attempt += 1;
                        warn!("Retrying WhatsApp backend creation in {} seconds...", delay);
                        tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                        continue;
                    }
                };

                // Create transport factory and HTTP client
                let transport_factory =
                    whatsapp_rust::transport::TokioWebSocketTransportFactory::new();
                let http_client = whatsapp_rust::transport::UreqHttpClient::new();

                // Build bot with event handler
                let inbound_tx_clone = inbound_tx.clone();
                let running_clone = running.clone();
                let config_allow_clone = config_allow.clone();
                let config_allow_groups_clone = config_allow_groups.clone();
                let dm_policy_clone = dm_policy.clone();
                let client_storage_clone = client_for_storage.clone();

                let bot_builder = whatsapp_rust::bot::Bot::builder()
                    .with_backend(backend.clone())
                    .with_transport_factory(transport_factory)
                    .with_http_client(http_client)
                    .with_runtime(TokioRuntime)
                    .on_event(move |event, client| {
                        let inbound_tx = inbound_tx_clone.clone();
                        let running = running_clone.clone();
                        let config_allow = config_allow_clone.clone();
                        let config_allow_groups = config_allow_groups_clone.clone();
                        let dm_policy = dm_policy_clone.clone();
                        let client_storage = client_storage_clone.clone();
                        async move {
                            // Store client for sending messages
                            {
                                let mut client_guard = client_storage.lock().await;
                                *client_guard = Some(client.clone());
                            }

                            // Process events
                            debug!("WhatsApp event received: type={:?}", std::mem::discriminant(&event));
                            match &event {
                                whatsapp_rust::types::events::Event::Message(msg, info) => {
                                    // In linked-device mode the bot IS the user's phone,
                                    // so is_from_me is true for ALL messages from this account.
                                    // Use should_skip_own_message() to filter out messages
                                    // sent to other people while letting self-chat through.
                                    if info.source.is_from_me {
                                        let recip_str = info.source.recipient
                                            .as_ref()
                                            .map(std::string::ToString::to_string);
                                        if should_skip_own_message(
                                            recip_str.as_deref(),
                                            &config_allow,
                                        ) {
                                            debug!(
                                                "Ignoring device-synced outgoing message to {:?}",
                                                recip_str
                                            );
                                            return;
                                        }
                                    }

                                    // Handle message event (organized inline to avoid type issues)
                                    let sender_jid = info.source.sender.to_string();
                                    let chat_jid = info.source.chat.to_string();
                                    let is_group = info.source.is_group;

                                    // Normalize sender JID to extract phone number
                                    let sender_normalized = normalize_jid(&sender_jid);
                                    let phone_number = sender_normalized
                                        .split('@')
                                        .next()
                                        .unwrap_or(&sender_normalized)
                                        .to_string();

                                    // For group messages, use the group JID as the chat_id
                                    // (session key), not the sender. For DMs, use the
                                    // sender's normalized JID.
                                    let session_chat_id = if is_group {
                                        normalize_jid(&chat_jid)
                                    } else {
                                        sender_normalized.clone()
                                    };

                                    // Check group access control
                                    if is_group {
                                        let group_id = normalize_jid(&chat_jid);
                                        if !check_group_access(&group_id, &config_allow_groups) {
                                            debug!("whatsapp: ignoring message from non-allowed group {}", group_id);
                                            return;
                                        }
                                    }

                                    // DM access check (skipped for group messages, consistent with other channels)
                                    if !is_group {
                                        // Check access based on dmPolicy — try phone number first, then raw sender
                                        let access = check_dm_access(&phone_number, &config_allow, "whatsapp", &dm_policy);
                                        let access = if matches!(access, DmCheckResult::Allowed | DmCheckResult::PairingRequired { .. }) {
                                            access
                                        } else {
                                            check_dm_access(&sender_jid, &config_allow, "whatsapp", &dm_policy)
                                        };
                                        match access {
                                            DmCheckResult::Allowed => {}
                                            DmCheckResult::PairingRequired { code } => {
                                                warn!("WhatsApp pairing request from {} (phone: {}) — approve with: oxicrab pairing approve whatsapp <code>",
                                                    sender_jid, phone_number);
                                                // Send pairing reply back to the user
                                                let reply = format_pairing_reply("whatsapp", &phone_number, &code);
                                                let jid_str = normalize_jid(&sender_jid);
                                                if let Ok(jid) = whatsapp_rust::Jid::from_str(&jid_str) {
                                                    let text_message = whatsapp_rust::waproto::whatsapp::Message {
                                                        conversation: Some(reply),
                                                        ..Default::default()
                                                    };
                                                    if let Err(e) = Box::pin(client.send_message(jid, text_message)).await {
                                                        error!("failed to send WhatsApp pairing reply: {}", e);
                                                    }
                                                }
                                                return;
                                            }
                                            DmCheckResult::Denied => {
                                                warn!("WhatsApp message from {} (phone: {}) blocked by allowFrom filter (allowed: {:?})",
                                                    sender_jid, phone_number, config_allow);
                                                return;
                                            }
                                        }
                                    }

                                    // Extract message content and media
                                    let base_msg = msg.get_base_message();
                                    let mut content: String;
                                    let mut media_paths: Vec<String> = vec![];

                                    // Classify media type from the message.
                                    // Extract file_length from protobuf metadata for
                                    // pre-download size validation (see MediaKind).
                                    let media = if let Some(ref img) = base_msg.image_message {
                                        Some(MediaKind::Image(&**img, img.mimetype.as_deref(), img.file_length))
                                    } else if let Some(ref audio) = base_msg.audio_message {
                                        Some(MediaKind::Audio(&**audio, audio.mimetype.as_deref(), audio.file_length))
                                    } else if let Some(ref video) = base_msg.video_message {
                                        Some(MediaKind::Video(&**video, video.mimetype.as_deref(), video.file_length))
                                    } else if let Some(ref doc) = base_msg.document_message {
                                        let mime = doc.mimetype.as_deref();
                                        if is_image_mime(mime) {
                                            Some(MediaKind::Image(&**doc, mime, doc.file_length))
                                        } else {
                                            Some(MediaKind::Document(&**doc, mime, doc.file_length))
                                        }
                                    } else {
                                        None
                                    };

                                    if let Some(media_kind) = media {
                                        let (downloadable, mimetype, media_type, tag, file_len) = match media_kind {
                                            MediaKind::Image(d, m, fl) => (d, m, "image", "image", fl),
                                            MediaKind::Audio(d, m, fl) => (d, m, "audio", "audio", fl),
                                            MediaKind::Document(d, m, fl) => (d, m, "document", "document", fl),
                                            MediaKind::Video(d, m, fl) => (d, m, "video", "video", fl),
                                        };
                                        content = msg.get_caption().unwrap_or_default().to_string();

                                        // Pre-download size check using protobuf file_length
                                        // metadata. When file_length is absent, use
                                        // MAX_MEDIA_DOWNLOAD as the assumed upper bound so
                                        // the download still proceeds (the post-download
                                        // check in download_whatsapp_media catches oversized
                                        // payloads).
                                        let max_bytes = file_len.unwrap_or(MAX_MEDIA_DOWNLOAD as u64);
                                        if max_bytes > MAX_MEDIA_DOWNLOAD as u64 {
                                            warn!(
                                                "WhatsApp {} too large (file_length={} bytes, max={}), skipping download",
                                                media_type, max_bytes, MAX_MEDIA_DOWNLOAD
                                            );
                                            if content.is_empty() {
                                                content = format!("[{tag} - too large ({max_bytes} bytes)]");
                                            }
                                        } else {
                                        match download_whatsapp_media(&client, downloadable, mimetype, &info.id, media_type).await {
                                            Ok(path) => {
                                                media_paths.push(path.clone());
                                                if content.is_empty() {
                                                    content = format!("[{tag}: {path}]");
                                                } else {
                                                    content = format!("{content}\n[{tag}: {path}]");
                                                }
                                            }
                                            Err(e) => {
                                                warn!("failed to download WhatsApp {} media: {}", media_type, e);
                                                if content.is_empty() {
                                                    content = format!("[{tag} - download failed]");
                                                }
                                            }
                                        }
                                        }
                                    } else {
                                        content = if let Some(text) = msg.text_content() { text.to_string() } else {
                                            warn!("WhatsApp message has no text content, using fallback");
                                            "[Media Message]".to_string()
                                        };
                                    }

                                    if content.trim().is_empty() {
                                        warn!("WhatsApp message content is empty, skipping");
                                        return;
                                    }

                                    let preview: String = content.chars().take(50).collect();
                                    info!("WhatsApp message from sender={}, chat_id={}, is_group={}, content={}...",
                                        sender_jid, session_chat_id, is_group, preview);

                                    let inbound_msg = InboundMessage::builder("whatsapp", phone_number, session_chat_id, content)
                                        .media(media_paths)
                                        .meta(META_MESSAGE_ID, Value::String(info.id.clone()))
                                        .meta(META_WHATSAPP_TIMESTAMP, Value::Number(serde_json::Number::from(info.timestamp.timestamp_millis())))
                                        .meta(meta::TS, Value::String(info.timestamp.timestamp_millis().to_string()))
                                        .is_group(is_group)
                                        .build();

                                    if let Err(e) = inbound_tx.send(inbound_msg).await {
                                        error!("Failed to send WhatsApp inbound message: {}", e);
                                    }
                                }
                                whatsapp_rust::types::events::Event::PairingQrCode { code, .. } => {
                                    // Display QR code (organized inline)
                                    println!("\n🤖 WhatsApp QR Code:");
                                    match qr2term::print_qr(code) {
                                        Ok(()) => {
                                            println!("\nScan with WhatsApp: Settings > Linked Devices > Link a Device");
                                        }
                                        Err(e) => {
                                            warn!("qr2term failed: {}, falling back to qrcode crate", e);
                                            match qrcode::QrCode::new(code) {
                                                Ok(qr) => {
                                                    let string = qr.render::<char>()
                                                        .quiet_zone(false)
                                                        .module_dimensions(2, 1)
                                                        .build();
                                                    let lines: Vec<&str> = string.lines().collect();
                                                    let max_lines = 25;
                                                    for line in lines.iter().take(max_lines.min(lines.len())) {
                                                        println!("{line}");
                                                    }
                                                    if lines.len() > max_lines {
                                                        println!("\n(QR code truncated to {max_lines} lines)");
                                                    }
                                                }
                                                Err(e2) => {
                                                    warn!("Failed to generate QR code: {}. Raw code: {}", e2, code);
                                                    println!("Raw QR code data: {code}");
                                                }
                                            }
                                            println!("\nScan with WhatsApp: Settings > Linked Devices > Link a Device");
                                        }
                                    }
                                    info!("WhatsApp QR code displayed");
                                }
                                whatsapp_rust::types::events::Event::PairingCode { code, .. } => {
                                    println!("\n🤖 WhatsApp Pairing Code: {code}\nEnter this code on your phone.\n");
                                    info!("WhatsApp pairing code: {}", code);
                                }
                                whatsapp_rust::types::events::Event::PairSuccess(_pair_success) => {
                                    println!("\n✅ WhatsApp connected successfully!\n");
                                    info!("WhatsApp pairing successful");
                                }
                                whatsapp_rust::types::events::Event::PairError(pair_error) => {
                                    error!("WhatsApp pairing failed: {:?}", pair_error);
                                }
                                whatsapp_rust::types::events::Event::Disconnected(_disconnected) => {
                                    warn!("WhatsApp disconnected");
                                    if *running.lock().await {
                                        info!("Will attempt to reconnect...");
                                    }
                                }
                                whatsapp_rust::types::events::Event::Connected(_connected) => {
                                    info!("WhatsApp connected");
                                }
                                _ => {
                                    debug!("WhatsApp event (not handled): {:?}", std::mem::discriminant(&event));
                                }
                            }
                        }
                    });

                // Build and run bot
                let connected_at = tokio::time::Instant::now();
                match bot_builder.build().await {
                    Ok(mut bot) => {
                        info!("WhatsApp bot built successfully, starting...");
                        match bot.run().await {
                            Ok(handle) => {
                                // Wait for bot to finish (or be stopped)
                                if let Err(e) = handle.await {
                                    error!("WhatsApp bot handle error: {}", e);
                                }
                            }
                            Err(e) => {
                                error!("WhatsApp bot run error: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to build WhatsApp bot: {}", e);
                    }
                }

                if *running.lock().await {
                    // Reset backoff if connection lasted more than 2 minutes (not a transient failure)
                    if connected_at.elapsed() > tokio::time::Duration::from_mins(2) {
                        reconnect_attempt = 0;
                    }
                    let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                    reconnect_attempt += 1;
                    warn!("WhatsApp bot stopped, reconnecting in {} seconds...", delay);
                    tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                } else {
                    reconnect_attempt = 0; // Reset on stop
                }
            }
        });

        self.bot_handle = Some(bot_task);
        info!("WhatsApp channel started successfully (bot connecting in background)");

        // Outbound messages are handled by the send() method which uses the stored client
        // No need for a separate outbound receiver task - messages come through ChannelManager.send()

        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        *self.running.lock().await = false;
        if let Some(handle) = self.bot_handle.take() {
            handle.abort();
        }
        Ok(())
    }

    async fn is_healthy(&self) -> bool {
        if let Some(ref handle) = self.bot_handle {
            !handle.is_finished()
        } else {
            false
        }
    }

    async fn send_typing(&self, chat_id: &str) -> Result<()> {
        let client_guard = self.client.lock().await;
        if let Some(client) = client_guard.as_ref() {
            let jid_str = normalize_jid(chat_id);
            if let Ok(jid) = whatsapp_rust::Jid::from_str(&jid_str) {
                let _ = client.chatstate().send_composing(&jid).await;
            }
        }
        Ok(())
    }

    async fn send_and_get_id(&self, msg: &OutboundMessage) -> Result<Option<String>> {
        if msg.channel != "whatsapp" {
            return Ok(None);
        }

        let client_guard = self.client.lock().await;
        if let Some(client) = client_guard.as_ref() {
            Box::pin(send_whatsapp_message(client, msg)).await
        } else {
            warn!("WhatsApp client not available yet, queuing message");
            let mut queue = self.message_queue.lock().await;
            if queue.len() >= 1000 {
                warn!("whatsapp: message queue full (1000), dropping oldest message");
                queue.pop_front();
            }
            queue.push_back(msg.clone());
            Ok(None)
        }
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        if msg.channel != "whatsapp" {
            debug!(
                "WhatsApp send: ignoring message for channel {}",
                msg.channel
            );
            return Ok(());
        }

        if !msg.media.is_empty() {
            warn!(
                "whatsapp: outbound media not yet supported, {} file(s) skipped",
                msg.media.len()
            );
        }

        debug!(
            "WhatsApp send: chat_id={}, content_len={}",
            msg.chat_id,
            msg.content.len()
        );

        let client_arc = {
            let guard = self.client.lock().await;
            guard.clone()
        };
        if let Some(client) = client_arc.as_ref() {
            // Process any queued messages first
            let mut queue = self.message_queue.lock().await;
            let queued: Vec<_> = queue.drain(..).collect();
            let queue_size = queued.len();
            drop(queue);

            if queue_size > 0 {
                info!("Processing {} queued WhatsApp messages", queue_size);
            }

            // Send queued messages
            for queued_msg in queued {
                if let Err(e) = Box::pin(send_whatsapp_message(client, &queued_msg)).await {
                    error!("Failed to send queued WhatsApp message: {}", e);
                }
            }

            // Send current message
            Box::pin(send_whatsapp_message(client, msg))
                .await
                .map(|_| ())
        } else {
            warn!("WhatsApp client not available yet, queuing message");
            let mut queue = self.message_queue.lock().await;
            if queue.len() >= 1000 {
                warn!("whatsapp: message queue full (1000), dropping oldest message");
                queue.pop_front();
            }
            queue.push_back(msg.clone());
            debug!("WhatsApp: queued message (queue size: {})", queue.len());
            Ok(())
        }
    }
}

async fn send_whatsapp_message(
    client: &Arc<whatsapp_rust::client::Client>,
    msg: &OutboundMessage,
) -> Result<Option<String>> {
    // Format chat_id - normalize JID (strip device suffix, ensure domain)
    let chat_id_str = normalize_jid(&msg.chat_id);

    debug!(
        "send_whatsapp_message: chat_id={}, formatted={}",
        msg.chat_id, chat_id_str
    );

    // Parse chat_id as JID (whatsapp-rust re-exports Jid)
    let jid = whatsapp_rust::Jid::from_str(&chat_id_str)
        .map_err(|e| anyhow::anyhow!("Invalid WhatsApp chat_id '{chat_id_str}': {e}"))?;

    // Split long messages using UTF-8 safe splitting
    let chunks = oxicrab_core::channels::base::split_message(&msg.content, 4096);

    let mut last_id = None;
    for (i, chunk) in chunks.iter().enumerate() {
        debug!(
            "send_whatsapp_message: chunk {}/{} ({} bytes)",
            i + 1,
            chunks.len(),
            chunk.len(),
        );
        let text_message = whatsapp_rust::waproto::whatsapp::Message {
            conversation: Some(chunk.clone()),
            ..Default::default()
        };

        match Box::pin(client.send_message(jid.clone(), text_message)).await {
            Ok(msg_id) => {
                info!("WhatsApp message sent to {}: id={}", jid, msg_id);
                last_id = Some(msg_id);
            }
            Err(e) => {
                error!("WhatsApp send to {} failed: {}", jid, e);
                return Err(anyhow::anyhow!("WhatsApp send error: {e}"));
            }
        }
    }
    Ok(last_id)
}

const MAX_MEDIA_DOWNLOAD: usize = 50 * 1024 * 1024; // 50 MB

/// Download a `WhatsApp` media file and save to ~/.oxicrab/media/.
async fn download_whatsapp_media(
    client: &Arc<whatsapp_rust::client::Client>,
    downloadable: &dyn whatsapp_rust::download::Downloadable,
    mimetype: Option<&str>,
    message_id: &str,
    media_type: &str,
) -> Result<String> {
    let media_dir = crate::media_utils::media_dir()?;

    // Infer extension from mimetype
    let ext = match mimetype {
        Some("image/png") => "png",
        Some("image/webp") => "webp",
        Some("image/gif") => "gif",
        Some("image/jpeg" | "image/jpg") => "jpg",
        Some("audio/ogg") => "ogg",
        Some("audio/mpeg") => "mp3",
        Some("audio/mp4") => "m4a",
        Some("audio/wav") => "wav",
        Some("audio/webm" | "video/webm") => "webm",
        Some("audio/flac") => "flac",
        Some("video/mp4") => "mp4",
        Some("video/3gpp") => "3gp",
        Some("application/pdf") => "pdf",
        Some("application/zip") => "zip",
        Some("text/plain") => "txt",
        Some(m) if m.starts_with("image/") => m.strip_prefix("image/").unwrap_or("bin"),
        Some(m) if m.starts_with("audio/") => m.strip_prefix("audio/").unwrap_or("ogg"),
        Some(m) if m.starts_with("video/") => m.strip_prefix("video/").unwrap_or("mp4"),
        _ if media_type == "audio" => "ogg",
        _ if media_type == "video" => "mp4",
        _ if media_type == "document" => "bin",
        _ => "jpg",
    };
    let file_path = media_dir.join(format!(
        "whatsapp_{}.{}",
        crate::media_utils::safe_filename(message_id),
        ext
    ));

    // NOTE: The whatsapp-rust Downloadable trait downloads the full payload into memory.
    // Callers should pre-check file_length from protobuf metadata before calling
    // this function (see MediaKind). This post-download check is a second safety net
    // that also catches the case where file_length metadata was absent.
    let data = client.download(downloadable).await?;
    if data.len() > MAX_MEDIA_DOWNLOAD {
        return Err(anyhow::anyhow!(
            "WhatsApp {} too large ({} bytes, max {})",
            media_type,
            data.len(),
            MAX_MEDIA_DOWNLOAD
        ));
    }
    tokio::fs::write(&file_path, &data).await?;

    let path_str = file_path.to_string_lossy().to_string();
    info!("WhatsApp media saved: {} ({} bytes)", path_str, data.len());
    Ok(path_str)
}

/// Classification of a `WhatsApp` media attachment for download.
/// The `Option<u64>` is the `file_length` from the protobuf metadata,
/// used for pre-download size validation.
enum MediaKind<'a> {
    Image(
        &'a dyn whatsapp_rust::download::Downloadable,
        Option<&'a str>,
        Option<u64>,
    ),
    Audio(
        &'a dyn whatsapp_rust::download::Downloadable,
        Option<&'a str>,
        Option<u64>,
    ),
    Document(
        &'a dyn whatsapp_rust::download::Downloadable,
        Option<&'a str>,
        Option<u64>,
    ),
    Video(
        &'a dyn whatsapp_rust::download::Downloadable,
        Option<&'a str>,
        Option<u64>,
    ),
}

/// Check if a MIME type is an image type.
fn is_image_mime(mime: Option<&str>) -> bool {
    mime.is_some_and(|m| m.starts_with("image/"))
}

/// Determine whether an `is_from_me` message should be skipped.
///
/// In linked-device mode ALL messages from the user's account have
/// `is_from_me == true`, including messages the user sends to the bot
/// (self-chat). We distinguish by looking at the `recipient` field:
///
/// - `recipient` absent -> no routing info, process the message
/// - `recipient` present and phone in `allow_from` -> self-chat, process it
/// - `recipient` present and phone NOT in `allow_from` -> outgoing to
///   someone else (device-synced), skip it
fn should_skip_own_message(
    recipient_jid: Option<&str>,
    allow_from: &oxicrab_core::config::schema::DenyByDefaultList,
) -> bool {
    let Some(recip) = recipient_jid else {
        return false;
    };
    let recip_phone = recip
        .split('@')
        .next()
        .unwrap_or(recip)
        .split(':')
        .next()
        .unwrap_or(recip);
    !check_allowed_sender(recip_phone, allow_from, "whatsapp")
}

#[cfg(test)]
mod tests;
