use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::{BaseChannel, split_message};
use crate::channels::utils::{
    DmCheckResult, check_dm_access, exponential_backoff_delay, format_pairing_reply,
};
use crate::config::{DiscordCommand, DiscordConfig};
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serenity::async_trait as serenity_async_trait;
use serenity::builder::{
    CreateActionRow, CreateButton, CreateCommand, CreateCommandOption, CreateEmbed,
    CreateEmbedFooter, CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage,
};
use serenity::model::application::{ButtonStyle, CommandOptionType, Interaction};
use serenity::model::channel::Message as DiscordMessage;
use serenity::model::gateway::{GatewayIntents, Ready};
use serenity::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

struct Handler {
    inbound_tx: mpsc::Sender<InboundMessage>,
    allow_list: Vec<String>,
    dm_policy: crate::config::DmPolicy,
    http_client: reqwest::Client,
    commands: Vec<DiscordCommand>,
}

impl Handler {
    async fn handle_command(
        &self,
        ctx: &Context,
        cmd: serenity::model::application::CommandInteraction,
    ) {
        let sender_id = cmd.user.id.to_string();

        match check_dm_access(&sender_id, &self.allow_list, "discord", &self.dm_policy) {
            DmCheckResult::Allowed => {}
            DmCheckResult::PairingRequired { code } => {
                let reply = format_pairing_reply("discord", &sender_id, &code);
                let response = CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content(reply)
                        .ephemeral(true),
                );
                if let Err(e) = cmd.create_response(&ctx.http, response).await {
                    warn!("Failed to send pairing response: {}", e);
                }
                return;
            }
            DmCheckResult::Denied => {
                let response = CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("You are not authorized to use this bot.")
                        .ephemeral(true),
                );
                if let Err(e) = cmd.create_response(&ctx.http, response).await {
                    warn!("Failed to send unauthorized response: {}", e);
                }
                return;
            }
        }

        // Defer response — shows "thinking..."
        let defer = CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new());
        if let Err(e) = cmd.create_response(&ctx.http, defer).await {
            error!("Failed to defer Discord interaction: {}", e);
            return;
        }

        // Extract content from command options
        let content: String = cmd
            .data
            .options
            .iter()
            .filter_map(|opt| opt.value.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        let content = if content.is_empty() {
            format!("/{}", cmd.data.name)
        } else {
            content
        };

        let mut metadata = HashMap::new();
        metadata.insert(
            "discord_interaction_token".to_string(),
            serde_json::Value::String(cmd.token.clone()),
        );
        metadata.insert(
            "discord_application_id".to_string(),
            serde_json::Value::String(cmd.application_id.to_string()),
        );

        let inbound_msg = InboundMessage {
            channel: "discord".to_string(),
            sender_id,
            chat_id: cmd.channel_id.to_string(),
            content,
            timestamp: Utc::now(),
            media: Vec::new(),
            metadata,
        };

        if let Err(e) = self.inbound_tx.send(inbound_msg).await {
            error!("Failed to send Discord slash command to bus: {}", e);
        }
    }

    async fn handle_component(
        &self,
        ctx: &Context,
        comp: serenity::model::application::ComponentInteraction,
    ) {
        let sender_id = comp.user.id.to_string();

        match check_dm_access(&sender_id, &self.allow_list, "discord", &self.dm_policy) {
            DmCheckResult::Allowed => {}
            DmCheckResult::PairingRequired { code } => {
                let reply = format_pairing_reply("discord", &sender_id, &code);
                let response = CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content(reply)
                        .ephemeral(true),
                );
                if let Err(e) = comp.create_response(&ctx.http, response).await {
                    warn!("Failed to send pairing response: {}", e);
                }
                return;
            }
            DmCheckResult::Denied => {
                let response = CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("You are not authorized to use this bot.")
                        .ephemeral(true),
                );
                if let Err(e) = comp.create_response(&ctx.http, response).await {
                    warn!("Failed to send unauthorized response: {}", e);
                }
                return;
            }
        }

        // Defer update
        let defer = CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new());
        if let Err(e) = comp.create_response(&ctx.http, defer).await {
            error!("Failed to defer Discord component interaction: {}", e);
            return;
        }

        let custom_id = comp.data.custom_id.clone();
        let content = format!("[button:{}]", custom_id);

        let mut metadata = HashMap::new();
        metadata.insert(
            "discord_interaction_token".to_string(),
            serde_json::Value::String(comp.token.clone()),
        );
        metadata.insert(
            "discord_application_id".to_string(),
            serde_json::Value::String(comp.application_id.to_string()),
        );
        metadata.insert(
            "discord_component_id".to_string(),
            serde_json::Value::String(custom_id),
        );

        let inbound_msg = InboundMessage {
            channel: "discord".to_string(),
            sender_id,
            chat_id: comp.channel_id.to_string(),
            content,
            timestamp: Utc::now(),
            media: Vec::new(),
            metadata,
        };

        if let Err(e) = self.inbound_tx.send(inbound_msg).await {
            error!("Failed to send Discord component interaction to bus: {}", e);
        }
    }
}

#[serenity_async_trait]
impl EventHandler for Handler {
    async fn cache_ready(&self, _ctx: Context, _guilds: Vec<serenity::model::id::GuildId>) {
        info!("Discord cache is ready");
    }

    async fn message(&self, ctx: Context, msg: DiscordMessage) {
        if msg.author.bot {
            return;
        }

        let sender_id = msg.author.id.to_string();

        match check_dm_access(&sender_id, &self.allow_list, "discord", &self.dm_policy) {
            DmCheckResult::Allowed => {}
            DmCheckResult::PairingRequired { code } => {
                let reply = format_pairing_reply("discord", &sender_id, &code);
                if let Err(e) = msg.reply(&ctx.http, &reply).await {
                    warn!("Failed to send pairing reply: {}", e);
                }
                return;
            }
            DmCheckResult::Denied => {
                return;
            }
        }

        // Download image attachments
        let mut media_paths = Vec::new();
        let mut content = msg.content.clone();
        for attachment in &msg.attachments {
            let content_type = attachment.content_type.as_deref().unwrap_or("");
            let is_image = content_type.starts_with("image/");
            let is_audio = content_type.starts_with("audio/");
            if !is_image && !is_audio {
                continue;
            }
            let (ext, tag) = if is_image {
                (
                    match content_type {
                        "image/jpeg" => "jpg",
                        "image/png" => "png",
                        "image/gif" => "gif",
                        "image/webp" => "webp",
                        _ => "bin",
                    },
                    "image",
                )
            } else {
                (
                    match content_type {
                        "audio/mpeg" => "mp3",
                        "audio/wav" => "wav",
                        "audio/webm" => "webm",
                        "audio/mp4" => "m4a",
                        _ => "ogg",
                    },
                    "audio",
                )
            };
            let Ok(media_dir) = crate::utils::media::media_dir() else {
                warn!("Failed to create media directory");
                continue;
            };
            let file_path = media_dir.join(format!("discord_{}.{}", attachment.id, ext));

            match self.http_client.get(&attachment.url).send().await {
                Ok(resp) => match resp.bytes().await {
                    Ok(bytes) => {
                        if let Err(e) = std::fs::write(&file_path, &bytes) {
                            warn!("Failed to write Discord media file: {}", e);
                        }
                        let path_str = file_path.to_string_lossy().to_string();
                        media_paths.push(path_str.clone());
                        content = format!("{}\n[{}: {}]", content, tag, path_str);
                    }
                    Err(e) => warn!("Failed to download Discord attachment: {}", e),
                },
                Err(e) => warn!("Failed to download Discord attachment: {}", e),
            }
        }

        let inbound_msg = InboundMessage {
            channel: "discord".to_string(),
            sender_id,
            chat_id: msg.channel_id.to_string(),
            content,
            timestamp: Utc::now(),
            media: media_paths,
            metadata: HashMap::new(),
        };

        if let Err(e) = self.inbound_tx.send(inbound_msg).await {
            error!("Failed to send Discord inbound message: {}", e);
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(cmd) => self.handle_command(&ctx, cmd).await,
            Interaction::Component(comp) => self.handle_component(&ctx, comp).await,
            _ => {}
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!(
            "Discord bot connected as {} (id: {})",
            ready.user.name, ready.user.id
        );

        // Register slash commands
        for cmd_config in &self.commands {
            let mut command =
                CreateCommand::new(&cmd_config.name).description(&cmd_config.description);
            for opt in &cmd_config.options {
                command = command.add_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        &opt.name,
                        &opt.description,
                    )
                    .required(opt.required),
                );
            }
            match serenity::model::application::Command::create_global_command(&ctx.http, command)
                .await
            {
                Ok(cmd) => info!("Registered Discord slash command: /{}", cmd.name),
                Err(e) => error!(
                    "Failed to register Discord slash command /{}: {}",
                    cmd_config.name, e
                ),
            }
        }
    }
}

pub struct DiscordChannel {
    config: DiscordConfig,
    inbound_tx: mpsc::Sender<InboundMessage>,
    running: Arc<tokio::sync::Mutex<bool>>,
    http_client: reqwest::Client,
    serenity_http: Arc<serenity::http::Http>,
    _client_handle: Option<tokio::task::JoinHandle<()>>,
}

impl DiscordChannel {
    pub fn new(config: DiscordConfig, inbound_tx: mpsc::Sender<InboundMessage>) -> Self {
        let serenity_http = Arc::new(serenity::http::Http::new(&config.token));
        Self {
            config,
            inbound_tx,
            running: Arc::new(tokio::sync::Mutex::new(false)),
            http_client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            serenity_http,
            _client_handle: None,
        }
    }
}

fn parse_embeds_from_metadata(metadata: &HashMap<String, serde_json::Value>) -> Vec<CreateEmbed> {
    let Some(embeds_val) = metadata.get("discord_embeds") else {
        return Vec::new();
    };
    let Some(embeds_arr) = embeds_val.as_array() else {
        return Vec::new();
    };

    embeds_arr
        .iter()
        .map(|e| {
            let mut embed = CreateEmbed::new();
            if let Some(title) = e["title"].as_str() {
                embed = embed.title(title);
            }
            if let Some(desc) = e["description"].as_str() {
                embed = embed.description(desc);
            }
            if let Some(color) = e["color"].as_u64() {
                embed = embed.color(color as u32);
            }
            if let Some(url) = e["url"].as_str() {
                embed = embed.url(url);
            }
            if let Some(footer) = e["footer"].as_str() {
                embed = embed.footer(CreateEmbedFooter::new(footer));
            }
            if let Some(thumb) = e["thumbnail"].as_str() {
                embed = embed.thumbnail(thumb);
            }
            if let Some(image) = e["image"].as_str() {
                embed = embed.image(image);
            }
            if let Some(fields) = e["fields"].as_array() {
                for f in fields {
                    let name = f["name"].as_str().unwrap_or("—");
                    let value = f["value"].as_str().unwrap_or("—");
                    let inline = f["inline"].as_bool().unwrap_or(false);
                    embed = embed.field(name, value, inline);
                }
            }
            embed
        })
        .collect()
}

fn parse_button_style(style: &str) -> ButtonStyle {
    match style {
        "primary" => ButtonStyle::Primary,
        "success" => ButtonStyle::Success,
        "danger" => ButtonStyle::Danger,
        _ => ButtonStyle::Secondary,
    }
}

fn parse_components_from_metadata(
    metadata: &HashMap<String, serde_json::Value>,
) -> Vec<CreateActionRow> {
    let Some(comp_val) = metadata.get("discord_components") else {
        return Vec::new();
    };
    let Some(rows_arr) = comp_val.as_array() else {
        return Vec::new();
    };

    rows_arr
        .iter()
        .filter_map(|row| {
            let buttons = row["buttons"].as_array()?;
            let btns: Vec<CreateButton> = buttons
                .iter()
                .filter_map(|b| {
                    let custom_id = b["custom_id"].as_str()?;
                    let label = b["label"].as_str().unwrap_or(custom_id);
                    let style = parse_button_style(b["style"].as_str().unwrap_or("secondary"));
                    let disabled = b["disabled"].as_bool().unwrap_or(false);
                    Some(
                        CreateButton::new(custom_id)
                            .label(label)
                            .style(style)
                            .disabled(disabled),
                    )
                })
                .collect();
            if btns.is_empty() {
                None
            } else {
                Some(CreateActionRow::Buttons(btns))
            }
        })
        .collect()
}

/// Send a followup message via Discord's webhook API for deferred interactions
async fn send_interaction_followup(
    http_client: &reqwest::Client,
    app_id: &str,
    token: &str,
    payload: &serde_json::Value,
) -> Result<()> {
    let url = format!("{}/webhooks/{}/{}", DISCORD_API_BASE, app_id, token);
    let resp = http_client.post(&url).json(payload).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Discord webhook followup failed ({}): {}", status, body);
    }
    Ok(())
}

/// Send a media file as a multipart followup
async fn send_interaction_media_followup(
    http_client: &reqwest::Client,
    app_id: &str,
    token: &str,
    file_path: &std::path::Path,
) -> Result<()> {
    let url = format!("{}/webhooks/{}/{}", DISCORD_API_BASE, app_id, token);
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let file_bytes = tokio::fs::read(file_path).await?;
    let part = reqwest::multipart::Part::bytes(file_bytes).file_name(file_name);
    let form = reqwest::multipart::Form::new().part("files[0]", part);

    let resp = http_client.post(&url).multipart(form).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        warn!(
            "Discord webhook media followup failed ({}): {}",
            status, body
        );
    }
    Ok(())
}

#[async_trait]
impl BaseChannel for DiscordChannel {
    fn name(&self) -> &'static str {
        "discord"
    }

    async fn start(&mut self) -> Result<()> {
        if self.config.token.is_empty() {
            return Err(anyhow::anyhow!("Discord token is empty"));
        }

        info!("Initializing Discord client...");
        *self.running.lock().await = true;

        let token = self.config.token.clone();
        let allow_from = self.config.allow_from.clone();
        let dm_policy = self.config.dm_policy.clone();
        let commands = self.config.commands.clone();
        let inbound_tx = self.inbound_tx.clone();
        let running = self.running.clone();

        let handle = tokio::spawn(async move {
            let mut reconnect_attempt = 0u32;
            loop {
                if !*running.lock().await {
                    info!("Discord channel stopped, exiting retry loop");
                    break;
                }

                let handler = Handler {
                    inbound_tx: inbound_tx.clone(),
                    allow_list: allow_from.clone(),
                    dm_policy: dm_policy.clone(),
                    http_client: reqwest::Client::builder()
                        .connect_timeout(std::time::Duration::from_secs(10))
                        .timeout(std::time::Duration::from_secs(30))
                        .build()
                        .unwrap_or_else(|_| reqwest::Client::new()),
                    commands: commands.clone(),
                };

                info!("Connecting to Discord gateway...");
                match Client::builder(
                    &token,
                    GatewayIntents::GUILD_MESSAGES
                        | GatewayIntents::DIRECT_MESSAGES
                        | GatewayIntents::MESSAGE_CONTENT,
                )
                .event_handler(handler)
                .await
                {
                    Ok(mut client) => match client.start().await {
                        Ok(()) => reconnect_attempt = 0,
                        Err(why) => error!("Discord client connection error: {:?}", why),
                    },
                    Err(e) => {
                        error!("Failed to create Discord client: {}", e);
                    }
                }

                // Check if we should reconnect
                if !*running.lock().await {
                    break;
                }

                let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                reconnect_attempt += 1;
                warn!(
                    "Discord client exited, reconnecting in {} seconds...",
                    delay
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
            }
        });

        self._client_handle = Some(handle);

        info!(
            "Discord channel started successfully - connection will be established in background"
        );
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        *self.running.lock().await = false;
        // Client will be dropped when handle completes
        if let Some(handle) = self._client_handle.take() {
            handle.abort();
        }
        Ok(())
    }

    async fn send_typing(&self, chat_id: &str) -> Result<()> {
        let channel_id = chat_id
            .parse::<u64>()
            .map_err(|e| anyhow::anyhow!("Invalid Discord channel_id: {}", e))?;
        let channel_id_typed = serenity::model::id::ChannelId::new(channel_id);
        channel_id_typed
            .broadcast_typing(&self.serenity_http)
            .await?;
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        if msg.channel != "discord" {
            return Ok(());
        }

        // Check for interaction followup path
        let interaction_token = msg
            .metadata
            .get("discord_interaction_token")
            .and_then(|v| v.as_str());
        let application_id = msg
            .metadata
            .get("discord_application_id")
            .and_then(|v| v.as_str());

        if let (Some(token), Some(app_id)) = (interaction_token, application_id) {
            return self.send_interaction_followup(msg, app_id, token).await;
        }

        // Regular channel message path
        let id_val = msg.chat_id.parse::<u64>()?;
        let chunks = split_message(&msg.content, 2000);
        let http = &self.serenity_http;

        // Check if chat_id is a user ID (from allow_from) — if so, open a DM channel
        let is_user_id = self
            .config
            .allow_from
            .iter()
            .any(|a| a.trim_start_matches('+') == msg.chat_id);

        let target_channel_id = if is_user_id {
            let user_id = serenity::model::id::UserId::new(id_val);
            let dm_channel = user_id.create_dm_channel(&http).await?;
            dm_channel.id
        } else {
            serenity::model::id::ChannelId::new(id_val)
        };

        // Send media attachments first
        for path in &msg.media {
            let file_path = std::path::Path::new(path);
            if !file_path.exists() {
                warn!("discord: media file not found: {}", path);
                continue;
            }
            match serenity::builder::CreateAttachment::path(file_path).await {
                Ok(attachment) => {
                    let builder = CreateMessage::new().add_file(attachment);
                    if let Err(e) = target_channel_id.send_message(&http, builder).await {
                        warn!("discord: failed to send attachment {}: {}", path, e);
                    }
                }
                Err(e) => {
                    warn!("discord: failed to read attachment {}: {}", path, e);
                }
            }
        }

        // Parse embeds and components from metadata
        let embeds = parse_embeds_from_metadata(&msg.metadata);
        let components = parse_components_from_metadata(&msg.metadata);

        // Send text content (attach embeds/components to the last chunk)
        let chunk_count = chunks.len();
        for (i, chunk) in chunks.iter().enumerate() {
            let is_last = i == chunk_count - 1;
            if is_last && (!embeds.is_empty() || !components.is_empty()) {
                let mut builder = CreateMessage::new().content(chunk);
                for embed in &embeds {
                    builder = builder.embed(embed.clone());
                }
                if !components.is_empty() {
                    builder = builder.components(components.clone());
                }
                target_channel_id
                    .send_message(&http, builder)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to send Discord message: {}", e))?;
            } else {
                target_channel_id
                    .say(&http, chunk)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to send Discord message: {}", e))?;
            }
        }

        // If there was no text but we have embeds/components, send them standalone
        if chunks.is_empty() && (!embeds.is_empty() || !components.is_empty()) {
            let mut builder = CreateMessage::new();
            for embed in &embeds {
                builder = builder.embed(embed.clone());
            }
            if !components.is_empty() {
                builder = builder.components(components);
            }
            target_channel_id
                .send_message(&http, builder)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to send Discord message: {}", e))?;
        }

        Ok(())
    }

    async fn send_and_get_id(&self, msg: &OutboundMessage) -> Result<Option<String>> {
        if msg.channel != "discord" {
            return Ok(None);
        }
        let id_val = msg.chat_id.parse::<u64>()?;
        let target = serenity::model::id::ChannelId::new(id_val);
        let chunks = split_message(&msg.content, 2000);
        let mut last_id = None;
        for chunk in &chunks {
            let sent = target
                .say(&self.serenity_http, chunk)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to send Discord message: {}", e))?;
            last_id = Some(sent.id.to_string());
        }
        Ok(last_id)
    }

    async fn edit_message(&self, chat_id: &str, message_id: &str, content: &str) -> Result<()> {
        let channel_id = chat_id.parse::<u64>()?;
        let msg_id = message_id.parse::<u64>()?;
        let channel = serenity::model::id::ChannelId::new(channel_id);
        let builder = serenity::builder::EditMessage::new().content(content);
        channel
            .edit_message(
                &self.serenity_http,
                serenity::model::id::MessageId::new(msg_id),
                builder,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to edit Discord message: {}", e))?;
        Ok(())
    }

    async fn delete_message(&self, chat_id: &str, message_id: &str) -> Result<()> {
        let channel_id = chat_id.parse::<u64>()?;
        let msg_id = message_id.parse::<u64>()?;
        let channel = serenity::model::id::ChannelId::new(channel_id);
        channel
            .delete_message(
                &self.serenity_http,
                serenity::model::id::MessageId::new(msg_id),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete Discord message: {}", e))?;
        Ok(())
    }
}

impl DiscordChannel {
    async fn send_interaction_followup(
        &self,
        msg: &OutboundMessage,
        app_id: &str,
        token: &str,
    ) -> Result<()> {
        let chunks = split_message(&msg.content, 2000);
        let embeds = parse_embeds_from_metadata(&msg.metadata);
        let components = parse_components_from_metadata(&msg.metadata);

        // Send media as separate followups
        for path in &msg.media {
            let file_path = std::path::Path::new(path);
            if !file_path.exists() {
                warn!("discord: media file not found: {}", path);
                continue;
            }
            if let Err(e) =
                send_interaction_media_followup(&self.http_client, app_id, token, file_path).await
            {
                warn!("discord: failed to send interaction media {}: {}", path, e);
            }
        }

        // Send text chunks
        let chunk_count = chunks.len();
        for (i, chunk) in chunks.iter().enumerate() {
            let is_last = i == chunk_count - 1;
            let mut payload = serde_json::json!({ "content": chunk });

            // Attach embeds/components to the last chunk
            if is_last {
                if !embeds.is_empty() {
                    let embed_json: Vec<serde_json::Value> = embeds
                        .iter()
                        .map(|_| {
                            // Build embed JSON from metadata directly
                            serde_json::json!({})
                        })
                        .collect();
                    // Use the raw metadata embeds directly for webhook
                    if let Some(raw_embeds) = msg.metadata.get("discord_embeds") {
                        payload["embeds"] = raw_embeds.clone();
                    } else if !embed_json.is_empty() {
                        payload["embeds"] = serde_json::json!(embed_json);
                    }
                }
                if !components.is_empty()
                    && let Some(raw_components) = msg.metadata.get("discord_components")
                {
                    // Convert to Discord API component format
                    let rows: Vec<serde_json::Value> = raw_components
                        .as_array()
                        .unwrap_or(&Vec::new())
                        .iter()
                        .map(|row| {
                            let buttons = row["buttons"]
                                .as_array()
                                .unwrap_or(&Vec::new())
                                .iter()
                                .filter_map(|b| {
                                    let custom_id = b["custom_id"].as_str()?;
                                    let label = b["label"].as_str().unwrap_or(custom_id);
                                    let style = match b["style"].as_str().unwrap_or("secondary") {
                                        "primary" => 1,
                                        "success" => 3,
                                        "danger" => 4,
                                        _ => 2,
                                    };
                                    let disabled = b["disabled"].as_bool().unwrap_or(false);
                                    Some(serde_json::json!({
                                        "type": 2,
                                        "custom_id": custom_id,
                                        "label": label,
                                        "style": style,
                                        "disabled": disabled
                                    }))
                                })
                                .collect::<Vec<_>>();
                            serde_json::json!({
                                "type": 1,
                                "components": buttons
                            })
                        })
                        .collect();
                    payload["components"] = serde_json::json!(rows);
                }
            }

            if let Err(e) =
                send_interaction_followup(&self.http_client, app_id, token, &payload).await
            {
                error!("Failed to send Discord interaction followup: {}", e);
                return Err(e);
            }
        }

        // If no text but have embeds/components
        if chunks.is_empty() && (!embeds.is_empty() || !components.is_empty()) {
            let mut payload = serde_json::json!({});
            if let Some(raw_embeds) = msg.metadata.get("discord_embeds") {
                payload["embeds"] = raw_embeds.clone();
            }
            if let Some(raw_components) = msg.metadata.get("discord_components") {
                let rows: Vec<serde_json::Value> = raw_components
                    .as_array()
                    .unwrap_or(&Vec::new())
                    .iter()
                    .map(|row| {
                        let buttons = row["buttons"]
                            .as_array()
                            .unwrap_or(&Vec::new())
                            .iter()
                            .filter_map(|b| {
                                let custom_id = b["custom_id"].as_str()?;
                                let label = b["label"].as_str().unwrap_or(custom_id);
                                let style = match b["style"].as_str().unwrap_or("secondary") {
                                    "primary" => 1,
                                    "success" => 3,
                                    "danger" => 4,
                                    _ => 2,
                                };
                                let disabled = b["disabled"].as_bool().unwrap_or(false);
                                Some(serde_json::json!({
                                    "type": 2,
                                    "custom_id": custom_id,
                                    "label": label,
                                    "style": style,
                                    "disabled": disabled
                                }))
                            })
                            .collect::<Vec<_>>();
                        serde_json::json!({
                            "type": 1,
                            "components": buttons
                        })
                    })
                    .collect();
                payload["components"] = serde_json::json!(rows);
            }
            send_interaction_followup(&self.http_client, app_id, token, &payload).await?;
        }

        debug!("Sent Discord interaction followup successfully");
        Ok(())
    }
}
