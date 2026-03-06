use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::{BaseChannel, split_message};
use crate::channels::utils::{
    DmCheckResult, check_dm_access, check_group_access, exponential_backoff_delay,
    format_pairing_reply,
};
use crate::config::TelegramConfig;
use crate::utils::regex::RegexPatterns;
use anyhow::Result;
use async_trait::async_trait;
use std::fmt::Write as _;

/// Maximum file download size for Telegram media (25 MB).
const MAX_TELEGRAM_DOWNLOAD: u32 = 25 * 1024 * 1024;
use std::sync::Arc;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{Message as TgMessage, MessageKind, Update};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

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
        let allow_groups = self.config.allow_groups.clone();
        let dm_policy = self.config.dm_policy.clone();
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
                let allow_groups_clone = allow_groups.clone();
                let dm_policy_clone = dm_policy.clone();

                let handler = Update::filter_message().endpoint(move |bot: Bot, msg: TgMessage| {
                    let inbound_tx = inbound_tx_clone.clone();
                    let allow_list = allow_list_clone.clone();
                    let allow_groups = allow_groups_clone.clone();
                    let dm_policy = dm_policy_clone.clone();
                    async move {
                        if let MessageKind::Common(_msg_common) = &msg.kind {
                            let sender_id = msg
                                .from
                                .as_ref()
                                .map(|u| u.id.to_string())
                                .unwrap_or_default();

                            // Skip messages with no sender (e.g. forwarded channel
                            // posts) — they have no identity for access control.
                            if sender_id.is_empty() {
                                debug!("ignoring Telegram message with no sender (channel post)");
                                return Ok(());
                            }

                            let is_group = msg.chat.is_group() || msg.chat.is_supergroup();
                            // Check group allowlist
                            if is_group
                                && !check_group_access(
                                    &msg.chat.id.to_string(),
                                    &allow_groups,
                                )
                            {
                                debug!(
                                    "telegram: ignoring message from non-allowed group {}",
                                    msg.chat.id
                                );
                                return Ok(());
                            }
                            // DM access check (skipped for group messages)
                            if !is_group {
                                match check_dm_access(&sender_id, &allow_list, "telegram", &dm_policy) {
                                    DmCheckResult::Allowed => {}
                                    DmCheckResult::PairingRequired { code } => {
                                        let reply = format_pairing_reply("telegram", &sender_id, &code);
                                        if let Err(e) = bot.send_message(msg.chat.id, reply).await {
                                            warn!("Failed to send pairing reply: {}", e);
                                        }
                                        return Ok(());
                                    }
                                    DmCheckResult::Denied => {
                                        return Ok(());
                                    }
                                }
                            }

                            // Handle photos
                            if let Some(photos) = msg.photo()
                                && let Some(photo) = photos.last() {
                                    let text = msg.caption().unwrap_or_default().to_string();
                                    let mut media_paths = Vec::new();
                                    let mut content = text.clone();

                                    // Download the photo
                                    match bot.get_file(photo.file.id.clone()).await {
                                        Ok(file) if file.size > MAX_TELEGRAM_DOWNLOAD => {
                                            warn!("telegram photo too large ({} bytes), skipping", file.size);
                                        }
                                        Ok(file) => {
                                            let Ok(media_dir) = crate::utils::media::media_dir() else {
                                                warn!("Failed to create media directory");
                                                return Ok(());
                                            };
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
                                                    content = format!("{content}\n[image: {path_str}]");
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            warn!("Failed to get Telegram file info: {}", e);
                                        }
                                    }

                                    if !content.trim().is_empty() || !media_paths.is_empty() {
                                        let is_group = msg.chat.is_group() || msg.chat.is_supergroup();
                                        let inbound_msg = InboundMessage::builder("telegram", sender_id, msg.chat.id.to_string(), content)
                                            .media(media_paths)
                                            .is_group(is_group)
                                            .build();
                                        if let Err(e) = inbound_tx.send(inbound_msg).await {
                                            error!(
                                                "Failed to send Telegram inbound message: {}",
                                                e
                                            );
                                        }
                                    }
                                    return Ok(());
                                }

                            // Handle voice messages
                            if let Some(voice) = msg.voice() {
                                let text = msg.caption().unwrap_or_default().to_string();
                                let mut media_paths = Vec::new();
                                let mut content = text;

                                match bot.get_file(voice.file.id.clone()).await {
                                    Ok(file) if file.size > MAX_TELEGRAM_DOWNLOAD => {
                                        warn!("telegram voice too large ({} bytes), skipping", file.size);
                                    }
                                    Ok(file) => {
                                        let Ok(media_dir) = crate::utils::media::media_dir() else {
                                            warn!("Failed to create media directory");
                                            return Ok(());
                                        };
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
                                                content = format!("{content}\n[audio: {path_str}]");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to get Telegram voice file info: {}", e);
                                    }
                                }

                                if !content.trim().is_empty() || !media_paths.is_empty() {
                                    let is_group = msg.chat.is_group() || msg.chat.is_supergroup();
                                    let inbound_msg = InboundMessage::builder("telegram", sender_id, msg.chat.id.to_string(), content)
                                        .media(media_paths)
                                        .is_group(is_group)
                                        .build();
                                    if let Err(e) = inbound_tx.send(inbound_msg).await {
                                        error!(
                                            "Failed to send Telegram inbound message: {}",
                                            e
                                        );
                                    }
                                }
                                return Ok(());
                            }

                            // Handle documents
                            if let Some(doc) = msg.document() {
                                let text = msg.caption().unwrap_or_default().to_string();
                                let mut media_paths = Vec::new();
                                let mut content = text;

                                match bot.get_file(doc.file.id.clone()).await {
                                    Ok(file) if file.size > MAX_TELEGRAM_DOWNLOAD => {
                                        warn!("telegram document too large ({} bytes), skipping", file.size);
                                    }
                                    Ok(file) => {
                                        let Ok(media_dir) = crate::utils::media::media_dir() else {
                                            warn!("Failed to create media directory");
                                            return Ok(());
                                        };
                                        // Use original extension, fall back to mime type.
                                        // Sanitize to alphanumeric to prevent path traversal.
                                        let raw_ext = doc
                                            .file_name
                                            .as_deref()
                                            .and_then(|n| n.rsplit_once('.').map(|(_, ext)| ext))
                                            .or_else(|| {
                                                doc.mime_type
                                                    .as_ref()
                                                    .map(|m| m.subtype().as_str())
                                            })
                                            .unwrap_or("bin");
                                        let ext: String = raw_ext
                                            .chars()
                                            .filter(char::is_ascii_alphanumeric)
                                            .take(10)
                                            .collect();
                                        let ext = if ext.is_empty() { "bin".to_string() } else { ext };
                                        let file_path = media_dir.join(format!(
                                            "telegram_{}.{}",
                                            doc.file.unique_id, ext
                                        ));

                                        let mut dst =
                                            tokio::fs::File::create(&file_path).await.map_err(|e| {
                                                warn!(
                                                    "Failed to create file for Telegram document: {}",
                                                    e
                                                );
                                                e
                                            });
                                        if let Ok(ref mut dst_file) = dst {
                                            if let Err(e) =
                                                bot.download_file(&file.path, dst_file).await
                                            {
                                                warn!(
                                                    "Failed to download Telegram document: {}",
                                                    e
                                                );
                                            } else {
                                                let path_str = file_path.to_string_lossy().to_string();
                                                let is_image = doc
                                                    .mime_type
                                                    .as_ref()
                                                    .is_some_and(|m| m.type_() == "image");
                                                let tag = if is_image { "image" } else { "document" };
                                                media_paths.push(path_str.clone());
                                                content =
                                                    format!("{content}\n[{tag}: {path_str}]");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to get Telegram document file info: {}", e);
                                    }
                                }

                                if !content.trim().is_empty() || !media_paths.is_empty() {
                                    let is_group = msg.chat.is_group() || msg.chat.is_supergroup();
                                    let inbound_msg = InboundMessage::builder("telegram", sender_id, msg.chat.id.to_string(), content)
                                        .media(media_paths)
                                        .is_group(is_group)
                                        .build();
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
                                let is_group = msg.chat.is_group() || msg.chat.is_supergroup();
                                let inbound_msg = InboundMessage::builder("telegram", sender_id, msg.chat.id.to_string(), text.to_string())
                                    .is_group(is_group)
                                    .build();

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
                let dispatch_start = std::time::Instant::now();
                dispatcher.dispatch().await;

                // Dispatcher returned — check if we should reconnect
                if !*running.lock().await {
                    break;
                }

                // Reset backoff if the dispatcher ran stably (>5min = healthy connection).
                // For shorter runs (>60s), halve the attempt counter to decay gradually.
                let elapsed = dispatch_start.elapsed().as_secs();
                if elapsed > 300 {
                    reconnect_attempt = 0;
                } else if elapsed > 60 && reconnect_attempt > 0 {
                    reconnect_attempt /= 2;
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
            .map_err(|e| anyhow::anyhow!("Invalid Telegram chat_id: {e}"))?;
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
            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            let is_image = matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "webp");

            if is_image {
                match self
                    .bot
                    .send_photo(ChatId(chat_id), teloxide::types::InputFile::file(file_path))
                    .await
                {
                    Ok(_) => info!("telegram: sent photo '{}'", path),
                    Err(e) => warn!("telegram: failed to send photo {}: {}", path, e),
                }
            } else {
                match self
                    .bot
                    .send_document(ChatId(chat_id), teloxide::types::InputFile::file(file_path))
                    .await
                {
                    Ok(_) => info!("telegram: sent document '{}'", path),
                    Err(e) => warn!("telegram: failed to send document {}: {}", path, e),
                }
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
        let chunks = split_message(&msg.content, 4096);
        let mut last_id = None;
        for chunk in &chunks {
            let html = markdown_to_telegram_html(chunk);
            let sent = self
                .bot
                .send_message(ChatId(chat_id), &html)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
            last_id = Some(sent.id.0.to_string());
        }
        Ok(last_id)
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

    // Extract markdown links before HTML escaping so URLs don't get
    // double-encoded (e.g. `&` -> `&amp;` inside href attributes).
    let link_re = RegexPatterns::markdown_link();
    let mut links: Vec<(String, String)> = Vec::new();
    let placeholder_prefix = "\x00LINK";
    let mut with_placeholders = String::new();
    let mut last_end = 0;
    for cap in link_re.captures_iter(text) {
        let m = cap.get(0).unwrap();
        with_placeholders.push_str(&text[last_end..m.start()]);
        let display = cap.get(1).map_or("", |c| c.as_str());
        let url = cap.get(2).map_or("", |c| c.as_str());
        let idx = links.len();
        links.push((display.to_string(), url.to_string()));
        let _ = write!(with_placeholders, "{placeholder_prefix}{idx}\x00");
        last_end = m.end();
    }
    with_placeholders.push_str(&text[last_end..]);

    // Escape HTML on the text (with link placeholders, not real URLs)
    let mut html = html_escape::encode_text(&with_placeholders).to_string();

    // Re-insert links with escaped display text but unescaped URLs in href
    for (idx, (display, url)) in links.iter().enumerate() {
        let placeholder = format!("{placeholder_prefix}{idx}\x00");
        let escaped_display = html_escape::encode_text(display);
        let escaped_url = url.replace('&', "&amp;").replace('"', "&quot;");
        let link_html = format!(r#"<a href="{escaped_url}">{escaped_display}</a>"#);
        html = html.replace(&placeholder, &link_html);
    }

    // Fenced code blocks: ```lang\n...\n``` → <pre><code>...</code></pre>
    // Must run before inline code to avoid partial matches
    html = RegexPatterns::markdown_code_block()
        .replace_all(&html, r"<pre><code>$2</code></pre>")
        .to_string();

    // Convert remaining markdown using shared regex patterns
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

#[cfg(test)]
mod tests;
