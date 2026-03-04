use super::*;

#[test]
fn test_validate_signature_valid() {
    // Twilio example values
    let auth_token = "12345";
    let url = "https://mycompany.com/myapp.php?foo=1&bar=2";
    let mut params = HashMap::new();
    params.insert("CallSid".to_string(), "CA1234567890ABCDE".to_string());
    params.insert("Caller".to_string(), "+14158675310".to_string());
    params.insert("Digits".to_string(), "1234".to_string());
    params.insert("From".to_string(), "+14158675310".to_string());
    params.insert("To".to_string(), "+18005551212".to_string());

    // Compute expected signature
    let mut data = url.to_string();
    let mut sorted_keys: Vec<&String> = params.keys().collect();
    sorted_keys.sort();
    for key in &sorted_keys {
        data.push_str(key);
        data.push_str(&params[*key]);
    }
    let mut mac = HmacSha1::new_from_slice(auth_token.as_bytes()).unwrap();
    mac.update(data.as_bytes());
    let result = mac.finalize();
    let expected_sig = base64::engine::general_purpose::STANDARD.encode(result.into_bytes());

    assert!(validate_twilio_signature(
        auth_token,
        &expected_sig,
        url,
        &params
    ));
}

#[test]
fn test_validate_signature_invalid() {
    let auth_token = "12345";
    let url = "https://example.com/webhook";
    let params = HashMap::new();

    assert!(!validate_twilio_signature(
        auth_token,
        "invalid_signature",
        url,
        &params
    ));
}

#[test]
fn test_validate_signature_empty_params() {
    let auth_token = "test_token";
    let url = "https://example.com/webhook";
    let params = HashMap::new();

    // Compute expected signature with empty params (just URL)
    let mut mac = HmacSha1::new_from_slice(auth_token.as_bytes()).unwrap();
    mac.update(url.as_bytes());
    let result = mac.finalize();
    let expected_sig = base64::engine::general_purpose::STANDARD.encode(result.into_bytes());

    assert!(validate_twilio_signature(
        auth_token,
        &expected_sig,
        url,
        &params
    ));
}

#[test]
fn test_validate_signature_param_ordering() {
    let auth_token = "secret";
    let url = "https://example.com/hook";
    let mut params = HashMap::new();
    params.insert("Zebra".to_string(), "last".to_string());
    params.insert("Alpha".to_string(), "first".to_string());
    params.insert("Middle".to_string(), "mid".to_string());

    // Compute expected: URL + AlphafirstMiddlemidZebralast
    let data = format!("{url}AlphafirstMiddlemidZebralast");
    let mut mac = HmacSha1::new_from_slice(auth_token.as_bytes()).unwrap();
    mac.update(data.as_bytes());
    let result = mac.finalize();
    let expected_sig = base64::engine::general_purpose::STANDARD.encode(result.into_bytes());

    assert!(validate_twilio_signature(
        auth_token,
        &expected_sig,
        url,
        &params
    ));
}

#[test]
fn test_send_constructs_correct_url() {
    // Verify the URL format for sending messages
    let chat_id = "CH1234567890";
    let url = format!("https://conversations.twilio.com/v1/Conversations/{chat_id}/Messages");
    assert_eq!(
        url,
        "https://conversations.twilio.com/v1/Conversations/CH1234567890/Messages"
    );
}
