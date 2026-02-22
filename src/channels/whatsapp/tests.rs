use super::*;

fn allow_list(nums: &[&str]) -> Vec<String> {
    nums.iter().map(ToString::to_string).collect()
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
    let allow: Vec<String> = vec![];
    assert!(should_skip_own_message(
        Some("19876543210@s.whatsapp.net"),
        &allow,
    ));
}

#[test]
fn test_no_skip_when_allow_from_wildcard() {
    // Wildcard "*" means allow everyone — process
    let allow = vec!["*".to_string()];
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
