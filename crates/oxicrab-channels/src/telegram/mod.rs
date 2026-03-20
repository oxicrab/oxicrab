use crate::regex_utils::RegexPatterns;
use crate::utils::{
    DmCheckResult, check_dm_access, check_group_access, exponential_backoff_delay,
    format_pairing_reply,
};
use anyhow::Result;
use async_trait::async_trait;
use oxicrab_core::bus::events::{InboundMessage, OutboundMessage, meta};
use oxicrab_core::channels::base::{BaseChannel, split_message};
use oxicrab_core::config::schema::TelegramConfig;
use std::fmt::Write as _;
use std::sync::Arc;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{
    CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, Message as TgMessage,
    MessageEntityKind, MessageKind, ParseMode, ReplyParameters, Update,
};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Maximum file download size for Telegram media (25 MB).
const MAX_TELEGRAM_DOWNLOAD: u32 = 25 * 1024 * 1024;

/// Telegram limits `callback_data` to 64 bytes.
const CALLBACK_DATA_MAX_BYTES: usize = 64;

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

/// Build an `InlineKeyboardMarkup` from unified button metadata, if present.
/// Each button becomes its own row. Callback data is truncated to 64 bytes.
fn build_inline_keyboard(msg: &OutboundMessage) -> Option<InlineKeyboardMarkup> {
    let buttons_val = msg.metadata.get(meta::BUTTONS)?;
    let buttons = buttons_val.as_array()?;
    let rows: Vec<Vec<InlineKeyboardButton>> = buttons
        .iter()
        .filter_map(|b| {
            let label = b["label"].as_str()?;
            let id = b["id"].as_str()?;
            let callback_data = if let Some(ctx) = b["context"].as_str() {
                format!("{id}|{ctx}")
            } else {
                id.to_string()
            };
            // Truncate callback_data to 64 bytes at a char boundary
            let truncated = if callback_data.len() > CALLBACK_DATA_MAX_BYTES {
                callback_data[..callback_data.floor_char_boundary(CALLBACK_DATA_MAX_BYTES)]
                    .to_string()
            } else {
                callback_data
            };
            Some(InlineKeyboardButton::callback(label, truncated))
        })
        .map(|b| vec![b])
        .collect();
    if rows.is_empty() {
        return None;
    }
    Some(InlineKeyboardMarkup::new(rows))
}

/// Send a single text chunk with HTML parse mode, falling back to plain text on error.
/// Optionally attaches `reply_to` and inline keyboard markup.
async fn send_chunk(
    bot: &Bot,
    chat_id: ChatId,
    html: &str,
    raw: &str,
    reply_to_msg_id: Option<i32>,
    keyboard: Option<InlineKeyboardMarkup>,
    is_group: bool,
) -> Result<TgMessage> {
    let mut request = bot.send_message(chat_id, html).parse_mode(ParseMode::Html);
    if let Some(reply_id) = reply_to_msg_id {
        request =
            request.reply_parameters(ReplyParameters::new(teloxide::types::MessageId(reply_id)));
    }
    if let Some(ref kb) = keyboard {
        request = request.reply_markup(kb.clone());
    }
    match request.await {
        Ok(sent) => {
            // Rate limit between chunks in groups
            if is_group {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Ok(sent)
        }
        Err(e) => {
            warn!("telegram HTML send failed, retrying as plain text: {e}");
            let mut fallback = bot.send_message(chat_id, raw);
            if let Some(reply_id) = reply_to_msg_id {
                fallback = fallback
                    .reply_parameters(ReplyParameters::new(teloxide::types::MessageId(reply_id)));
            }
            if let Some(kb) = keyboard {
                fallback = fallback.reply_markup(kb);
            }
            let sent = fallback.await?;
            if is_group {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Ok(sent)
        }
    }
}

/// Determine the file extension from the Telegram file path, defaulting to the given fallback.
fn extension_from_tg_path(tg_path: &str, fallback: &str) -> String {
    std::path::Path::new(tg_path)
        .extension()
        .and_then(|e| e.to_str())
        .map_or_else(
            || fallback.to_string(),
            |e| {
                let sanitized: String = e
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .take(10)
                    .collect();
                if sanitized.is_empty() {
                    fallback.to_string()
                } else {
                    sanitized
                }
            },
        )
}

#[async_trait]
impl BaseChannel for TelegramChannel {
    fn name(&self) -> &'static str {
        "telegram"
    }

    async fn start(&mut self) -> Result<()> {
        info!("initializing Telegram bot");
        *self.running.lock().await = true;

        let bot = self.bot.clone();
        let inbound_tx = self.inbound_tx.clone();
        let allow_list = self.config.allow_from.clone();
        let allow_groups = self.config.allow_groups.clone();
        let dm_policy = self.config.dm_policy.clone();
        let mention_only = self.config.mention_only;
        let running = self.running.clone();

        // Fetch the bot's own username for mention filtering
        let bot_username: Arc<tokio::sync::Mutex<Option<String>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let bot_user_id: Arc<tokio::sync::Mutex<Option<u64>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        match bot.get_me().await {
            Ok(me) => {
                if let Some(ref username) = me.username {
                    info!("telegram bot username: @{username}");
                    *bot_username.lock().await = Some(username.clone());
                }
                *bot_user_id.lock().await = Some(me.id.0);
            }
            Err(e) => {
                warn!("telegram: failed to fetch bot info: {e}");
            }
        }

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
                let bot_username_clone = bot_username.clone();
                let bot_user_id_clone = bot_user_id.clone();

                // Callback query handler for inline keyboard button clicks
                let cb_inbound_tx = inbound_tx.clone();
                let cb_allow_list = allow_list.clone();
                let cb_allow_groups = allow_groups.clone();
                let cb_dm_policy = dm_policy.clone();

                let message_handler =
                    Update::filter_message().endpoint(move |bot: Bot, msg: TgMessage| {
                        let inbound_tx = inbound_tx_clone.clone();
                        let allow_list = allow_list_clone.clone();
                        let allow_groups = allow_groups_clone.clone();
                        let dm_policy = dm_policy_clone.clone();
                        let bot_username = bot_username_clone.clone();
                        let bot_user_id = bot_user_id_clone.clone();
                        async move {
                            handle_message(
                                bot,
                                msg,
                                &inbound_tx,
                                &allow_list,
                                &allow_groups,
                                &dm_policy,
                                mention_only,
                                &bot_username,
                                &bot_user_id,
                            )
                            .await
                        }
                    });

                let callback_handler =
                    Update::filter_callback_query().endpoint(move |bot: Bot, q: CallbackQuery| {
                        let inbound_tx = cb_inbound_tx.clone();
                        let allow_list = cb_allow_list.clone();
                        let allow_groups = cb_allow_groups.clone();
                        let dm_policy = cb_dm_policy.clone();
                        async move {
                            handle_callback_query(
                                bot,
                                q,
                                &inbound_tx,
                                &allow_list,
                                &allow_groups,
                                &dm_policy,
                            )
                            .await
                        }
                    });

                let handler = dptree::entry()
                    .branch(message_handler)
                    .branch(callback_handler);

                info!("starting Telegram dispatcher");
                let mut dispatcher = Dispatcher::builder(bot_clone, handler).build();
                let dispatch_start = std::time::Instant::now();
                dispatcher.dispatch().await;

                // Dispatcher returned -- check if we should reconnect
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
                    "Telegram dispatcher exited, reconnecting in {} seconds",
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
            .map_err(|e| anyhow::anyhow!("invalid Telegram chat_id: {e}"))?;
        self.bot
            .send_chat_action(ChatId(chat_id), teloxide::types::ChatAction::Typing)
            .await?;
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        if msg.channel != "telegram" {
            return Ok(());
        }

        let chat_id_num = msg.chat_id.parse::<i64>()?;
        let tg_chat_id = ChatId(chat_id_num);

        // Determine if target is a group (for rate limiting between chunks)
        // We approximate by checking if chat_id is negative (groups have negative IDs).
        let is_group = chat_id_num < 0;

        // Wire reply_to for the first chunk only
        let reply_to_msg_id = msg
            .reply_to
            .as_deref()
            .and_then(|id| id.parse::<i32>().ok());

        // Send media attachments first
        send_media_attachments(&self.bot, tg_chat_id, &msg.media).await;

        // Fix #7: convert markdown to HTML first, THEN split
        let html_content = markdown_to_telegram_html(&msg.content);
        let html_chunks = split_message(&html_content, 4096);
        // Also split raw content for fallback (matched by index)
        let raw_chunks = split_message(&msg.content, 4096);

        // Fix #1: build inline keyboard from unified button metadata
        let keyboard = build_inline_keyboard(msg);

        for (i, html_chunk) in html_chunks.iter().enumerate() {
            let raw_chunk = raw_chunks
                .get(i)
                .map_or(html_chunk.as_str(), String::as_str);
            // Only reply_to on the first chunk
            let reply_id = if i == 0 { reply_to_msg_id } else { None };
            // Attach keyboard to the last chunk only
            let kb = if i == html_chunks.len() - 1 {
                keyboard.clone()
            } else {
                None
            };
            send_chunk(
                &self.bot, tg_chat_id, html_chunk, raw_chunk, reply_id, kb, is_group,
            )
            .await?;
        }

        Ok(())
    }

    async fn send_and_get_id(&self, msg: &OutboundMessage) -> Result<Option<String>> {
        if msg.channel != "telegram" {
            return Ok(None);
        }
        let chat_id_num = msg.chat_id.parse::<i64>()?;
        let tg_chat_id = ChatId(chat_id_num);
        let is_group = chat_id_num < 0;
        let reply_to_msg_id = msg
            .reply_to
            .as_deref()
            .and_then(|id| id.parse::<i32>().ok());

        // Fix #3: send media like send() does
        send_media_attachments(&self.bot, tg_chat_id, &msg.media).await;

        // Fix #7: convert then split
        let html_content = markdown_to_telegram_html(&msg.content);
        let html_chunks = split_message(&html_content, 4096);
        let raw_chunks = split_message(&msg.content, 4096);

        let keyboard = build_inline_keyboard(msg);

        let mut last_id = None;
        for (i, html_chunk) in html_chunks.iter().enumerate() {
            let raw_chunk = raw_chunks
                .get(i)
                .map_or(html_chunk.as_str(), String::as_str);
            let reply_id = if i == 0 { reply_to_msg_id } else { None };
            let kb = if i == html_chunks.len() - 1 {
                keyboard.clone()
            } else {
                None
            };
            let sent = send_chunk(
                &self.bot, tg_chat_id, html_chunk, raw_chunk, reply_id, kb, is_group,
            )
            .await?;
            last_id = Some(sent.id.0.to_string());
        }
        Ok(last_id)
    }

    async fn edit_message(&self, chat_id: &str, message_id: &str, content: &str) -> Result<()> {
        let chat_id = chat_id.parse::<i64>()?;
        let msg_id = message_id.parse::<i32>()?;
        let html = markdown_to_telegram_html(content);
        // Fix #4: truncate to 4096 chars for edit_message_text
        let truncated = if html.len() > 4096 {
            &html[..html.floor_char_boundary(4096)]
        } else {
            &html
        };
        self.bot
            .edit_message_text(
                ChatId(chat_id),
                teloxide::types::MessageId(msg_id),
                truncated,
            )
            .parse_mode(ParseMode::Html)
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

/// Send media attachments (photos, documents) for an outbound message.
async fn send_media_attachments(bot: &Bot, chat_id: ChatId, media: &[String]) {
    for path in media {
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
            match bot
                .send_photo(chat_id, teloxide::types::InputFile::file(file_path))
                .await
            {
                Ok(_) => info!("telegram: sent photo '{}'", path),
                Err(e) => warn!("telegram: failed to send photo {}: {}", path, e),
            }
        } else {
            match bot
                .send_document(chat_id, teloxide::types::InputFile::file(file_path))
                .await
            {
                Ok(_) => info!("telegram: sent document '{}'", path),
                Err(e) => warn!("telegram: failed to send document {}: {}", path, e),
            }
        }
    }
}

/// Handle an incoming Telegram message (text, photo, voice, document, video, audio, animation).
#[allow(clippy::too_many_arguments)]
async fn handle_message(
    bot: Bot,
    msg: TgMessage,
    inbound_tx: &mpsc::Sender<InboundMessage>,
    allow_list: &[String],
    allow_groups: &[String],
    dm_policy: &oxicrab_core::config::schema::DmPolicy,
    mention_only: bool,
    bot_username: &Arc<tokio::sync::Mutex<Option<String>>>,
    bot_user_id: &Arc<tokio::sync::Mutex<Option<u64>>>,
) -> Result<()> {
    if !matches!(&msg.kind, MessageKind::Common(_)) {
        return Ok(());
    }

    let sender_id = msg
        .from
        .as_ref()
        .map(|u| u.id.to_string())
        .unwrap_or_default();

    // Skip messages with no sender (e.g. forwarded channel posts)
    if sender_id.is_empty() {
        debug!("ignoring Telegram message with no sender (channel post)");
        return Ok(());
    }

    let is_group = msg.chat.is_group() || msg.chat.is_supergroup();

    // Check group allowlist
    if is_group && !check_group_access(&msg.chat.id.to_string(), allow_groups) {
        debug!(
            "telegram: ignoring message from non-allowed group {}",
            msg.chat.id
        );
        return Ok(());
    }

    // DM access check (skipped for group messages)
    if !is_group {
        match check_dm_access(&sender_id, allow_list, "telegram", dm_policy) {
            DmCheckResult::Allowed => {}
            DmCheckResult::PairingRequired { code } => {
                let reply = format_pairing_reply("telegram", &sender_id, &code);
                if let Err(e) = bot.send_message(msg.chat.id, reply).await {
                    warn!("failed to send pairing reply: {}", e);
                }
                return Ok(());
            }
            DmCheckResult::Denied => {
                return Ok(());
            }
        }
    }

    // Fix #6: mention-only filtering in groups
    if is_group && mention_only {
        let is_mentioned = is_bot_mentioned(&msg, bot_username, bot_user_id).await;
        if !is_mentioned {
            debug!("telegram: ignoring group message (mention_only enabled, bot not mentioned)");
            return Ok(());
        }
    }

    // Common builder setup: set ts metadata (Fix #5)
    let build_msg = |sender: String, content: String, media: Vec<String>| {
        InboundMessage::builder("telegram", sender, msg.chat.id.to_string(), content)
            .media(media)
            .is_group(is_group)
            .meta(meta::TS, serde_json::Value::String(msg.id.0.to_string()))
    };

    // Handle photos
    if let Some(photos) = msg.photo()
        && let Some(photo) = photos.last()
    {
        let text = msg.caption().unwrap_or_default().to_string();
        let mut media_paths = Vec::new();
        let mut content = text;

        match bot.get_file(photo.file.id.clone()).await {
            Ok(file) if file.size > MAX_TELEGRAM_DOWNLOAD => {
                warn!("telegram photo too large ({} bytes), skipping", file.size);
            }
            Ok(file) => {
                // Fix #12: use file extension from Telegram path
                let ext = extension_from_tg_path(&file.path, "jpg");
                if let Some(path_str) =
                    download_file(&bot, &file.path, &photo.file.unique_id.0, &ext, "photo").await
                {
                    media_paths.push(path_str.clone());
                    content = format!("{content}\n[image: {path_str}]");
                }
            }
            Err(e) => {
                warn!("failed to get Telegram file info: {}", e);
            }
        }

        if !content.trim().is_empty() || !media_paths.is_empty() {
            let inbound_msg = build_msg(sender_id, content, media_paths).build();
            if let Err(e) = inbound_tx.send(inbound_msg).await {
                error!("failed to send Telegram inbound message: {}", e);
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
                if let Some(path_str) =
                    download_file(&bot, &file.path, &voice.file.unique_id.0, "ogg", "voice").await
                {
                    media_paths.push(path_str.clone());
                    content = format!("{content}\n[audio: {path_str}]");
                }
            }
            Err(e) => {
                warn!("failed to get Telegram voice file info: {}", e);
            }
        }

        if !content.trim().is_empty() || !media_paths.is_empty() {
            let inbound_msg = build_msg(sender_id, content, media_paths).build();
            if let Err(e) = inbound_tx.send(inbound_msg).await {
                error!("failed to send Telegram inbound message: {}", e);
            }
        }
        return Ok(());
    }

    // Fix #10: handle video messages
    if let Some(video) = msg.video() {
        let text = msg.caption().unwrap_or_default().to_string();
        let mut media_paths = Vec::new();
        let mut content = text;

        match bot.get_file(video.file.id.clone()).await {
            Ok(file) if file.size > MAX_TELEGRAM_DOWNLOAD => {
                warn!("telegram video too large ({} bytes), skipping", file.size);
            }
            Ok(file) => {
                let ext = extension_from_tg_path(&file.path, "mp4");
                if let Some(path_str) =
                    download_file(&bot, &file.path, &video.file.unique_id.0, &ext, "video").await
                {
                    media_paths.push(path_str.clone());
                    content = format!("{content}\n[video: {path_str}]");
                }
            }
            Err(e) => {
                warn!("failed to get Telegram video file info: {}", e);
            }
        }

        if !content.trim().is_empty() || !media_paths.is_empty() {
            let inbound_msg = build_msg(sender_id, content, media_paths).build();
            if let Err(e) = inbound_tx.send(inbound_msg).await {
                error!("failed to send Telegram inbound message: {}", e);
            }
        }
        return Ok(());
    }

    // Fix #10: handle animation (GIF) messages
    if let Some(animation) = msg.animation() {
        let text = msg.caption().unwrap_or_default().to_string();
        let mut media_paths = Vec::new();
        let mut content = text;

        match bot.get_file(animation.file.id.clone()).await {
            Ok(file) if file.size > MAX_TELEGRAM_DOWNLOAD => {
                warn!(
                    "telegram animation too large ({} bytes), skipping",
                    file.size
                );
            }
            Ok(file) => {
                let ext = extension_from_tg_path(&file.path, "mp4");
                if let Some(path_str) = download_file(
                    &bot,
                    &file.path,
                    &animation.file.unique_id.0,
                    &ext,
                    "animation",
                )
                .await
                {
                    media_paths.push(path_str.clone());
                    content = format!("{content}\n[image: {path_str}]");
                }
            }
            Err(e) => {
                warn!("failed to get Telegram animation file info: {}", e);
            }
        }

        if !content.trim().is_empty() || !media_paths.is_empty() {
            let inbound_msg = build_msg(sender_id, content, media_paths).build();
            if let Err(e) = inbound_tx.send(inbound_msg).await {
                error!("failed to send Telegram inbound message: {}", e);
            }
        }
        return Ok(());
    }

    // Fix #10: handle audio messages
    if let Some(audio) = msg.audio() {
        let text = msg.caption().unwrap_or_default().to_string();
        let mut media_paths = Vec::new();
        let mut content = text;

        match bot.get_file(audio.file.id.clone()).await {
            Ok(file) if file.size > MAX_TELEGRAM_DOWNLOAD => {
                warn!("telegram audio too large ({} bytes), skipping", file.size);
            }
            Ok(file) => {
                let ext = extension_from_tg_path(&file.path, "mp3");
                if let Some(path_str) =
                    download_file(&bot, &file.path, &audio.file.unique_id.0, &ext, "audio").await
                {
                    media_paths.push(path_str.clone());
                    content = format!("{content}\n[audio: {path_str}]");
                }
            }
            Err(e) => {
                warn!("failed to get Telegram audio file info: {}", e);
            }
        }

        if !content.trim().is_empty() || !media_paths.is_empty() {
            let inbound_msg = build_msg(sender_id, content, media_paths).build();
            if let Err(e) = inbound_tx.send(inbound_msg).await {
                error!("failed to send Telegram inbound message: {}", e);
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
                warn!(
                    "telegram document too large ({} bytes), skipping",
                    file.size
                );
            }
            Ok(file) => {
                // Use original extension, fall back to mime type.
                // Sanitize to alphanumeric to prevent path traversal.
                let raw_ext = doc
                    .file_name
                    .as_deref()
                    .and_then(|n| n.rsplit_once('.').map(|(_, ext)| ext))
                    .or_else(|| doc.mime_type.as_ref().map(|m| m.subtype().as_str()))
                    .unwrap_or("bin");
                let ext: String = raw_ext
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .take(10)
                    .collect();
                let ext = if ext.is_empty() {
                    "bin".to_string()
                } else {
                    ext
                };
                if let Some(path_str) =
                    download_file(&bot, &file.path, &doc.file.unique_id.0, &ext, "document").await
                {
                    let is_image = doc.mime_type.as_ref().is_some_and(|m| m.type_() == "image");
                    let tag = if is_image { "image" } else { "document" };
                    media_paths.push(path_str.clone());
                    content = format!("{content}\n[{tag}: {path_str}]");
                }
            }
            Err(e) => {
                warn!("failed to get Telegram document file info: {}", e);
            }
        }

        if !content.trim().is_empty() || !media_paths.is_empty() {
            let inbound_msg = build_msg(sender_id, content, media_paths).build();
            if let Err(e) = inbound_tx.send(inbound_msg).await {
                error!("failed to send Telegram inbound message: {}", e);
            }
        }
        return Ok(());
    }

    // Handle text-only messages
    if let Some(text) = msg.text() {
        let inbound_msg = build_msg(sender_id, text.to_string(), Vec::new()).build();
        if let Err(e) = inbound_tx.send(inbound_msg).await {
            error!("failed to send Telegram inbound message: {}", e);
        }
    }

    Ok(())
}

/// Handle callback queries from inline keyboard button clicks.
async fn handle_callback_query(
    bot: Bot,
    q: CallbackQuery,
    inbound_tx: &mpsc::Sender<InboundMessage>,
    allow_list: &[String],
    allow_groups: &[String],
    dm_policy: &oxicrab_core::config::schema::DmPolicy,
) -> Result<()> {
    let sender_id = q.from.id.to_string();
    let callback_data = q.data.as_deref().unwrap_or_default();
    if callback_data.is_empty() {
        return Ok(());
    }

    // Determine chat from the message the button was attached to
    let Some(ref message) = q.message else {
        // Answer the callback even if we can't process it
        let _ = bot.answer_callback_query(q.id.clone()).await;
        return Ok(());
    };
    let chat_id = message.chat().id;
    let is_group = message.chat().is_group() || message.chat().is_supergroup();

    // Access control
    if is_group && !check_group_access(&chat_id.to_string(), allow_groups) {
        debug!(
            "telegram: ignoring callback from non-allowed group {}",
            chat_id
        );
        let _ = bot.answer_callback_query(q.id.clone()).await;
        return Ok(());
    }
    if !is_group {
        match check_dm_access(&sender_id, allow_list, "telegram", dm_policy) {
            DmCheckResult::Allowed => {}
            DmCheckResult::PairingRequired { .. } | DmCheckResult::Denied => {
                let _ = bot.answer_callback_query(q.id.clone()).await;
                return Ok(());
            }
        }
    }

    // Parse callback_data: "action_id|context" or just "action_id"
    let (action_id, context_str) = callback_data
        .split_once('|')
        .map_or((callback_data, ""), |(id, ctx)| (id, ctx));

    // Try to parse context as ActionDispatchPayload for direct dispatch
    let (content, dispatch) = if context_str.is_empty() {
        (format!("[button:{action_id}]"), None)
    } else if let Ok(payload) =
        serde_json::from_str::<crate::dispatch::ActionDispatchPayload>(context_str)
    {
        let dispatch = oxicrab_core::dispatch::ActionDispatch {
            tool: payload.tool,
            params: payload.params,
            source: oxicrab_core::dispatch::ActionSource::Button {
                action_id: action_id.to_string(),
            },
        };
        (format!("[button:{action_id}]"), Some(dispatch))
    } else {
        // Legacy fallback: send as text to LLM
        (
            format!("[button:{action_id}]\nButton context: {context_str}"),
            None,
        )
    };

    let mut builder =
        InboundMessage::builder("telegram", sender_id.clone(), chat_id.to_string(), content)
            .meta(
                "action_id",
                serde_json::Value::String(action_id.to_string()),
            )
            .is_group(is_group);
    if !context_str.is_empty() {
        builder = builder.meta(
            "button_context",
            serde_json::Value::String(context_str.to_string()),
        );
    }
    if let Some(d) = dispatch {
        builder = builder.action(d);
    }
    let inbound_msg = builder.build();

    if let Err(e) = inbound_tx.send(inbound_msg).await {
        error!("failed to send Telegram callback inbound message: {}", e);
    }

    // Acknowledge the callback query
    if let Err(e) = bot.answer_callback_query(q.id.clone()).await {
        warn!("failed to answer Telegram callback query: {}", e);
    }

    info!("telegram: button click action_id={action_id} from user={sender_id} in chat={chat_id}");
    Ok(())
}

/// Check if the bot is mentioned in a group message (via @mention or reply).
async fn is_bot_mentioned(
    msg: &TgMessage,
    bot_username: &Arc<tokio::sync::Mutex<Option<String>>>,
    bot_user_id: &Arc<tokio::sync::Mutex<Option<u64>>>,
) -> bool {
    // Check for @mention in message entities
    if let Some(entities) = msg.entities() {
        let username_guard = bot_username.lock().await;
        if let Some(ref bot_name) = *username_guard {
            for entity in entities {
                if let MessageEntityKind::Mention = entity.kind {
                    // Extract the mention text from the message
                    let text = msg.text().unwrap_or_default();
                    if let Some(mention) = text.get(entity.offset..entity.offset + entity.length) {
                        // Telegram mentions include the @ prefix
                        if mention
                            .strip_prefix('@')
                            .unwrap_or(mention)
                            .eq_ignore_ascii_case(bot_name)
                        {
                            return true;
                        }
                    }
                }
            }
        }
    }

    // Check if this is a reply to the bot
    if let Some(reply) = msg.reply_to_message()
        && let Some(ref from) = reply.from
    {
        let bot_id_guard = bot_user_id.lock().await;
        if let Some(bid) = *bot_id_guard
            && from.id.0 == bid
        {
            return true;
        }
    }

    false
}

/// Download a Telegram file to the media directory.
/// Returns the local file path as a string, or `None` on failure.
async fn download_file(
    bot: &Bot,
    tg_file_path: &str,
    unique_id: &str,
    ext: &str,
    label: &str,
) -> Option<String> {
    let media_dir = crate::media_utils::media_dir().ok()?;
    let file_path = media_dir.join(format!("telegram_{unique_id}.{ext}"));

    let mut dst = match tokio::fs::File::create(&file_path).await {
        Ok(f) => f,
        Err(e) => {
            warn!("failed to create file for Telegram {label}: {e}");
            return None;
        }
    };
    if let Err(e) = bot.download_file(tg_file_path, &mut dst).await {
        warn!("failed to download Telegram {label}: {e}");
        return None;
    }
    Some(file_path.to_string_lossy().to_string())
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
        let escaped_url = url
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;");
        let link_html = format!(r#"<a href="{escaped_url}">{escaped_display}</a>"#);
        html = html.replace(&placeholder, &link_html);
    }

    // Fenced code blocks: ```lang\n...\n``` -> <pre><code>...</code></pre>
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
    // Fix #8: strikethrough ~~text~~ -> <s>text</s>
    html = RegexPatterns::markdown_strike()
        .replace_all(&html, r"<s>$1</s>")
        .to_string();
    html = RegexPatterns::markdown_code()
        .replace_all(&html, r"<code>$1</code>")
        .to_string();

    html
}

#[cfg(test)]
mod tests;
