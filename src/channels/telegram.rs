use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::{split_message, BaseChannel};
use crate::channels::utils::{check_allowed_sender, exponential_backoff_delay};
use crate::config::TelegramConfig;
use crate::utils::regex::RegexPatterns;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{Message as TgMessage, MessageKind, Update};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub struct TelegramChannel {
    config: TelegramConfig,
    inbound_tx: mpsc::Sender<InboundMessage>,
    bot: Bot,
    running: Arc<tokio::sync::Mutex<bool>>,
    dispatcher_handle: Option<tokio::task::JoinHandle<()>>,
}

impl TelegramChannel {
    pub fn new(config: TelegramConfig, inbound_tx: mpsc::Sender<InboundMessage>) -> Self {
        let bot = Bot::new(&config.token);
        Self {
            config,
            inbound_tx,
            bot,
            running: Arc::new(tokio::sync::Mutex::new(false)),
            dispatcher_handle: None,
        }
    }
}

#[async_trait]
impl BaseChannel for TelegramChannel {
    fn name(&self) -> &'static str {
        "telegram"
    }

    async fn start(&mut self) -> Result<()> {
        info!("Initializing Telegram bot...");
        *self.running.lock().await = true;

        let bot = self.bot.clone();
        let inbound_tx = self.inbound_tx.clone();
        let allow_list = self.config.allow_from.clone();
        let running = self.running.clone();

        // Spawn dispatcher in background task with retry loop
        let handle = tokio::spawn(async move {
            let mut reconnect_attempt = 0u32;
            loop {
                // Check if we should still be running
                if !*running.lock().await {
                    info!("Telegram channel stopped, exiting retry loop");
                    break;
                }

                let bot_clone = bot.clone();
                let inbound_tx_clone = inbound_tx.clone();
                let allow_list_clone = allow_list.clone();

                let handler = Update::filter_message().endpoint(move |bot: Bot, msg: TgMessage| {
                    let inbound_tx = inbound_tx_clone.clone();
                    let allow_list = allow_list_clone.clone();
                    async move {
                        if let MessageKind::Common(_msg_common) = &msg.kind {
                            let sender_id = msg
                                .from
                                .as_ref()
                                .map(|u| u.id.to_string())
                                .unwrap_or_default();

                            // Check allowlist using utility function
                            if !check_allowed_sender(&sender_id, &allow_list) {
                                return Ok(());
                            }

                            // Handle photos
                            if let Some(photos) = msg.photo() {
                                if let Some(photo) = photos.last() {
                                    let text = msg.caption().unwrap_or("").to_string();
                                    let mut media_paths = Vec::new();
                                    let mut content = text.clone();

                                    // Download the photo
                                    match bot.get_file(photo.file.id.clone()).await {
                                        Ok(file) => {
                                            let media_dir = dirs::home_dir()
                                                .unwrap_or_else(|| std::path::PathBuf::from("."))
                                                .join(".nanobot")
                                                .join("media");
                                            if let Err(e) = std::fs::create_dir_all(&media_dir) {
                                                warn!("Failed to create media directory: {}", e);
                                            }
                                            let file_path = media_dir
                                                .join(format!("telegram_{}.jpg", photo.file.unique_id));

                                            let mut dst =
                                                tokio::fs::File::create(&file_path).await.map_err(|e| {
                                                    warn!(
                                                        "Failed to create file for Telegram photo: {}",
                                                        e
                                                    );
                                                    e
                                                });
                                            if let Ok(ref mut dst_file) = dst {
                                                if let Err(e) =
                                                    bot.download_file(&file.path, dst_file).await
                                                {
                                                    warn!(
                                                        "Failed to download Telegram photo: {}",
                                                        e
                                                    );
                                                } else {
                                                    let path_str = file_path.to_string_lossy().to_string();
                                                    media_paths.push(path_str.clone());
                                                    content = format!("{}\n[image: {}]", content, path_str);
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            warn!("Failed to get Telegram file info: {}", e);
                                        }
                                    }

                                    if !content.trim().is_empty() || !media_paths.is_empty() {
                                        let inbound_msg = InboundMessage {
                                            channel: "telegram".to_string(),
                                            sender_id,
                                            chat_id: msg.chat.id.to_string(),
                                            content,
                                            timestamp: Utc::now(),
                                            media: media_paths,
                                            metadata: HashMap::new(),
                                        };
                                        if let Err(e) = inbound_tx.send(inbound_msg).await {
                                            error!(
                                                "Failed to send Telegram inbound message: {}",
                                                e
                                            );
                                        }
                                    }
                                    return Ok(());
                                }
                            }

                            // Handle voice messages
                            if let Some(voice) = msg.voice() {
                                let text = msg.caption().unwrap_or("").to_string();
                                let mut media_paths = Vec::new();
                                let mut content = text;

                                match bot.get_file(voice.file.id.clone()).await {
                                    Ok(file) => {
                                        let media_dir = dirs::home_dir()
                                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                                            .join(".nanobot")
                                            .join("media");
                                        if let Err(e) = std::fs::create_dir_all(&media_dir) {
                                            warn!("Failed to create media directory: {}", e);
                                        }
                                        let file_path = media_dir
                                            .join(format!("telegram_{}.ogg", voice.file.unique_id));

                                        let mut dst =
                                            tokio::fs::File::create(&file_path).await.map_err(|e| {
                                                warn!(
                                                    "Failed to create file for Telegram voice: {}",
                                                    e
                                                );
                                                e
                                            });
                                        if let Ok(ref mut dst_file) = dst {
                                            if let Err(e) =
                                                bot.download_file(&file.path, dst_file).await
                                            {
                                                warn!(
                                                    "Failed to download Telegram voice: {}",
                                                    e
                                                );
                                            } else {
                                                let path_str = file_path.to_string_lossy().to_string();
                                                media_paths.push(path_str.clone());
                                                content = format!("{}\n[audio: {}]", content, path_str);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to get Telegram voice file info: {}", e);
                                    }
                                }

                                if !content.trim().is_empty() || !media_paths.is_empty() {
                                    let inbound_msg = InboundMessage {
                                        channel: "telegram".to_string(),
                                        sender_id,
                                        chat_id: msg.chat.id.to_string(),
                                        content,
                                        timestamp: Utc::now(),
                                        media: media_paths,
                                        metadata: HashMap::new(),
                                    };
                                    if let Err(e) = inbound_tx.send(inbound_msg).await {
                                        error!(
                                            "Failed to send Telegram inbound message: {}",
                                            e
                                        );
                                    }
                                }
                                return Ok(());
                            }

                            // Handle text-only messages
                            if let Some(text) = msg.text() {
                                let inbound_msg = InboundMessage {
                                    channel: "telegram".to_string(),
                                    sender_id,
                                    chat_id: msg.chat.id.to_string(),
                                    content: text.to_string(),
                                    timestamp: Utc::now(),
                                    media: vec![],
                                    metadata: HashMap::new(),
                                };

                                if let Err(e) = inbound_tx.send(inbound_msg).await {
                                    error!("Failed to send Telegram inbound message: {}", e);
                                }
                            }
                        }
                        Ok::<(), anyhow::Error>(())
                    }
                });

                info!("Starting Telegram dispatcher...");
                let mut dispatcher = Dispatcher::builder(bot_clone, handler).build();
                dispatcher.dispatch().await;

                // Dispatcher returned — check if we should reconnect
                if !*running.lock().await {
                    break;
                }

                let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                reconnect_attempt += 1;
                warn!(
                    "Telegram dispatcher exited, reconnecting in {} seconds...",
                    delay
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
            }
        });
        self.dispatcher_handle = Some(handle);

        info!("Telegram channel started successfully");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        *self.running.lock().await = false;
        if let Some(handle) = self.dispatcher_handle.take() {
            handle.abort();
        }
        Ok(())
    }

    async fn send_typing(&self, chat_id: &str) -> Result<()> {
        let chat_id = chat_id
            .parse::<i64>()
            .map_err(|e| anyhow::anyhow!("Invalid Telegram chat_id: {}", e))?;
        self.bot
            .send_chat_action(ChatId(chat_id), teloxide::types::ChatAction::Typing)
            .await?;
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        if msg.channel != "telegram" {
            return Ok(());
        }

        let chat_id = msg.chat_id.parse::<i64>()?;

        // Send media attachments first
        for path in &msg.media {
            let file_path = std::path::Path::new(path);
            if !file_path.exists() {
                warn!("telegram: media file not found: {}", path);
                continue;
            }
            let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let is_image = matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "webp");

            if is_image {
                if let Err(e) = self
                    .bot
                    .send_photo(ChatId(chat_id), teloxide::types::InputFile::file(file_path))
                    .await
                {
                    warn!("telegram: failed to send photo {}: {}", path, e);
                }
            } else if let Err(e) = self
                .bot
                .send_document(ChatId(chat_id), teloxide::types::InputFile::file(file_path))
                .await
            {
                warn!("telegram: failed to send document {}: {}", path, e);
            }
        }

        // Send text content
        let chunks = split_message(&msg.content, 4096);

        for chunk in chunks {
            // Convert markdown to HTML for Telegram
            let html = markdown_to_telegram_html(&chunk);

            self.bot
                .send_message(ChatId(chat_id), &html)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
        }

        Ok(())
    }

    async fn send_and_get_id(&self, msg: &OutboundMessage) -> Result<Option<String>> {
        if msg.channel != "telegram" {
            return Ok(None);
        }
        let chat_id = msg.chat_id.parse::<i64>()?;
        let html = markdown_to_telegram_html(&msg.content);
        let sent = self
            .bot
            .send_message(ChatId(chat_id), &html)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await?;
        Ok(Some(sent.id.0.to_string()))
    }

    async fn edit_message(&self, chat_id: &str, message_id: &str, content: &str) -> Result<()> {
        let chat_id = chat_id.parse::<i64>()?;
        let msg_id = message_id.parse::<i32>()?;
        let html = markdown_to_telegram_html(content);
        self.bot
            .edit_message_text(ChatId(chat_id), teloxide::types::MessageId(msg_id), &html)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await?;
        Ok(())
    }

    async fn delete_message(&self, chat_id: &str, message_id: &str) -> Result<()> {
        let chat_id = chat_id.parse::<i64>()?;
        let msg_id = message_id.parse::<i32>()?;
        self.bot
            .delete_message(ChatId(chat_id), teloxide::types::MessageId(msg_id))
            .await?;
        Ok(())
    }
}

fn markdown_to_telegram_html(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    let mut html = text.to_string();

    // Escape HTML
    html = html
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    // Convert markdown using shared regex patterns
    html = RegexPatterns::markdown_link()
        .replace_all(&html, r#"<a href="$2">$1</a>"#)
        .to_string();
    html = RegexPatterns::markdown_bold()
        .replace_all(&html, r"<b>$1</b>")
        .to_string();
    html = RegexPatterns::markdown_italic()
        .replace_all(&html, r"<i>$1</i>")
        .to_string();
    html = RegexPatterns::markdown_code()
        .replace_all(&html, r"<code>$1</code>")
        .to_string();

    html
}
