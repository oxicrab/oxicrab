use super::*;
use oxicrab_core::config::schema::DenyByDefaultList;

fn allow_list(nums: &[&str]) -> DenyByDefaultList {
    DenyByDefaultList::new(nums.iter().map(ToString::to_string).collect())
}

// --- normalize_jid tests ---

#[test]
fn test_normalize_jid_strips_device_suffix() {
    assert_eq!(
        normalize_jid("15037348571:20@s.whatsapp.net"),
        "15037348571@s.whatsapp.net"
    );
}

#[test]
fn test_normalize_jid_preserves_clean_jid() {
    assert_eq!(
        normalize_jid("15037348571@s.whatsapp.net"),
        "15037348571@s.whatsapp.net"
    );
}

#[test]
fn test_normalize_jid_adds_default_domain() {
    assert_eq!(normalize_jid("15037348571"), "15037348571@s.whatsapp.net");
}

#[test]
fn test_normalize_jid_bare_number_with_device() {
    assert_eq!(
        normalize_jid("15037348571:20"),
        "15037348571@s.whatsapp.net"
    );
}

#[test]
fn test_normalize_jid_group_jid() {
    assert_eq!(
        normalize_jid("120363123456789@g.us"),
        "120363123456789@g.us"
    );
}

#[test]
fn test_normalize_jid_lid() {
    assert_eq!(normalize_jid("194506284601577@lid"), "194506284601577@lid");
}

#[test]
fn test_normalize_jid_lid_with_device() {
    assert_eq!(
        normalize_jid("194506284601577:33@lid"),
        "194506284601577@lid"
    );
}

// --- should_skip_own_message tests ---

#[test]
fn test_skip_own_msg_to_other_person() {
    // User sent a message to someone NOT in allowFrom — skip
    let allow = allow_list(&["15037348571"]);
    assert!(should_skip_own_message(
        Some("19876543210@s.whatsapp.net"),
        &allow,
    ));
}

#[test]
fn test_no_skip_self_chat() {
    // User sent a message to themselves (self-chat) — process
    let allow = allow_list(&["15037348571"]);
    assert!(!should_skip_own_message(
        Some("15037348571@s.whatsapp.net"),
        &allow,
    ));
}

#[test]
fn test_no_skip_self_chat_with_device_id() {
    // Recipient JID has device suffix like "15037348571:20@s.whatsapp.net"
    let allow = allow_list(&["15037348571"]);
    assert!(!should_skip_own_message(
        Some("15037348571:20@s.whatsapp.net"),
        &allow,
    ));
}

#[test]
fn test_no_skip_when_no_recipient() {
    // No recipient field at all — process the message
    let allow = allow_list(&["15037348571"]);
    assert!(!should_skip_own_message(None, &allow));
}

#[test]
fn test_skip_when_allow_from_empty() {
    // Empty allowFrom now means deny-all (default-deny)
    let allow = DenyByDefaultList::default();
    assert!(should_skip_own_message(
        Some("19876543210@s.whatsapp.net"),
        &allow,
    ));
}

#[test]
fn test_no_skip_when_allow_from_wildcard() {
    // Wildcard "*" means allow everyone — process
    let allow = allow_list(&["*"]);
    assert!(!should_skip_own_message(
        Some("19876543210@s.whatsapp.net"),
        &allow,
    ));
}

#[test]
fn test_skip_own_msg_lid_recipient() {
    // Recipient uses LID format instead of phone number
    let allow = allow_list(&["15037348571"]);
    assert!(should_skip_own_message(Some("194506284601577@lid"), &allow,));
}

#[test]
fn test_no_skip_multiple_allow_from() {
    // Multiple numbers in allowFrom, recipient matches one
    let allow = allow_list(&["15037348571", "15551234567"]);
    assert!(!should_skip_own_message(
        Some("15551234567@s.whatsapp.net"),
        &allow,
    ));
}

#[test]
fn test_skip_other_with_multiple_allow_from() {
    // Multiple numbers in allowFrom, recipient matches none
    let allow = allow_list(&["15037348571", "15551234567"]);
    assert!(should_skip_own_message(
        Some("19999999999@s.whatsapp.net"),
        &allow,
    ));
}

// --- is_image_mime tests ---

#[test]
fn test_is_image_mime_jpeg() {
    assert!(is_image_mime(Some("image/jpeg")));
}

#[test]
fn test_is_image_mime_png() {
    assert!(is_image_mime(Some("image/png")));
}

#[test]
fn test_is_image_mime_webp() {
    assert!(is_image_mime(Some("image/webp")));
}

#[test]
fn test_is_image_mime_not_image() {
    assert!(!is_image_mime(Some("application/pdf")));
    assert!(!is_image_mime(Some("audio/ogg")));
    assert!(!is_image_mime(Some("text/plain")));
}

#[test]
fn test_is_image_mime_none() {
    assert!(!is_image_mime(None));
}

#[test]
fn test_is_image_mime_empty() {
    assert!(!is_image_mime(Some("")));
}
