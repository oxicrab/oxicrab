use super::*;
use axum::extract::ConnectInfo;
use ipnet::IpNet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

fn make_state() -> HttpApiState {
    HttpApiState {
        inbound_tx: Arc::new(mpsc::channel(1).0),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(HashMap::new()),
        outbound_tx: None,
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    }
}

#[tokio::test]
async fn test_health_endpoint_returns_json() {
    use axum::http::Request;
    use tower::ServiceExt;

    let state = make_state();
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("GET")
        .uri("/api/health")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ready");
    assert_eq!(json["version"], VERSION);
}

#[tokio::test]
async fn test_health_endpoint_starting_before_ready() {
    use axum::http::Request;
    use tower::ServiceExt;

    let mut state = make_state();
    state.ready = Arc::new(AtomicBool::new(false));
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("GET")
        .uri("/api/health")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "starting");
}

#[test]
fn test_route_response_non_http_returns_false() {
    let state = make_state();
    let msg = OutboundMessage::builder("telegram", "123", "hello").build();
    assert!(!route_response(&state, msg));
}

#[test]
fn test_route_response_http_with_pending() {
    let state = make_state();
    let (tx, mut rx) = oneshot::channel();
    state
        .pending
        .lock()
        .unwrap()
        .insert("req-1".to_string(), tx);

    let msg = OutboundMessage::builder("http", "req-1", "response text").build();
    assert!(route_response(&state, msg));
    let received = rx.try_recv().unwrap();
    assert_eq!(received.content, "response text");
}

#[test]
fn test_route_response_http_no_pending() {
    let state = make_state();
    let msg = OutboundMessage::builder("http", "nonexistent", "orphan").build();
    // Should not panic, just return true (consumed) and warn
    assert!(route_response(&state, msg));
}

#[test]
fn test_validate_webhook_signature_valid() {
    let secret = "test-secret";
    let body = b"hello world";
    // Compute expected signature
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let sig = hex::encode(mac.finalize().into_bytes());
    assert!(validate_webhook_signature(secret, &sig, body));
}

#[test]
fn test_validate_webhook_signature_with_prefix() {
    let secret = "test-secret";
    let body = b"hello world";
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    assert!(validate_webhook_signature(secret, &sig, body));
}

#[test]
fn test_validate_webhook_signature_uppercase_hex() {
    let secret = "test-secret";
    let body = b"hello world";
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let sig = hex::encode(mac.finalize().into_bytes()).to_ascii_uppercase();
    assert!(validate_webhook_signature(secret, &sig, body));
}

#[test]
fn test_validate_webhook_signature_invalid() {
    assert!(!validate_webhook_signature(
        "secret",
        "bad-signature",
        b"body"
    ));
}

#[test]
fn test_apply_template_body_only() {
    let result = apply_template("Event: {{body}}", "something happened", None);
    assert_eq!(result, "Event: something happened");
}

#[test]
fn test_apply_template_json_keys() {
    let json: serde_json::Value =
        serde_json::json!({"repo": "oxicrab", "action": "push", "count": 3});
    let result = apply_template(
        "{{action}} to {{repo}} ({{count}} commits)",
        "",
        Some(&json),
    );
    assert_eq!(result, "push to oxicrab (3 commits)");
}

#[test]
fn test_apply_template_missing_key_preserved() {
    let json: serde_json::Value = serde_json::json!({"name": "test"});
    let result = apply_template("{{name}} {{missing}}", "", Some(&json));
    assert_eq!(result, "test {{missing}}");
}

fn make_webhook_config(enabled: bool, secret: &str) -> WebhookConfig {
    WebhookConfig {
        enabled,
        secret: secret.to_string(),
        targets: vec![WebhookTarget {
            channel: "slack".to_string(),
            chat_id: "C123".to_string(),
        }],
        ..Default::default()
    }
}

fn make_state_with_webhooks(webhooks: HashMap<String, WebhookConfig>) -> HttpApiState {
    HttpApiState {
        inbound_tx: Arc::new(mpsc::channel(1).0),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(webhooks),
        outbound_tx: None,
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    }
}

fn make_state_with_webhooks_and_outbound(
    webhooks: HashMap<String, WebhookConfig>,
) -> (HttpApiState, mpsc::Receiver<OutboundMessage>) {
    let (outbound_tx, outbound_rx) = mpsc::channel(16);
    (
        HttpApiState {
            inbound_tx: Arc::new(mpsc::channel(1).0),
            pending: Arc::new(Mutex::new(HashMap::new())),
            webhooks: Arc::new(webhooks),
            outbound_tx: Some(Arc::new(outbound_tx)),
            leak_detector: Arc::new(NoopRedactor),
            ready: Arc::new(AtomicBool::new(true)),
            status: Arc::new(OnceLock::new()),
            echo_mode: false,
        },
        outbound_rx,
    )
}

fn sign_body(secret: &str, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

#[tokio::test]
async fn test_webhook_disabled_returns_404() {
    use axum::http::Request;
    use tower::ServiceExt;

    let mut webhooks = HashMap::new();
    webhooks.insert(
        "test-hook".to_string(),
        make_webhook_config(false, "secret123"),
    );
    let (state, _outbound_rx) = make_state_with_webhooks_and_outbound(webhooks);
    let app = build_router(state, None, None, None);

    let body = b"payload";
    let sig = sign_body("secret123", body);
    let req = Request::builder()
        .method("POST")
        .uri("/api/webhook/test-hook")
        .header("X-Signature-256", &sig)
        .body(axum::body::Body::from(&body[..]))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_webhook_enabled_validates_signature() {
    use axum::http::Request;
    use tower::ServiceExt;

    let mut webhooks = HashMap::new();
    webhooks.insert(
        "test-hook".to_string(),
        make_webhook_config(true, "secret123"),
    );
    let (state, _outbound_rx) = make_state_with_webhooks_and_outbound(webhooks);
    let app = build_router(state, None, None, None);

    let body = b"payload";
    let sig = sign_body("secret123", body);
    let req = Request::builder()
        .method("POST")
        .uri("/api/webhook/test-hook")
        .header("X-Signature-256", &sig)
        .body(axum::body::Body::from(&body[..]))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    // Valid signature on enabled webhook — should succeed (200 OK)
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_webhook_bad_signature_returns_forbidden() {
    use axum::http::Request;
    use tower::ServiceExt;

    let mut webhooks = HashMap::new();
    webhooks.insert(
        "test-hook".to_string(),
        make_webhook_config(true, "secret123"),
    );
    let (state, _outbound_rx) = make_state_with_webhooks_and_outbound(webhooks);
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("POST")
        .uri("/api/webhook/test-hook")
        .header("X-Signature-256", "bad-sig")
        .body(axum::body::Body::from("payload"))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_webhook_unknown_name_returns_404() {
    use axum::http::Request;
    use tower::ServiceExt;

    let state = make_state_with_webhooks(HashMap::new());
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("POST")
        .uri("/api/webhook/nonexistent")
        .header("X-Signature-256", "anything")
        .body(axum::body::Body::from("payload"))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_webhook_missing_signature_returns_forbidden() {
    use axum::http::Request;
    use tower::ServiceExt;

    let mut webhooks = HashMap::new();
    webhooks.insert(
        "test-hook".to_string(),
        make_webhook_config(true, "secret123"),
    );
    let state = make_state_with_webhooks(webhooks);
    let app = build_router(state, None, None, None);

    // No signature header at all
    let req = Request::builder()
        .method("POST")
        .uri("/api/webhook/test-hook")
        .body(axum::body::Body::from("payload"))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_webhook_payload_too_large_returns_413() {
    use axum::http::Request;
    use tower::ServiceExt;

    let mut webhooks = HashMap::new();
    webhooks.insert(
        "test-hook".to_string(),
        make_webhook_config(true, "secret123"),
    );
    let state = make_state_with_webhooks(webhooks);
    let app = build_router(state, None, None, None);

    let oversized = vec![b'x'; WEBHOOK_MAX_BODY + 1];
    let sig = sign_body("secret123", &oversized);
    let req = Request::builder()
        .method("POST")
        .uri("/api/webhook/test-hook")
        .header("X-Signature-256", &sig)
        .body(axum::body::Body::from(oversized))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn test_webhook_alternative_signature_headers() {
    use axum::http::Request;
    use tower::ServiceExt;

    let body = b"payload";
    let sig = sign_body("secret123", body);

    // Test X-Hub-Signature-256 (GitHub style)
    for header_name in ["X-Hub-Signature-256", "X-Webhook-Signature"] {
        let mut webhooks = HashMap::new();
        webhooks.insert(
            "test-hook".to_string(),
            make_webhook_config(true, "secret123"),
        );
        let (state, _outbound_rx) = make_state_with_webhooks_and_outbound(webhooks);
        let app = build_router(state, None, None, None);

        let req = Request::builder()
            .method("POST")
            .uri("/api/webhook/test-hook")
            .header(header_name, &sig)
            .body(axum::body::Body::from(&body[..]))
            .unwrap();

        let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "header {header_name} should be accepted"
        );
    }
}

#[tokio::test]
async fn test_webhook_direct_delivery_to_targets() {
    use axum::http::Request;
    use tower::ServiceExt;

    let (outbound_tx, mut outbound_rx) = mpsc::channel(16);

    let mut webhooks = HashMap::new();
    webhooks.insert(
        "deploy".to_string(),
        WebhookConfig {
            secret: "deploy-secret".to_string(),
            template: "Deploy event: {{body}}".to_string(),
            targets: vec![
                WebhookTarget {
                    channel: "slack".to_string(),
                    chat_id: "C123".to_string(),
                },
                WebhookTarget {
                    channel: "telegram".to_string(),
                    chat_id: "456".to_string(),
                },
            ],
            ..Default::default()
        },
    );

    let state = HttpApiState {
        inbound_tx: Arc::new(mpsc::channel(1).0),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(webhooks),
        outbound_tx: Some(Arc::new(outbound_tx)),
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let app = build_router(state, None, None, None);

    let body = b"v2.0 released";
    let sig = sign_body("deploy-secret", body);
    let req = Request::builder()
        .method("POST")
        .uri("/api/webhook/deploy")
        .header("X-Signature-256", &sig)
        .body(axum::body::Body::from(&body[..]))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp_body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["attempted"], 2);
    assert_eq!(json["delivered"], 2);
    assert_eq!(json["failed"], 0);

    // Verify both targets received the templated message
    let msg1 = outbound_rx.recv().await.unwrap();
    assert_eq!(msg1.channel, "slack");
    assert_eq!(msg1.chat_id, "C123");
    assert_eq!(msg1.content, "Deploy event: v2.0 released");

    let msg2 = outbound_rx.recv().await.unwrap();
    assert_eq!(msg2.channel, "telegram");
    assert_eq!(msg2.chat_id, "456");
    assert_eq!(msg2.content, "Deploy event: v2.0 released");
}

#[tokio::test]
async fn test_webhook_agent_turn_routes_through_agent() {
    use axum::http::Request;
    use tower::ServiceExt;

    let (inbound_tx, mut inbound_rx) = mpsc::channel(16);
    let (outbound_tx, mut outbound_rx) = mpsc::channel(16);

    let mut webhooks = HashMap::new();
    webhooks.insert(
        "alert".to_string(),
        WebhookConfig {
            secret: "alert-secret".to_string(),
            template: "Alert: {{body}}".to_string(),
            targets: vec![WebhookTarget {
                channel: "discord".to_string(),
                chat_id: "G789".to_string(),
            }],
            agent_turn: true,
            ..Default::default()
        },
    );

    let state = HttpApiState {
        inbound_tx: Arc::new(inbound_tx),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(webhooks),
        outbound_tx: Some(Arc::new(outbound_tx)),
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let pending = state.pending.clone();
    let app = build_router(state, None, None, None);

    let body = b"server down";
    let sig = sign_body("alert-secret", body);
    let req = Request::builder()
        .method("POST")
        .uri("/api/webhook/alert")
        .header("X-Signature-256", &sig)
        .body(axum::body::Body::from(&body[..]))
        .unwrap();

    // Spawn the request so we can simulate the agent response concurrently
    let handle = tokio::spawn(async move { app.oneshot(req).await.unwrap() });

    // Receive the inbound message (agent would normally process this)
    let inbound = inbound_rx.recv().await.unwrap();
    assert_eq!(inbound.content, "Alert: server down");
    assert_eq!(inbound.sender_id, "webhook:alert");

    // Simulate agent response by sending to the pending oneshot
    let request_id = inbound.chat_id.clone();
    let tx = pending.lock().unwrap().remove(&request_id).unwrap();
    tx.send(
        OutboundMessage::builder("http", request_id, "I'm investigating the server issue.").build(),
    )
    .unwrap();

    let resp = handle.await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp_body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["attempted"], 1);
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["failed"], 0);

    // Verify the agent's response was delivered to the target
    let delivered = outbound_rx.recv().await.unwrap();
    assert_eq!(delivered.channel, "discord");
    assert_eq!(delivered.chat_id, "G789");
    assert_eq!(delivered.content, "I'm investigating the server issue.");
}

#[tokio::test]
async fn test_chat_handler_sends_inbound_and_returns_response() {
    use axum::http::Request;
    use tower::ServiceExt;

    let (inbound_tx, mut inbound_rx) = mpsc::channel(16);
    let state = HttpApiState {
        inbound_tx: Arc::new(inbound_tx),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(HashMap::new()),
        outbound_tx: None,
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let pending = state.pending.clone();
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("POST")
        .uri("/api/chat")
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(r#"{"message":"hello"}"#))
        .unwrap();

    // Spawn the request so we can simulate the agent response concurrently
    let handle = tokio::spawn(async move { app.oneshot(req).await.unwrap() });

    // Receive the inbound message
    let msg = inbound_rx.recv().await.unwrap();
    assert_eq!(msg.channel, "http");
    assert_eq!(msg.content, "hello");
    assert_eq!(msg.sender_id, "http-api");

    // Send a response through the pending oneshot
    let request_id = msg.chat_id.clone();
    let tx = pending.lock().unwrap().remove(&request_id).unwrap();
    tx.send(OutboundMessage::builder("http", request_id, "world").build())
        .unwrap();

    let resp = handle.await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["content"], "world");
}

#[tokio::test]
async fn test_chat_handler_with_session_id() {
    use axum::http::Request;
    use tower::ServiceExt;

    let (inbound_tx, mut inbound_rx) = mpsc::channel(16);
    let state = HttpApiState {
        inbound_tx: Arc::new(inbound_tx),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(HashMap::new()),
        outbound_tx: None,
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let pending = state.pending.clone();
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("POST")
        .uri("/api/chat")
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(
            r#"{"message":"hi","session_id":"my-session"}"#,
        ))
        .unwrap();

    let handle = tokio::spawn(async move { app.oneshot(req).await.unwrap() });

    let msg = inbound_rx.recv().await.unwrap();
    let request_id = msg.chat_id.clone();
    let tx = pending.lock().unwrap().remove(&request_id).unwrap();
    tx.send(OutboundMessage::builder("http", request_id, "reply").build())
        .unwrap();

    let resp = handle.await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["session_id"], "my-session");
}

#[tokio::test]
async fn test_chat_handler_creates_pending_and_publishes_inbound() {
    use axum::http::Request;
    use tower::ServiceExt;

    let (inbound_tx, mut inbound_rx) = mpsc::channel(16);
    let state = HttpApiState {
        inbound_tx: Arc::new(inbound_tx),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(HashMap::new()),
        outbound_tx: None,
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let pending = state.pending.clone();
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("POST")
        .uri("/api/chat")
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(r#"{"message":"test"}"#))
        .unwrap();

    // Spawn but don't resolve — we just want to verify setup
    let _handle = tokio::spawn(async move { app.oneshot(req).await.unwrap() });

    // Wait for the inbound message to arrive
    let msg = inbound_rx.recv().await.unwrap();
    assert_eq!(msg.channel, "http");
    assert_eq!(msg.content, "test");
    assert!(msg.chat_id.starts_with("http-"));

    // Verify the pending map has an entry for this request
    let has_pending = pending.lock().unwrap().contains_key(&msg.chat_id);
    assert!(has_pending, "pending map should contain the request_id");
}

#[tokio::test]
async fn test_deliver_to_targets_no_outbound_tx() {
    let state = HttpApiState {
        inbound_tx: Arc::new(mpsc::channel(1).0),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(HashMap::new()),
        outbound_tx: None,
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let targets = vec![WebhookTarget {
        channel: "slack".to_string(),
        chat_id: "C123".to_string(),
    }];
    let outcome = deliver_to_targets(&state, &targets, "hello", "test-hook").await;
    assert_eq!(outcome.attempted, 1);
    assert_eq!(outcome.delivered, 0);
    assert_eq!(outcome.failed, 1);
}

#[test]
fn test_extract_rate_limit_client_ip_ignores_spoofed_xff_without_trusted_proxy() {
    let state = RateLimitState {
        limiter: Arc::new(governor::RateLimiter::keyed(governor::Quota::per_second(
            std::num::NonZeroU32::new(1).unwrap(),
        ))),
        trust_proxy: true,
        trusted_proxies: Arc::new(vec!["10.0.0.0/8".parse::<IpNet>().unwrap()]),
        retry_after_secs: 1,
    };
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "198.51.100.10".parse().unwrap());

    let ip = extract_rate_limit_client_ip(
        &state,
        &headers,
        Some(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)),
            443,
        ))),
    );
    assert_eq!(ip, "203.0.113.7");
}

#[test]
fn test_extract_rate_limit_client_ip_uses_xff_for_trusted_proxy() {
    let state = RateLimitState {
        limiter: Arc::new(governor::RateLimiter::keyed(governor::Quota::per_second(
            std::num::NonZeroU32::new(1).unwrap(),
        ))),
        trust_proxy: true,
        trusted_proxies: Arc::new(vec!["10.0.0.0/8".parse::<IpNet>().unwrap()]),
        retry_after_secs: 1,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        "198.51.100.10, 10.1.2.3".parse().unwrap(),
    );

    let ip = extract_rate_limit_client_ip(
        &state,
        &headers,
        Some(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)),
            443,
        ))),
    );
    assert_eq!(ip, "198.51.100.10");
}

#[tokio::test]
async fn test_webhook_direct_delivery_returns_error_when_no_targets_accept_delivery() {
    use axum::http::Request;
    use tower::ServiceExt;

    let (outbound_tx, outbound_rx) = mpsc::channel(1);
    drop(outbound_rx);

    let mut webhooks = HashMap::new();
    webhooks.insert(
        "deploy".to_string(),
        WebhookConfig {
            secret: "deploy-secret".to_string(),
            template: "Deploy event: {{body}}".to_string(),
            targets: vec![WebhookTarget {
                channel: "slack".to_string(),
                chat_id: "C123".to_string(),
            }],
            ..Default::default()
        },
    );

    let state = HttpApiState {
        inbound_tx: Arc::new(mpsc::channel(1).0),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(webhooks),
        outbound_tx: Some(Arc::new(outbound_tx)),
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let app = build_router(state, None, None, None);

    let body = b"v2.0 released";
    let sig = sign_body("deploy-secret", body);
    let req = Request::builder()
        .method("POST")
        .uri("/api/webhook/deploy")
        .header("X-Signature-256", &sig)
        .body(axum::body::Body::from(&body[..]))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    let resp_body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
    assert_eq!(json["delivered"], 0);
    assert_eq!(json["failed"], 1);
}

#[tokio::test]
async fn test_webhook_template_with_json_fields() {
    use axum::http::Request;
    use tower::ServiceExt;

    let (outbound_tx, mut outbound_rx) = mpsc::channel(16);

    let mut webhooks = HashMap::new();
    webhooks.insert(
        "gh-push".to_string(),
        WebhookConfig {
            secret: "json-secret".to_string(),
            template: "{{action}} on {{repo}}".to_string(),
            targets: vec![WebhookTarget {
                channel: "slack".to_string(),
                chat_id: "C456".to_string(),
            }],
            ..Default::default()
        },
    );

    let state = HttpApiState {
        inbound_tx: Arc::new(mpsc::channel(1).0),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(webhooks),
        outbound_tx: Some(Arc::new(outbound_tx)),
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let app = build_router(state, None, None, None);

    let body = serde_json::to_vec(&serde_json::json!({"action":"push","repo":"test"})).unwrap();
    let sig = sign_body("json-secret", &body);
    let req = Request::builder()
        .method("POST")
        .uri("/api/webhook/gh-push")
        .header("X-Signature-256", &sig)
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let msg = outbound_rx.recv().await.unwrap();
    assert_eq!(msg.content, "push on test");
    assert_eq!(msg.channel, "slack");
    assert_eq!(msg.chat_id, "C456");
}

#[test]
fn test_apply_template_body_does_not_expand_keys() {
    let json: serde_json::Value = serde_json::json!({"secret": "s3cret-value"});
    let result = apply_template("Event: {{body}}", "check {{secret}} here", Some(&json));
    // {{secret}} in body text must NOT be expanded — body is literal
    assert_eq!(result, "Event: check {{secret}} here");
}

#[tokio::test]
async fn test_chat_handler_rejects_oversized_message() {
    use axum::http::Request;
    use tower::ServiceExt;

    let (inbound_tx, _rx) = mpsc::channel(16);
    let state = HttpApiState {
        inbound_tx: Arc::new(inbound_tx),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(HashMap::new()),
        outbound_tx: None,
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let app = build_router(state, None, None, None);

    let big_msg = "x".repeat(MAX_MESSAGE_SIZE + 1);
    let body = serde_json::json!({"message": big_msg});
    let req = Request::builder()
        .method("POST")
        .uri("/api/chat")
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn test_chat_handler_inbound_send_fails() {
    use axum::http::Request;
    use tower::ServiceExt;

    // Create a channel and immediately drop the receiver so send fails
    let (inbound_tx, inbound_rx) = mpsc::channel(1);
    drop(inbound_rx);

    let state = HttpApiState {
        inbound_tx: Arc::new(inbound_tx),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(HashMap::new()),
        outbound_tx: None,
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("POST")
        .uri("/api/chat")
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(r#"{"message":"hello"}"#))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[test]
fn test_gateway_response_format_deserialize_json_string() {
    let body: ChatRequest =
        serde_json::from_str(r#"{"message":"hi","responseFormat":"json"}"#).unwrap();
    assert!(body.response_format.is_some());
    match body.response_format.unwrap() {
        GatewayResponseFormat::Simple(s) => assert_eq!(s, "json"),
        GatewayResponseFormat::Schema { .. } => panic!("expected Simple variant"),
    }
}

#[test]
fn test_gateway_response_format_deserialize_schema_object() {
    let body: ChatRequest = serde_json::from_str(
        r#"{"message":"hi","responseFormat":{"name":"test","schema":{"type":"object"}}}"#,
    )
    .unwrap();
    assert!(body.response_format.is_some());
    match body.response_format.unwrap() {
        GatewayResponseFormat::Schema { name, schema } => {
            assert_eq!(name, "test");
            assert_eq!(schema, serde_json::json!({"type": "object"}));
        }
        GatewayResponseFormat::Simple(_) => panic!("expected Schema variant"),
    }
}

#[test]
fn test_gateway_response_format_deserialize_absent() {
    let body: ChatRequest = serde_json::from_str(r#"{"message":"hi"}"#).unwrap();
    assert!(body.response_format.is_none());
}

#[test]
fn test_response_format_json_roundtrip_json_object() {
    use oxicrab_core::providers::base::ResponseFormat;
    let rf = ResponseFormat::JsonObject;
    let json = response_format_to_json(&rf);
    assert_eq!(json, serde_json::Value::String("json".to_string()));
    let parsed = response_format_from_json(&json).unwrap();
    match parsed {
        ResponseFormat::JsonObject => {}
        ResponseFormat::JsonSchema { .. } => panic!("expected JsonObject"),
    }
}

#[test]
fn test_response_format_json_roundtrip_json_schema() {
    use oxicrab_core::providers::base::ResponseFormat;
    let rf = ResponseFormat::JsonSchema {
        name: "my_schema".to_string(),
        schema: serde_json::json!({"type": "object", "properties": {"x": {"type": "number"}}}),
    };
    let json = response_format_to_json(&rf);
    let parsed = response_format_from_json(&json).unwrap();
    match parsed {
        ResponseFormat::JsonSchema { name, schema } => {
            assert_eq!(name, "my_schema");
            assert_eq!(
                schema,
                serde_json::json!({"type": "object", "properties": {"x": {"type": "number"}}})
            );
        }
        ResponseFormat::JsonObject => panic!("expected JsonSchema"),
    }
}

#[test]
fn test_response_format_from_json_invalid() {
    assert!(response_format_from_json(&serde_json::Value::Null).is_none());
    assert!(response_format_from_json(&serde_json::Value::String("xml".to_string())).is_none());
    assert!(response_format_from_json(&serde_json::json!(42)).is_none());
}

#[test]
fn test_gateway_response_format_into_response_format() {
    use oxicrab_core::providers::base::ResponseFormat;

    // "json" -> JsonObject
    let grf = GatewayResponseFormat::Simple("json".to_string());
    match grf.into_response_format() {
        ResponseFormat::JsonObject => {}
        ResponseFormat::JsonSchema { .. } => panic!("expected JsonObject"),
    }

    // Unknown string -> fallback to JsonObject
    let grf = GatewayResponseFormat::Simple("xml".to_string());
    match grf.into_response_format() {
        ResponseFormat::JsonObject => {}
        ResponseFormat::JsonSchema { .. } => panic!("expected JsonObject fallback"),
    }

    // Schema variant
    let grf = GatewayResponseFormat::Schema {
        name: "test".to_string(),
        schema: serde_json::json!({"type": "string"}),
    };
    match grf.into_response_format() {
        ResponseFormat::JsonSchema { name, schema } => {
            assert_eq!(name, "test");
            assert_eq!(schema, serde_json::json!({"type": "string"}));
        }
        ResponseFormat::JsonObject => panic!("expected JsonSchema"),
    }
}

#[tokio::test]
async fn test_chat_handler_with_response_format_metadata() {
    use axum::http::Request;
    use tower::ServiceExt;

    let (inbound_tx, mut inbound_rx) = mpsc::channel(16);
    let state = HttpApiState {
        inbound_tx: Arc::new(inbound_tx),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(HashMap::new()),
        outbound_tx: None,
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let pending = state.pending.clone();
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("POST")
        .uri("/api/chat")
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(
            r#"{"message":"list items","responseFormat":"json"}"#,
        ))
        .unwrap();

    let handle = tokio::spawn(async move { app.oneshot(req).await.unwrap() });

    let msg = inbound_rx.recv().await.unwrap();
    assert_eq!(msg.content, "list items");
    // Verify response_format was serialized into metadata
    let rf_meta = msg
        .metadata
        .get(oxicrab_core::bus::meta::RESPONSE_FORMAT)
        .unwrap();
    assert_eq!(rf_meta, &serde_json::Value::String("json".to_string()));

    // Complete the request to avoid timeout
    let request_id = msg.chat_id.clone();
    let tx = pending.lock().unwrap().remove(&request_id).unwrap();
    tx.send(OutboundMessage::builder("http", request_id, r#"{"items":[]}"#).build())
        .unwrap();

    let resp = handle.await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_chat_handler_without_response_format_no_metadata() {
    use axum::http::Request;
    use tower::ServiceExt;

    let (inbound_tx, mut inbound_rx) = mpsc::channel(16);
    let state = HttpApiState {
        inbound_tx: Arc::new(inbound_tx),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(HashMap::new()),
        outbound_tx: None,
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let pending = state.pending.clone();
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("POST")
        .uri("/api/chat")
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(r#"{"message":"hello"}"#))
        .unwrap();

    let handle = tokio::spawn(async move { app.oneshot(req).await.unwrap() });

    let msg = inbound_rx.recv().await.unwrap();
    assert!(
        !msg.metadata
            .contains_key(oxicrab_core::bus::meta::RESPONSE_FORMAT)
    );

    let request_id = msg.chat_id.clone();
    let tx = pending.lock().unwrap().remove(&request_id).unwrap();
    tx.send(OutboundMessage::builder("http", request_id, "world").build())
        .unwrap();

    let resp = handle.await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_chat_handler_rejects_oversized_schema() {
    use axum::http::Request;
    use tower::ServiceExt;

    let (inbound_tx, _inbound_rx) = mpsc::channel(16);
    let state = HttpApiState {
        inbound_tx: Arc::new(inbound_tx),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(HashMap::new()),
        outbound_tx: None,
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let app = build_router(state, None, None, None);

    // Create a schema that exceeds MAX_SCHEMA_SIZE (100 KB)
    let large_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "data": {
                "type": "string",
                "description": "x".repeat(101 * 1024) // 101 KB string
            }
        }
    });

    let req_body = serde_json::json!({
        "message": "test",
        "responseFormat": {
            "name": "test",
            "schema": large_schema
        }
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/chat")
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(
            serde_json::to_string(&req_body).unwrap(),
        ))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(body["error"].as_str().unwrap().contains("schema too large"));
}

#[tokio::test]
async fn test_chat_handler_rejects_oversized_schema_name() {
    use axum::http::Request;
    use tower::ServiceExt;

    let (inbound_tx, _inbound_rx) = mpsc::channel(16);
    let state = HttpApiState {
        inbound_tx: Arc::new(inbound_tx),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(HashMap::new()),
        outbound_tx: None,
        leak_detector: Arc::new(NoopRedactor),
        ready: Arc::new(AtomicBool::new(true)),
        status: Arc::new(OnceLock::new()),
        echo_mode: false,
    };
    let app = build_router(state, None, None, None);

    let req_body = serde_json::json!({
        "message": "test",
        "responseFormat": {
            "name": "x".repeat(257), // 257 chars, exceeds 256 limit
            "schema": {"type": "string"}
        }
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/chat")
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(
            serde_json::to_string(&req_body).unwrap(),
        ))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("schema name too long")
    );
}

#[tokio::test]
async fn test_webhook_replay_protection_rejects_old_timestamp() {
    use axum::http::Request;
    use tower::ServiceExt;

    let mut webhooks = HashMap::new();
    webhooks.insert(
        "test-hook".to_string(),
        make_webhook_config(true, "secret123"),
    );
    let (state, _outbound_rx) = make_state_with_webhooks_and_outbound(webhooks);
    let app = build_router(state, None, None, None);

    let body = b"payload";
    let sig = sign_body("secret123", body);
    // Timestamp 10 minutes ago — should be rejected
    let old_ts = (chrono::Utc::now().timestamp() - 600).to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/webhook/test-hook")
        .header("X-Signature-256", &sig)
        .header("X-Webhook-Timestamp", &old_ts)
        .body(axum::body::Body::from(&body[..]))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_webhook_replay_protection_accepts_recent_timestamp() {
    use axum::http::Request;
    use tower::ServiceExt;

    let mut webhooks = HashMap::new();
    webhooks.insert(
        "test-hook".to_string(),
        make_webhook_config(true, "secret123"),
    );
    let (state, _outbound_rx) = make_state_with_webhooks_and_outbound(webhooks);
    let app = build_router(state, None, None, None);

    let body = b"payload";
    let sig = sign_body("secret123", body);
    // Timestamp 1 minute ago — should be accepted
    let recent_ts = (chrono::Utc::now().timestamp() - 60).to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/webhook/test-hook")
        .header("X-Signature-256", &sig)
        .header("X-Webhook-Timestamp", &recent_ts)
        .body(axum::body::Body::from(&body[..]))
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_status_json_initializing() {
    use axum::http::Request;
    use tower::ServiceExt;

    let state = make_state();
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("GET")
        .uri("/api/status")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["mode"], "initializing");
    assert_eq!(json["status"], "unavailable");
}

#[tokio::test]
async fn test_status_json_true_echo_mode() {
    use axum::http::Request;
    use tower::ServiceExt;

    let mut state = make_state();
    state.echo_mode = true;
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("GET")
        .uri("/api/status")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["mode"], "echo");
    assert_eq!(json["status"], "unavailable");
}

#[tokio::test]
async fn test_status_html_endpoint() {
    use axum::http::Request;
    use tower::ServiceExt;

    let state = make_state();
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("GET")
        .uri("/status")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/html"));
}

#[tokio::test]
async fn test_status_html_requires_auth_when_key_set() {
    use axum::http::Request;
    use tower::ServiceExt;

    let state = make_state();
    let app = build_router(state, None, Some(Arc::new("test-key".to_string())), None);

    let req = Request::builder()
        .method("GET")
        .uri("/status")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_status_json_requires_auth_when_key_set() {
    use axum::http::Request;
    use tower::ServiceExt;

    let state = make_state();
    let app = build_router(state, None, Some(Arc::new("test-key".to_string())), None);

    let req = Request::builder()
        .method("GET")
        .uri("/api/status")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_status_json_with_populated_state() {
    use axum::http::Request;
    use tower::ServiceExt;

    let db = Arc::new(oxicrab_memory::MemoryDB::new(":memory:").expect("in-memory DB"));
    let status_state = status::StatusState {
        start_time: std::time::Instant::now(),
        config_snapshot: Arc::new(status::StatusConfigSnapshot {
            models: status::ModelsSnapshot {
                default: "test/model".to_string(),
                tasks: HashMap::new(),
                fallbacks: vec![],
                chat_routing: None,
            },
            channels: status::ChannelsSnapshot {
                telegram: true,
                discord: false,
                slack: false,
                whatsapp: false,
                twilio: false,
            },
            safety: status::SafetySnapshot {
                prompt_guard: status::PromptGuardSnapshot {
                    enabled: true,
                    action: "Block".to_string(),
                },
                exfiltration_guard: false,
                sandbox: status::SandboxSnapshot {
                    enabled: true,
                    block_network: true,
                },
            },
            gateway: status::GatewaySnapshot {
                rate_limit: status::RateLimitSnapshot {
                    enabled: false,
                    rps: 10,
                    burst: 30,
                },
                webhooks: vec![],
                a2a: false,
            },
            embeddings_enabled: false,
        }),
        tool_snapshot: Arc::new(status::ToolSnapshot {
            total: 2,
            deferred: 0,
            by_category: {
                let mut m = HashMap::new();
                m.insert("Core".to_string(), vec!["shell".to_string()]);
                m
            },
        }),
        memory_db: db,
    };

    let lock = Arc::new(OnceLock::new());
    let _ = lock.set(status_state);

    let mut state = make_state();
    state.status = lock;
    let app = build_router(state, None, None, None);

    let req = Request::builder()
        .method("GET")
        .uri("/api/status")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["version"], VERSION);
    assert!(json["uptime_seconds"].is_number());
    assert_eq!(json["models"]["default"], "test/model");
    assert_eq!(json["channels"]["telegram"], true);
    assert_eq!(json["tools"]["total"], 2);
    assert!(json["tokens"]["today"]["input"].is_number());
    assert!(json["cron"]["jobs"].is_array());
}
