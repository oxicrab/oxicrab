use super::cli_types::ChannelCommands;
use crate::config::load_config;
use anyhow::Result;

// Variables/async used conditionally inside #[cfg(feature)] blocks
#[allow(clippy::too_many_lines, unused_variables, clippy::unused_async)]
pub(super) async fn channels_command(cmd: ChannelCommands) -> Result<()> {
    match cmd {
        ChannelCommands::Status => {
            let config = load_config(None)?;

            println!("Channel Status");
            println!(
                "\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}"
            );

            // WhatsApp
            #[cfg(feature = "channel-whatsapp")]
            {
                let wa = &config.channels.whatsapp;
                println!(
                    "WhatsApp: {}",
                    if wa.enabled {
                        "\u{2713} enabled"
                    } else {
                        "\u{2717} disabled"
                    }
                );
                if wa.enabled {
                    let session_path = crate::utils::get_oxicrab_home().map_or_else(
                        |_| std::path::PathBuf::from(".oxicrab/whatsapp/whatsapp.db"),
                        |h| h.join("whatsapp").join("whatsapp.db"),
                    );
                    let session_exists = session_path.exists();
                    println!(
                        "  Session: {} ({})",
                        session_path.display(),
                        if session_exists {
                            "exists"
                        } else {
                            "not paired - run 'oxicrab channels login'"
                        }
                    );
                }
            }
            #[cfg(not(feature = "channel-whatsapp"))]
            println!("WhatsApp: not compiled (enable 'channel-whatsapp' feature)");

            // Discord
            #[cfg(feature = "channel-discord")]
            {
                let dc = &config.channels.discord;
                println!(
                    "Discord: {}",
                    if dc.enabled {
                        "\u{2713} enabled"
                    } else {
                        "\u{2717} disabled"
                    }
                );
                if dc.enabled {
                    println!(
                        "  Token: {}",
                        if dc.token.is_empty() {
                            "not set"
                        } else {
                            "configured"
                        }
                    );
                }
            }
            #[cfg(not(feature = "channel-discord"))]
            println!("Discord: not compiled (enable 'channel-discord' feature)");

            // Telegram
            #[cfg(feature = "channel-telegram")]
            {
                let tg = &config.channels.telegram;
                println!(
                    "Telegram: {}",
                    if tg.enabled {
                        "\u{2713} enabled"
                    } else {
                        "\u{2717} disabled"
                    }
                );
                if tg.enabled {
                    println!(
                        "  Token: {}",
                        if tg.token.is_empty() {
                            "not set"
                        } else {
                            "configured"
                        }
                    );
                }
            }
            #[cfg(not(feature = "channel-telegram"))]
            println!("Telegram: not compiled (enable 'channel-telegram' feature)");

            // Slack
            #[cfg(feature = "channel-slack")]
            {
                let sl = &config.channels.slack;
                println!(
                    "Slack: {}",
                    if sl.enabled {
                        "\u{2713} enabled"
                    } else {
                        "\u{2717} disabled"
                    }
                );
                if sl.enabled {
                    println!(
                        "  Bot Token: {}",
                        if sl.bot_token.is_empty() {
                            "not set"
                        } else {
                            "configured"
                        }
                    );
                }
            }
            #[cfg(not(feature = "channel-slack"))]
            println!("Slack: not compiled (enable 'channel-slack' feature)");
        }
        ChannelCommands::Login => {
            #[cfg(feature = "channel-whatsapp")]
            whatsapp_login().await?;
            #[cfg(not(feature = "channel-whatsapp"))]
            anyhow::bail!("WhatsApp support not compiled (enable 'channel-whatsapp' feature)");
        }
    }
    Ok(())
}

#[cfg(feature = "channel-whatsapp")]
async fn whatsapp_login() -> Result<()> {
    use crate::utils::get_oxicrab_home;
    use std::sync::Arc;
    use wa_rs::bot::Bot;
    use wa_rs::store::SqliteStore;
    use wa_rs::types::events::Event;
    use wa_rs_tokio_transport::TokioWebSocketTransportFactory;
    use wa_rs_ureq_http::UreqHttpClient;

    println!("\u{1f916} Starting WhatsApp authentication...");
    println!("Scan the QR code that appears below to connect.\n");

    // Determine session path
    let session_path = get_oxicrab_home()?.join("whatsapp");
    std::fs::create_dir_all(&session_path)?;

    let session_db = session_path.join("whatsapp.db");
    let session_db_str = session_db.to_string_lossy().to_string();

    // Create backend
    let backend = Arc::new(SqliteStore::new(&session_db_str).await?);

    // Create transport and HTTP client
    let transport_factory = TokioWebSocketTransportFactory::new();
    let http_client = UreqHttpClient::new();

    // Build bot with QR code display
    let bot = Bot::builder()
        .with_backend(backend)
        .with_transport_factory(transport_factory)
        .with_http_client(http_client)
        .on_event(|event, _client| async move {
            match event {
                Event::PairingQrCode { code, .. } => {
                    println!("\n\u{1f916} WhatsApp QR Code:");
                    // Render QR code in terminal (compact)
                    match qrcode::QrCode::new(&code) {
                        Ok(qr) => {
                            let string = qr
                                .render::<char>()
                                .quiet_zone(false)
                                .module_dimensions(1, 1)
                                .build();
                            println!("{string}");
                        }
                        Err(e) => {
                            eprintln!("Failed to generate QR code: {e}. Raw code: {code}");
                            println!("Raw QR code data: {code}");
                        }
                    }
                }
                Event::PairingCode { code, timeout } => {
                    println!("\n\u{1f916} WhatsApp Pairing Code: {code}");
                    println!("Enter this code on your phone.");
                    println!("Code expires in: {timeout:?}\n");
                }
                Event::PairSuccess(_pair_success) => {
                    println!("\n\u{2705} WhatsApp connected successfully!");
                    println!("You can now close this window. The session is saved.\n");
                }
                Event::PairError(pair_error) => {
                    eprintln!("\n\u{274c} WhatsApp pairing failed: {pair_error:?}");
                }
                Event::Connected(_connected) => {
                    println!("\n\u{2705} WhatsApp connected!\n");
                }
                Event::Disconnected(_disconnected) => {
                    eprintln!("\n\u{26a0}\u{fe0f}  WhatsApp disconnected");
                }
                _ => {}
            }
        })
        .build()
        .await?;

    println!("Waiting for QR code...\n");

    // Run bot - this will display QR code and wait for pairing
    let mut bot_mut = bot;
    match bot_mut.run().await {
        Ok(handle) => {
            // Wait for pairing to complete or user interruption
            tokio::select! {
                _ = handle => {
                    println!("\nBot stopped.");
                }
                _ = tokio::signal::ctrl_c() => {
                    println!("\n\nInterrupted. Session saved - you can reconnect later.");
                }
            }
        }
        Err(e) => {
            anyhow::bail!("Failed to start WhatsApp bot: {e}");
        }
    }

    Ok(())
}
