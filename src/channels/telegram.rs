use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::{BaseChannel, split_message};
use crate::config::TelegramConfig;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{Message as TgMessage, MessageKind, Update};
use tokio::sync::mpsc;

pub struct TelegramChannel {
    config: TelegramConfig,
    inbound_tx: mpsc::UnboundedSender<InboundMessage>,
    bot: Bot,
    _running: Arc<tokio::sync::Mutex<bool>>,
}

impl TelegramChannel {
    pub fn new(
        config: TelegramConfig,
        inbound_tx: mpsc::UnboundedSender<InboundMessage>,
    ) -> Self {
        let bot = Bot::new(&config.token);
        Self {
            config,
            inbound_tx,
            bot,
            _running: Arc::new(tokio::sync::Mutex::new(false)),
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
                        let sender_id = msg.from().map(|u| u.id.to_string()).unwrap_or_default();
                        
                        // Check allowlist
                        let normalized: std::collections::HashSet<String> = allow_list
                            .iter()
                            .map(|a: &String| a.trim_start_matches('+').to_string())
                            .collect();
                        
                        if !allow_list.is_empty() && !normalized.contains(&sender_id) {
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

                        let _ = inbound_tx.send(inbound_msg);
                    }
                }
                Ok::<(), anyhow::Error>(())
            }
        });

        tracing::info!("Starting Telegram dispatcher...");
        let mut dispatcher = Dispatcher::builder(bot, handler).build();
        
        // Spawn dispatcher in background task so it doesn't block
        tokio::spawn(async move {
            dispatcher.dispatch().await;
        });
        
        tracing::info!("Telegram channel started successfully");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        *self._running.lock().await = false;
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
}

fn markdown_to_telegram_html(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    // Simple markdown to HTML conversion
    let re_bold = Regex::new(r"\*\*(.+?)\*\*").unwrap();
    let re_italic = Regex::new(r"_(.+?)_").unwrap();
    let re_code = Regex::new(r"`([^`]+)`").unwrap();
    let re_link = Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap();

    let mut html = text.to_string();
    
    // Escape HTML
    html = html.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;");
    
    // Convert markdown
    html = re_link.replace_all(&html, r#"<a href="$2">$1</a>"#).to_string();
    html = re_bold.replace_all(&html, r#"<b>$1</b>"#).to_string();
    html = re_italic.replace_all(&html, r#"<i>$1</i>"#).to_string();
    html = re_code.replace_all(&html, r#"<code>$1</code>"#).to_string();

    html
}
