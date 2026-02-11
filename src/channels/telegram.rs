use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::{split_message, BaseChannel};
use crate::channels::utils::check_allowed_sender;
use crate::config::TelegramConfig;
use crate::utils::regex::RegexPatterns;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{Message as TgMessage, MessageKind, Update};
use tokio::sync::mpsc;

pub struct TelegramChannel {
    config: TelegramConfig,
    inbound_tx: mpsc::Sender<InboundMessage>,
    bot: Bot,
    _running: Arc<tokio::sync::Mutex<bool>>,
    dispatcher_handle: Option<tokio::task::JoinHandle<()>>,
}

impl TelegramChannel {
    pub fn new(config: TelegramConfig, inbound_tx: mpsc::Sender<InboundMessage>) -> Self {
        let bot = Bot::new(&config.token);
        Self {
            config,
            inbound_tx,
            bot,
            _running: Arc::new(tokio::sync::Mutex::new(false)),
            dispatcher_handle: None,
        }
    }
}

#[async_trait]
impl BaseChannel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    async fn start(&mut self) -> Result<()> {
        tracing::info!("Initializing Telegram bot...");
        *self._running.lock().await = true;

        let bot = self.bot.clone();
        let inbound_tx = self.inbound_tx.clone();
        let allow_list = self.config.allow_from.clone();

        let handler = Update::filter_message().endpoint(move |msg: TgMessage| {
            let inbound_tx = inbound_tx.clone();
            let allow_list = allow_list.clone();
            async move {
                if let MessageKind::Common(_msg_common) = &msg.kind {
                    let text = msg.text();
                    if let Some(text) = text {
                        let sender_id = msg
                            .from
                            .as_ref()
                            .map(|u| u.id.to_string())
                            .unwrap_or_default();

                        // Check allowlist using utility function
                        if !check_allowed_sender(&sender_id, &allow_list) {
                            return Ok(());
                        }

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
                            tracing::error!("Failed to send Telegram inbound message: {}", e);
                        }
                    }
                }
                Ok::<(), anyhow::Error>(())
            }
        });

        tracing::info!("Starting Telegram dispatcher...");
        let mut dispatcher = Dispatcher::builder(bot, handler).build();

        // Spawn dispatcher in background task and track handle
        let handle = tokio::spawn(async move {
            dispatcher.dispatch().await;
        });
        self.dispatcher_handle = Some(handle);

        tracing::info!("Telegram channel started successfully");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        *self._running.lock().await = false;
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

    async fn edit_message(&self, chat_id: &str, message_id: &str, new_content: &str) -> Result<()> {
        let chat_id = chat_id
            .parse::<i64>()
            .map_err(|e| anyhow::anyhow!("Invalid Telegram chat_id: {}", e))?;
        let msg_id = message_id
            .parse::<i32>()
            .map_err(|e| anyhow::anyhow!("Invalid Telegram message_id: {}", e))?;
        let html = markdown_to_telegram_html(new_content);

        self.bot
            .edit_message_text(ChatId(chat_id), teloxide::types::MessageId(msg_id), &html)
            .parse_mode(teloxide::types::ParseMode::Html)
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
