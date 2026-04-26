use serenity::builder::{CreateActionRow, CreateButton, CreateEmbed, CreateEmbedFooter};
use serenity::model::application::ButtonStyle;
use std::collections::HashMap;

pub(super) fn parse_embeds_from_metadata(
    metadata: &HashMap<String, serde_json::Value>,
) -> Vec<CreateEmbed> {
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
                    let inline = f["inline"].as_bool().unwrap_or_default();
                    embed = embed.field(name, value, inline);
                }
            }
            embed
        })
        .collect()
}

pub(super) fn parse_button_style(style: &str) -> ButtonStyle {
    match style {
        "primary" => ButtonStyle::Primary,
        "success" => ButtonStyle::Success,
        "danger" => ButtonStyle::Danger,
        _ => ButtonStyle::Secondary,
    }
}

pub(super) fn parse_components_from_metadata(
    metadata: &HashMap<String, serde_json::Value>,
    dispatch_store: Option<&crate::dispatch::DispatchContextStore>,
) -> Vec<CreateActionRow> {
    parse_unified_buttons(metadata, dispatch_store)
}

/// Convert unified `metadata["buttons"]` to Discord action rows.
/// Format: `[{"id": "yes", "label": "Yes", "style": "primary"}, ...]`
///
/// If `dispatch_store` is provided, any button whose `context` field parses as an
/// `ActionDispatchPayload` is stored so the payload can be retrieved on click.
pub(super) fn parse_unified_buttons(
    metadata: &HashMap<String, serde_json::Value>,
    dispatch_store: Option<&crate::dispatch::DispatchContextStore>,
) -> Vec<CreateActionRow> {
    let Some(buttons_val) = metadata.get(oxicrab_core::bus::events::meta::BUTTONS) else {
        return Vec::new();
    };
    parse_unified_buttons_value(buttons_val, dispatch_store)
}

/// Variant that takes the raw button value directly (as the agent
/// loop passes it via `StreamEvent::End.buttons`). Used by the
/// Discord stream consumer where there is no enclosing
/// `OutboundMessage`.
pub(crate) fn parse_unified_buttons_value(
    buttons_val: &serde_json::Value,
    dispatch_store: Option<&crate::dispatch::DispatchContextStore>,
) -> Vec<CreateActionRow> {
    let Some(buttons_arr) = buttons_val.as_array() else {
        return Vec::new();
    };

    let btns: Vec<CreateButton> = buttons_arr
        .iter()
        .filter_map(|b| {
            let id = b["id"].as_str()?;
            let label = b["label"].as_str().unwrap_or(id);
            let style = parse_button_style(b["style"].as_str().unwrap_or("secondary"));
            if let Some(store) = dispatch_store
                && let Some(ctx_str) = b["context"].as_str()
                && let Ok(payload) =
                    serde_json::from_str::<crate::dispatch::ActionDispatchPayload>(ctx_str)
            {
                store.insert(id.to_string(), payload);
            }
            Some(CreateButton::new(id).label(label).style(style))
        })
        .collect();

    if btns.is_empty() {
        Vec::new()
    } else {
        vec![CreateActionRow::Buttons(btns)]
    }
}

/// Convert metadata to Discord API JSON for interaction followups.
/// NOTE: Relies on `parse_components_from_metadata()` having been called first
/// with a `dispatch_store` to register button dispatch contexts.
pub(super) fn components_to_api_json(
    metadata: &HashMap<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    let buttons_val = metadata.get(oxicrab_core::bus::events::meta::BUTTONS)?;
    let buttons_arr = buttons_val.as_array()?;
    if buttons_arr.is_empty() {
        return None;
    }
    let btns: Vec<serde_json::Value> = buttons_arr
        .iter()
        .filter_map(|b| {
            let id = b["id"].as_str()?;
            let label = b["label"].as_str().unwrap_or(id);
            let style = match b["style"].as_str().unwrap_or("secondary") {
                "primary" => 1,
                "success" => 3,
                "danger" => 4,
                _ => 2,
            };
            Some(serde_json::json!({
                "type": 2,
                "custom_id": id,
                "label": label,
                "style": style
            }))
        })
        .collect();
    if btns.is_empty() {
        return None;
    }
    Some(serde_json::json!([{
        "type": 1,
        "components": btns
    }]))
}
