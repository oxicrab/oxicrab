//! Predicate that decides whether a `WhatsApp` group message addresses
//! the bot. The channel handler uses this to gate group dispatch:
//! groups are shared spaces, so the bot only participates when
//! explicitly invited.
//!
//! A message addresses the bot if EITHER:
//!
//! 1. The bot's JID appears in `context_info.mentioned_jid` of any
//!    message variant on the base message (the standard `@mention`).
//! 2. `context_info.participant` equals the bot's JID, meaning the
//!    message is a quote-reply to one of the bot's prior messages
//!    (treated as an implicit mention).
//!
//! JIDs are compared post-normalization (strip device suffix, ensure
//! the `@s.whatsapp.net` domain) because mentions in `mentioned_jid`
//! sometimes include and sometimes omit the domain depending on the
//! sending client.

use whatsapp_rust::waproto::whatsapp as wa;

/// Strip a JID's device suffix (`:NN`) and ensure a domain.
/// Mirrors `super::normalize_jid` but stays inside this module so the
/// mention predicate can be unit-tested without pulling in the full
/// channel struct.
fn normalize(jid: &str) -> String {
    if jid.contains('@') {
        let (user, domain) = jid.split_once('@').unwrap_or((jid, "s.whatsapp.net"));
        let user = user.split(':').next().unwrap_or(user);
        format!("{user}@{domain}")
    } else {
        let user = jid.split(':').next().unwrap_or(jid);
        format!("{user}@s.whatsapp.net")
    }
}

/// Returns true if `ctx` mentions or is a quote-reply to the bot.
fn context_addresses_bot(ctx: &wa::ContextInfo, bot_jid: &str) -> bool {
    if ctx.mentioned_jid.iter().any(|j| normalize(j) == bot_jid) {
        return true;
    }
    if let Some(participant) = ctx.participant.as_ref()
        && normalize(participant) == bot_jid
    {
        return true;
    }
    false
}

/// Returns true if any of the message variants the bot routinely
/// receives carry a `context_info` that addresses `bot_jid`.
///
/// `bot_jid` MUST be pre-normalized via `normalize`. The caller (the
/// channel handler) already normalizes when caching the JID, so this
/// keeps the hot path allocation-free.
pub fn message_mentions_bot(base: &wa::Message, bot_jid: &str) -> bool {
    if let Some(ext) = &base.extended_text_message
        && let Some(ctx) = &ext.context_info
        && context_addresses_bot(ctx, bot_jid)
    {
        return true;
    }
    if let Some(img) = &base.image_message
        && let Some(ctx) = &img.context_info
        && context_addresses_bot(ctx, bot_jid)
    {
        return true;
    }
    if let Some(vid) = &base.video_message
        && let Some(ctx) = &vid.context_info
        && context_addresses_bot(ctx, bot_jid)
    {
        return true;
    }
    if let Some(audio) = &base.audio_message
        && let Some(ctx) = &audio.context_info
        && context_addresses_bot(ctx, bot_jid)
    {
        return true;
    }
    if let Some(doc) = &base.document_message
        && let Some(ctx) = &doc.context_info
        && context_addresses_bot(ctx, bot_jid)
    {
        return true;
    }
    false
}

/// Public wrapper around `normalize` for the channel handler so it
/// produces the canonical comparison form when caching the bot JID.
pub fn normalize_bot_jid(jid: &str) -> String {
    normalize(jid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatsapp_rust::waproto::whatsapp::message::ExtendedTextMessage;

    const BOT_JID: &str = "15551234567@s.whatsapp.net";
    const OTHER_JID: &str = "15559999999@s.whatsapp.net";

    fn text_msg_with_ctx(ctx: wa::ContextInfo) -> wa::Message {
        wa::Message {
            extended_text_message: Some(Box::new(ExtendedTextMessage {
                text: Some("hello bot".into()),
                context_info: Some(Box::new(ctx)),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    #[test]
    fn plain_text_without_mention_is_not_addressed() {
        let msg = wa::Message {
            conversation: Some("just chatting in the group".into()),
            ..Default::default()
        };
        assert!(!message_mentions_bot(&msg, BOT_JID));
    }

    #[test]
    fn extended_text_without_context_is_not_addressed() {
        let msg = wa::Message {
            extended_text_message: Some(Box::new(ExtendedTextMessage {
                text: Some("not a mention".into()),
                context_info: None,
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(!message_mentions_bot(&msg, BOT_JID));
    }

    #[test]
    fn mention_in_extended_text_addresses_bot() {
        let msg = text_msg_with_ctx(wa::ContextInfo {
            mentioned_jid: vec![BOT_JID.into()],
            ..Default::default()
        });
        assert!(message_mentions_bot(&msg, BOT_JID));
    }

    #[test]
    fn mention_with_device_suffix_is_normalized() {
        let msg = text_msg_with_ctx(wa::ContextInfo {
            mentioned_jid: vec!["15551234567:20@s.whatsapp.net".into()],
            ..Default::default()
        });
        assert!(message_mentions_bot(&msg, BOT_JID));
    }

    #[test]
    fn mention_of_someone_else_does_not_address_bot() {
        let msg = text_msg_with_ctx(wa::ContextInfo {
            mentioned_jid: vec![OTHER_JID.into()],
            ..Default::default()
        });
        assert!(!message_mentions_bot(&msg, BOT_JID));
    }

    #[test]
    fn quote_reply_to_bot_addresses_bot() {
        let msg = text_msg_with_ctx(wa::ContextInfo {
            participant: Some(BOT_JID.into()),
            stanza_id: Some("orig-msg-id".into()),
            ..Default::default()
        });
        assert!(message_mentions_bot(&msg, BOT_JID));
    }

    #[test]
    fn quote_reply_to_someone_else_does_not_address_bot() {
        let msg = text_msg_with_ctx(wa::ContextInfo {
            participant: Some(OTHER_JID.into()),
            stanza_id: Some("orig-msg-id".into()),
            ..Default::default()
        });
        assert!(!message_mentions_bot(&msg, BOT_JID));
    }

    #[test]
    fn image_caption_mention_addresses_bot() {
        let msg = wa::Message {
            image_message: Some(Box::new(wa::message::ImageMessage {
                caption: Some("look at this @bot".into()),
                context_info: Some(Box::new(wa::ContextInfo {
                    mentioned_jid: vec![BOT_JID.into()],
                    ..Default::default()
                })),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(message_mentions_bot(&msg, BOT_JID));
    }

    #[test]
    fn normalize_strips_device_suffix() {
        assert_eq!(normalize("15551234567:20@s.whatsapp.net"), BOT_JID);
        assert_eq!(normalize("15551234567@s.whatsapp.net"), BOT_JID);
        assert_eq!(normalize("15551234567"), BOT_JID);
    }
}
