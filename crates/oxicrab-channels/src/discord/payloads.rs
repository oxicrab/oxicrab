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
    // Prefer discord_components (legacy, backward-compatible).
    if let Some(comp_val) = metadata.get("discord_components")
        && let Some(rows_arr) = comp_val.as_array()
    {
        let rows: Vec<CreateActionRow> = rows_arr
            .iter()
            .filter_map(|row| {
                let buttons = row["buttons"].as_array()?;
                let btns: Vec<CreateButton> = buttons
                    .iter()
                    .filter_map(|b| {
                        let custom_id = b["custom_id"].as_str()?;
                        let label = b["label"].as_str().unwrap_or(custom_id);
                        let style = parse_button_style(b["style"].as_str().unwrap_or("secondary"));
                        let disabled = b["disabled"].as_bool().unwrap_or_default();
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
            .collect();
        if !rows.is_empty() {
            return rows;
        }
    }

    // Fallback: unified "buttons" format.
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
///
/// Checks `discord_components` first, then falls back to unified `buttons` key.
pub(super) fn components_to_api_json(
    metadata: &HashMap<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    // Try discord_components first (legacy format).
    if let Some(raw_components) = metadata.get("discord_components")
        && let Some(rows_arr) = raw_components.as_array()
        && !rows_arr.is_empty()
    {
        let rows: Vec<serde_json::Value> = rows_arr
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
                        let disabled = b["disabled"].as_bool().unwrap_or_default();
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
        return Some(serde_json::json!(rows));
    }

    // Fallback: unified "buttons" format.
    if let Some(buttons_val) = metadata.get(oxicrab_core::bus::events::meta::BUTTONS)
        && let Some(buttons_arr) = buttons_val.as_array()
        && !buttons_arr.is_empty()
    {
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
        if !btns.is_empty() {
            return Some(serde_json::json!([{
                "type": 1,
                "components": btns
            }]));
        }
    }

    None
}
