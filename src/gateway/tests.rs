use super::*;

fn make_state() -> HttpApiState {
    HttpApiState {
        inbound_tx: Arc::new(mpsc::channel(1).0),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(HashMap::new()),
        outbound_tx: None,
    }
}

#[tokio::test]
async fn test_health_endpoint_returns_json() {
    use axum::http::Request;
    use tower::ServiceExt;

    let state = make_state();
    let app = build_router(state);

    let req = Request::builder()
        .method("GET")
        .uri("/api/health")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["version"], crate::VERSION);
}

#[test]
fn test_route_response_non_http_returns_false() {
    let state = make_state();
    let msg = OutboundMessage {
        channel: "telegram".to_string(),
        chat_id: "123".to_string(),
        content: "hello".to_string(),
        reply_to: None,
        media: vec![],
        metadata: HashMap::new(),
    };
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

    let msg = OutboundMessage {
        channel: "http".to_string(),
        chat_id: "req-1".to_string(),
        content: "response text".to_string(),
        reply_to: None,
        media: vec![],
        metadata: HashMap::new(),
    };
    assert!(route_response(&state, msg));
    let received = rx.try_recv().unwrap();
    assert_eq!(received.content, "response text");
}

#[test]
fn test_route_response_http_no_pending() {
    let state = make_state();
    let msg = OutboundMessage {
        channel: "http".to_string(),
        chat_id: "nonexistent".to_string(),
        content: "orphan".to_string(),
        reply_to: None,
        media: vec![],
        metadata: HashMap::new(),
    };
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
        template: "{{body}}".to_string(),
        targets: vec![],
        agent_turn: false,
    }
}

fn make_state_with_webhooks(webhooks: HashMap<String, WebhookConfig>) -> HttpApiState {
    HttpApiState {
        inbound_tx: Arc::new(mpsc::channel(1).0),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(webhooks),
        outbound_tx: None,
    }
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
    let state = make_state_with_webhooks(webhooks);
    let app = build_router(state);

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
    let state = make_state_with_webhooks(webhooks);
    let app = build_router(state);

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
    let state = make_state_with_webhooks(webhooks);
    let app = build_router(state);

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
    let app = build_router(state);

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
    let app = build_router(state);

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
    let app = build_router(state);

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
        let state = make_state_with_webhooks(webhooks);
        let app = build_router(state);

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
            "header {} should be accepted",
            header_name
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
            enabled: true,
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
            agent_turn: false,
        },
    );

    let state = HttpApiState {
        inbound_tx: Arc::new(mpsc::channel(1).0),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(webhooks),
        outbound_tx: Some(Arc::new(outbound_tx)),
    };
    let app = build_router(state);

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
    assert_eq!(json["delivered"], true);

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
            enabled: true,
            secret: "alert-secret".to_string(),
            template: "Alert: {{body}}".to_string(),
            targets: vec![WebhookTarget {
                channel: "discord".to_string(),
                chat_id: "G789".to_string(),
            }],
            agent_turn: true,
        },
    );

    let state = HttpApiState {
        inbound_tx: Arc::new(inbound_tx),
        pending: Arc::new(Mutex::new(HashMap::new())),
        webhooks: Arc::new(webhooks),
        outbound_tx: Some(Arc::new(outbound_tx)),
    };
    let pending = state.pending.clone();
    let app = build_router(state);

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
    tx.send(OutboundMessage {
        channel: "http".to_string(),
        chat_id: request_id,
        content: "I'm investigating the server issue.".to_string(),
        reply_to: None,
        media: vec![],
        metadata: HashMap::new(),
    })
    .unwrap();

    let resp = handle.await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp_body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["delivered"], true);

    // Verify the agent's response was delivered to the target
    let delivered = outbound_rx.recv().await.unwrap();
    assert_eq!(delivered.channel, "discord");
    assert_eq!(delivered.chat_id, "G789");
    assert_eq!(delivered.content, "I'm investigating the server issue.");
}
