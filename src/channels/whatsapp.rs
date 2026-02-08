use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::BaseChannel;
use crate::channels::utils::{check_allowed_sender, exponential_backoff_delay};
use crate::config::WhatsAppConfig;
use crate::utils::get_nanobot_home;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use whatsapp_rust::proto_helpers::MessageExt;

pub struct WhatsAppChannel {
    config: WhatsAppConfig,
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
    bot_handle: Option<tokio::task::JoinHandle<()>>,
    running: Arc<tokio::sync::Mutex<bool>>,
    session_path: PathBuf,
    client: Arc<tokio::sync::Mutex<Option<Arc<whatsapp_rust::client::Client>>>>,
    message_queue: Arc<tokio::sync::Mutex<Vec<OutboundMessage>>>,
}

impl WhatsAppChannel {
    pub fn new(
        config: WhatsAppConfig,
        inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
        _outbound_rx: mpsc::Receiver<OutboundMessage>,
    ) -> Self {
        // Determine session path for WhatsApp session storage
        let session_path = get_nanobot_home()
            .map(|home| home.join("whatsapp"))
            .unwrap_or_else(|_| PathBuf::from(".nanobot/whatsapp"));

        Self {
            config,
            inbound_tx,
            bot_handle: None,
            running: Arc::new(tokio::sync::Mutex::new(false)),
            session_path,
            client: Arc::new(tokio::sync::Mutex::new(None)),
            message_queue: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl BaseChannel for WhatsAppChannel {
    fn name(&self) -> &str {
        "whatsapp"
    }

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

        // Ensure database file can be created
        if let Some(parent) = session_db.parent() {
            std::fs::create_dir_all(parent)?;
        }

        debug!("WhatsApp session database path: {}", session_db_str);

        let inbound_tx = self.inbound_tx.clone();
        let running = self.running.clone();
        let config_allow = self.config.allow_from.clone();
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
                    whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory::new();
                let http_client = whatsapp_rust_ureq_http_client::UreqHttpClient::new();

                // Build bot with event handler
                let inbound_tx_clone = inbound_tx.clone();
                let running_clone = running.clone();
                let config_allow_clone = config_allow.clone();
                let client_storage_clone = client_for_storage.clone();

                let bot_builder = whatsapp_rust::bot::Bot::builder()
                    .with_backend(backend.clone())
                    .with_transport_factory(transport_factory)
                    .with_http_client(http_client)
                    .on_event(move |event, client| {
                        let inbound_tx = inbound_tx_clone.clone();
                        let running = running_clone.clone();
                        let config_allow = config_allow_clone.clone();
                        let client_storage = client_storage_clone.clone();
                        async move {
                            // Store client for sending messages
                            {
                                let mut client_guard = client_storage.lock().await;
                                *client_guard = Some(client.clone());
                            }

                            // Process events
                            info!("WhatsApp event received: type={:?}", std::mem::discriminant(&event));
                            match &event {
                                whatsapp_rust::types::events::Event::Message(msg, info) => {
                                    // Handle message event (organized inline to avoid type issues)
                                    let sender = info.source.sender.to_string();
                                    
                                    // Extract phone number without device ID
                                    let chat_id = if sender.contains('@') {
                                        sender.split('@').next().unwrap_or(&sender).to_string()
                                    } else {
                                        sender.clone()
                                    };

                                    // Remove device ID suffix
                                    let phone_number = if chat_id.contains(':') {
                                        chat_id.split(':').next().unwrap_or(&chat_id).to_string()
                                    } else {
                                        chat_id.clone()
                                    };

                                    // Check allow_from filter using utility function
                                    if !check_allowed_sender(&phone_number, &config_allow)
                                        && !check_allowed_sender(&chat_id, &config_allow)
                                    {
                                        warn!("WhatsApp message from {} (phone: {}) blocked by allowFrom filter (allowed: {:?})", 
                                            chat_id, phone_number, config_allow);
                                        return;
                                    }

                                    // Extract message content
                                    let content: String = match msg.text_content() {
                                        Some(text) => text.to_string(),
                                        None => {
                                            warn!("WhatsApp message has no text content, using fallback");
                                            "[Media Message]".to_string()
                                        }
                                    };

                                    if content.trim().is_empty() {
                                        warn!("WhatsApp message content is empty, skipping");
                                        return;
                                    }

                                    info!("WhatsApp message from sender={}, chat_id={}, content={}...", 
                                        sender, chat_id, &content[..content.len().min(50)]);

                                    let inbound_msg = InboundMessage {
                                        channel: "whatsapp".to_string(),
                                        sender_id: chat_id.clone(),
                                        chat_id: sender.clone(),
                                        content,
                                        timestamp: Utc::now(),
                                        media: vec![],
                                        metadata: {
                                            let mut meta = HashMap::new();
                                            meta.insert("message_id".to_string(), 
                                                Value::String(info.id.to_string()));
                                            meta.insert("whatsapp_timestamp".to_string(), 
                                                Value::Number(serde_json::Number::from(info.timestamp.timestamp_millis())));
                                            meta.insert("is_group".to_string(), 
                                                Value::Bool(info.source.is_group));
                                            meta
                                        },
                                    };

                                    info!("WhatsApp: sending inbound message to bus: sender={}, chat_id={}, content_len={}", 
                                        chat_id, sender, inbound_msg.content.len());
                                    if let Err(e) = inbound_tx.send(inbound_msg).await {
                                        error!("Failed to send WhatsApp inbound message: {}", e);
                                    } else {
                                        info!("WhatsApp: successfully sent inbound message to bus");
                                    }
                                }
                                whatsapp_rust::types::events::Event::PairingQrCode { code, .. } => {
                                    // Display QR code (organized inline)
                                    println!("\n🤖 WhatsApp QR Code:");
                                    match qr2term::print_qr(code) {
                                        Ok(_) => {
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
                                                        println!("{}", line);
                                                    }
                                                    if lines.len() > max_lines {
                                                        println!("\n(QR code truncated to {} lines)", max_lines);
                                                    }
                                                }
                                                Err(e2) => {
                                                    warn!("Failed to generate QR code: {}. Raw code: {}", e2, code);
                                                    println!("Raw QR code data: {}", code);
                                                }
                                            }
                                            println!("\nScan with WhatsApp: Settings > Linked Devices > Link a Device");
                                        }
                                    }
                                    info!("WhatsApp QR code displayed");
                                }
                                whatsapp_rust::types::events::Event::PairingCode { code, .. } => {
                                    println!("\n🤖 WhatsApp Pairing Code: {}\nEnter this code on your phone.\n", code);
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

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        if msg.channel != "whatsapp" {
            debug!(
                "WhatsApp send: ignoring message for channel {}",
                msg.channel
            );
            return Ok(());
        }

        info!(
            "WhatsApp send: received message to send: chat_id={}, content_len={}",
            msg.chat_id,
            msg.content.len()
        );

        let client_guard = self.client.lock().await;
        if let Some(client) = client_guard.as_ref() {
            info!("WhatsApp client is available, sending message");
            // Process any queued messages first
            let mut queue = self.message_queue.lock().await;
            let queued = queue.drain(..).collect::<Vec<_>>();
            let queue_size = queued.len();
            drop(queue);

            if queue_size > 0 {
                info!("Processing {} queued WhatsApp messages", queue_size);
            }

            // Send queued messages
            for queued_msg in queued {
                if let Err(e) = send_whatsapp_message(client, &queued_msg).await {
                    error!("Failed to send queued WhatsApp message: {}", e);
                } else {
                    info!("Successfully sent queued WhatsApp message");
                }
            }

            // Send current message
            info!("Sending current WhatsApp message via send_whatsapp_message");
            match send_whatsapp_message(client, msg).await {
                Ok(_) => {
                    info!("WhatsApp send() completed successfully");
                    Ok(())
                }
                Err(e) => {
                    error!("WhatsApp send() failed: {}", e);
                    Err(e)
                }
            }
        } else {
            // Client not ready yet - queue the message
            warn!("WhatsApp client not available yet, queuing message (queue size will be logged on next send)");
            let mut queue = self.message_queue.lock().await;
            queue.push(msg.clone());
            info!("WhatsApp: queued message (queue size: {})", queue.len());
            Ok(())
        }
    }
}

async fn send_whatsapp_message(
    client: &Arc<whatsapp_rust::client::Client>,
    msg: &OutboundMessage,
) -> Result<()> {
    // Format chat_id - ensure it has @s.whatsapp.net suffix if it's a phone number
    // Note: WhatsApp JIDs should NOT include device ID when sending (e.g., use "15037348571@s.whatsapp.net" not "15037348571:20@s.whatsapp.net")
    let chat_id_str = if msg.chat_id.contains('@') {
        // Remove device ID if present (e.g., "15037348571:20@s.whatsapp.net" -> "15037348571@s.whatsapp.net")
        let parts: Vec<&str> = msg.chat_id.split('@').collect();
        if parts.len() == 2 {
            let user_part = parts[0];
            let domain_part = parts[1];
            // Remove device ID (everything after ':')
            let user_without_device = if user_part.contains(':') {
                user_part.split(':').next().unwrap_or(user_part)
            } else {
                user_part
            };
            format!("{}@{}", user_without_device, domain_part)
        } else {
            msg.chat_id.clone()
        }
    } else {
        format!("{}@s.whatsapp.net", msg.chat_id)
    };

    info!(
        "send_whatsapp_message: original chat_id={}, formatted chat_id_str={} (device ID removed)",
        msg.chat_id, chat_id_str
    );

    // Parse chat_id as JID (whatsapp-rust re-exports Jid)
    use std::str::FromStr;
    let jid = match whatsapp_rust::Jid::from_str(&chat_id_str) {
        Ok(j) => {
            info!("send_whatsapp_message: parsed JID successfully: {}", j);
            j
        }
        Err(e) => {
            error!("Invalid WhatsApp chat_id format {}: {}", chat_id_str, e);
            return Err(anyhow::anyhow!("Invalid chat_id: {}", e));
        }
    };

    // Split long messages using UTF-8 safe splitting
    let chunks = crate::channels::base::split_message(&msg.content, 4096);

    for (i, chunk) in chunks.iter().enumerate() {
        info!(
            "send_whatsapp_message: sending chunk {}/{} ({} bytes) to JID {}",
            i + 1,
            chunks.len(),
            chunk.len(),
            jid
        );
        // Create a text message using waproto (re-exported by whatsapp-rust)
        let mut text_message = whatsapp_rust::waproto::whatsapp::Message::default();
        text_message.conversation = Some(chunk.clone());

        let preview_end = chunk.char_indices().nth(50).map(|(i, _)| i).unwrap_or(chunk.len());
        info!(
            "send_whatsapp_message: calling client.send_message with JID={}, content_preview={}...",
            jid,
            &chunk[..preview_end]
        );
        match client.send_message(jid.clone(), text_message).await {
            Ok(msg_id) => {
                info!(
                    "Successfully sent WhatsApp message to {} (JID: {}): message_id={}",
                    chat_id_str, jid, msg_id
                );
            }
            Err(e) => {
                error!(
                    "Failed to send WhatsApp message to {} (JID: {}): {}",
                    chat_id_str, jid, e
                );
                return Err(anyhow::anyhow!("WhatsApp send error: {}", e));
            }
        }
    }
    info!(
        "send_whatsapp_message: completed sending all {} chunks",
        chunks.len()
    );
    Ok(())
}
