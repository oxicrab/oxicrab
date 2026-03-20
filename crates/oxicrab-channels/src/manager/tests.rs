use super::*;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

/// Mock channel with configurable failure behavior.
/// `fail_count` controls how many times `send()` fails before succeeding.
/// Set to `usize::MAX` for a channel that always fails.
struct MockChannel {
    channel_name: String,
    fail_count: Arc<AtomicUsize>,
    send_attempts: Arc<AtomicUsize>,
}

impl MockChannel {
    fn new(name: &str, fail_count: usize) -> Self {
        Self {
            channel_name: name.to_string(),
            fail_count: Arc::new(AtomicUsize::new(fail_count)),
            send_attempts: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl BaseChannel for MockChannel {
    fn name(&self) -> &str {
        &self.channel_name
    }

    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn send(&self, _msg: &OutboundMessage) -> anyhow::Result<()> {
        let attempt = self.send_attempts.fetch_add(1, Ordering::SeqCst);
        if attempt < self.fail_count.load(Ordering::SeqCst) {
            Err(anyhow::anyhow!(
                "mock send failure (attempt {})",
                attempt + 1
            ))
        } else {
            Ok(())
        }
    }
}

/// Mock channel with controllable health status and start/stop tracking.
struct SupervisorMockChannel {
    channel_name: String,
    healthy: Arc<AtomicBool>,
    start_count: Arc<AtomicU32>,
    stop_count: Arc<AtomicU32>,
    start_fails: bool,
}

impl SupervisorMockChannel {
    fn new(name: &str) -> Self {
        Self {
            channel_name: name.to_string(),
            healthy: Arc::new(AtomicBool::new(true)),
            start_count: Arc::new(AtomicU32::new(0)),
            stop_count: Arc::new(AtomicU32::new(0)),
            start_fails: false,
        }
    }

    fn with_start_fails(mut self) -> Self {
        self.start_fails = true;
        self
    }
}

#[async_trait]
impl BaseChannel for SupervisorMockChannel {
    fn name(&self) -> &str {
        &self.channel_name
    }

    async fn start(&mut self) -> anyhow::Result<()> {
        self.start_count.fetch_add(1, Ordering::SeqCst);
        if self.start_fails {
            Err(anyhow::anyhow!("mock start failure"))
        } else {
            Ok(())
        }
    }

    async fn stop(&mut self) -> anyhow::Result<()> {
        self.stop_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn send(&self, _msg: &OutboundMessage) -> anyhow::Result<()> {
        Ok(())
    }

    async fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::SeqCst)
    }
}

/// Mock channel that always fails with a specific error message.
/// Used to test non-retryable error classification.
struct NonRetryableMockChannel {
    channel_name: String,
    error_msg: String,
    send_attempts: Arc<AtomicUsize>,
}

impl NonRetryableMockChannel {
    fn new(name: &str, error_msg: &str) -> Self {
        Self {
            channel_name: name.to_string(),
            error_msg: error_msg.to_string(),
            send_attempts: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl BaseChannel for NonRetryableMockChannel {
    fn name(&self) -> &str {
        &self.channel_name
    }

    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn send(&self, _msg: &OutboundMessage) -> anyhow::Result<()> {
        self.send_attempts.fetch_add(1, Ordering::SeqCst);
        Err(anyhow::anyhow!("{}", self.error_msg))
    }
}

fn make_outbound(channel: &str) -> OutboundMessage {
    OutboundMessage::builder(channel, "chat1", "hello").build()
}

#[tokio::test]
async fn test_send_no_matching_channel() {
    let mgr = ChannelManager::with_channels(vec![]);
    let result = mgr.send(&make_outbound("nonexistent")).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No channel found"));
}

#[tokio::test]
async fn test_send_matching_channel_succeeds() {
    let mgr = ChannelManager::with_channels(vec![Box::new(MockChannel::new("test", 0))]);
    let result = mgr.send(&make_outbound("test")).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_send_retries_on_failure() {
    // Fail first 2 attempts, succeed on 3rd
    let mgr = ChannelManager::with_channels(vec![Box::new(MockChannel::new("test", 2))]);
    let result = mgr.send(&make_outbound("test")).await;
    assert!(result.is_ok(), "should succeed after retries");
}

#[tokio::test]
async fn test_send_exhausts_retries() {
    // Always fail (fail_count > max_attempts=3)
    let mgr = ChannelManager::with_channels(vec![Box::new(MockChannel::new("test", usize::MAX))]);
    let result = mgr.send(&make_outbound("test")).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("after 3 attempts"));
}

#[tokio::test]
async fn test_enabled_channels_empty_by_default() {
    let mgr = ChannelManager::with_channels(vec![]);
    assert!(mgr.enabled_channels().is_empty());
}

#[tokio::test]
async fn test_send_typing_no_channel_does_not_panic() {
    let mgr = ChannelManager::with_channels(vec![]);
    // Should return silently, not panic
    mgr.send_typing("nonexistent", "chat1").await;
}

// --- Non-retryable error tests ---

#[tokio::test]
async fn test_send_does_not_retry_not_found() {
    let channel = NonRetryableMockChannel::new("test", "not found");
    let send_attempts = channel.send_attempts.clone();
    let mgr = ChannelManager::with_channels(vec![Box::new(channel)]);
    let result = mgr.send(&make_outbound("test")).await;
    assert!(result.is_err());
    // Should fail immediately without retrying
    assert_eq!(send_attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_send_does_not_retry_unauthorized() {
    let channel = NonRetryableMockChannel::new("test", "unauthorized");
    let send_attempts = channel.send_attempts.clone();
    let mgr = ChannelManager::with_channels(vec![Box::new(channel)]);
    let result = mgr.send(&make_outbound("test")).await;
    assert!(result.is_err());
    assert_eq!(send_attempts.load(Ordering::SeqCst), 1);
}

// --- Supervisor / health check tests ---

#[tokio::test]
async fn test_supervisor_detects_unhealthy_channel() {
    let healthy = Arc::new(AtomicBool::new(true));
    let start_count = Arc::new(AtomicU32::new(0));
    let stop_count = Arc::new(AtomicU32::new(0));

    let channel = SupervisorMockChannel {
        channel_name: "test".to_string(),
        healthy: healthy.clone(),
        start_count: start_count.clone(),
        stop_count: stop_count.clone(),
        start_fails: false,
    };

    let mut mgr = ChannelManager::with_channels(vec![Box::new(channel)]);
    mgr.start_all().await.unwrap();

    // All healthy — no restarts needed
    let restarted = mgr.check_and_restart_unhealthy().await;
    assert_eq!(restarted, 0);

    // Mark unhealthy
    healthy.store(false, Ordering::SeqCst);
    let restarted = mgr.check_and_restart_unhealthy().await;
    assert_eq!(restarted, 1);
    assert_eq!(stop_count.load(Ordering::SeqCst), 1);
    // start_count is 2: once from start_all, once from restart
    assert_eq!(start_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_supervisor_skips_healthy_channels() {
    let healthy = Arc::new(AtomicBool::new(true));
    let start_count = Arc::new(AtomicU32::new(0));
    let stop_count = Arc::new(AtomicU32::new(0));

    let channel = SupervisorMockChannel {
        channel_name: "test".to_string(),
        healthy: healthy.clone(),
        start_count: start_count.clone(),
        stop_count: stop_count.clone(),
        start_fails: false,
    };

    let mut mgr = ChannelManager::with_channels(vec![Box::new(channel)]);
    mgr.start_all().await.unwrap();

    // Run supervisor multiple times — all healthy
    for _ in 0..5 {
        assert_eq!(mgr.check_and_restart_unhealthy().await, 0);
    }
    // Only started once (from start_all)
    assert_eq!(start_count.load(Ordering::SeqCst), 1);
    assert_eq!(stop_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn test_supervisor_multiple_channels_only_restarts_unhealthy() {
    let healthy_a = Arc::new(AtomicBool::new(true));
    let healthy_b = Arc::new(AtomicBool::new(true));
    let start_a = Arc::new(AtomicU32::new(0));
    let start_b = Arc::new(AtomicU32::new(0));
    let stop_a = Arc::new(AtomicU32::new(0));
    let stop_b = Arc::new(AtomicU32::new(0));

    let ch_a = SupervisorMockChannel {
        channel_name: "alpha".to_string(),
        healthy: healthy_a.clone(),
        start_count: start_a.clone(),
        stop_count: stop_a.clone(),
        start_fails: false,
    };
    let ch_b = SupervisorMockChannel {
        channel_name: "beta".to_string(),
        healthy: healthy_b.clone(),
        start_count: start_b.clone(),
        stop_count: stop_b.clone(),
        start_fails: false,
    };

    let mut mgr = ChannelManager::with_channels(vec![Box::new(ch_a), Box::new(ch_b)]);
    mgr.start_all().await.unwrap();

    // Only "beta" goes unhealthy
    healthy_b.store(false, Ordering::SeqCst);
    let restarted = mgr.check_and_restart_unhealthy().await;
    assert_eq!(restarted, 1, "should restart only the unhealthy channel");

    // "alpha" should not have been touched
    assert_eq!(
        stop_a.load(Ordering::SeqCst),
        0,
        "alpha should not be stopped"
    );
    assert_eq!(
        start_a.load(Ordering::SeqCst),
        1,
        "alpha started only once (start_all)"
    );

    // "beta" should have been stopped and restarted
    assert_eq!(
        stop_b.load(Ordering::SeqCst),
        1,
        "beta should be stopped once"
    );
    assert_eq!(
        start_b.load(Ordering::SeqCst),
        2,
        "beta started twice (start_all + restart)"
    );
}

#[tokio::test]
async fn test_supervisor_restart_failure_counted_as_not_restarted() {
    let healthy = Arc::new(AtomicBool::new(false));
    let start_count = Arc::new(AtomicU32::new(0));
    let stop_count = Arc::new(AtomicU32::new(0));

    let channel = SupervisorMockChannel {
        channel_name: "failing".to_string(),
        healthy: healthy.clone(),
        start_count: start_count.clone(),
        stop_count: stop_count.clone(),
        start_fails: true,
    };

    let mut mgr = ChannelManager::with_channels(vec![Box::new(channel)]);

    // Channel is unhealthy and restart will fail
    let restarted = mgr.check_and_restart_unhealthy().await;
    assert_eq!(restarted, 0, "failed restart should not count");
    assert_eq!(
        stop_count.load(Ordering::SeqCst),
        1,
        "stop was still called"
    );
    assert_eq!(start_count.load(Ordering::SeqCst), 1, "start was attempted");
}

#[tokio::test]
async fn test_supervisor_channel_recovers_after_restart() {
    let healthy = Arc::new(AtomicBool::new(true));
    let start_count = Arc::new(AtomicU32::new(0));

    let channel = SupervisorMockChannel {
        channel_name: "recovering".to_string(),
        healthy: healthy.clone(),
        start_count: start_count.clone(),
        stop_count: Arc::new(AtomicU32::new(0)),
        start_fails: false,
    };

    let mut mgr = ChannelManager::with_channels(vec![Box::new(channel)]);
    mgr.start_all().await.unwrap();

    // Goes unhealthy, gets restarted
    healthy.store(false, Ordering::SeqCst);
    let restarted = mgr.check_and_restart_unhealthy().await;
    assert_eq!(restarted, 1);

    // Mark healthy again (simulating successful reconnection)
    healthy.store(true, Ordering::SeqCst);

    // Subsequent checks should find it healthy — no more restarts
    let restarted = mgr.check_and_restart_unhealthy().await;
    assert_eq!(restarted, 0);
    assert_eq!(
        start_count.load(Ordering::SeqCst),
        2,
        "started only twice total"
    );
}

#[tokio::test]
async fn test_start_all_with_mixed_success_and_failure() {
    let ch_ok = SupervisorMockChannel::new("ok_channel");
    let start_ok = ch_ok.start_count.clone();
    let ch_fail = SupervisorMockChannel::new("fail_channel").with_start_fails();

    let mut mgr = ChannelManager::with_channels(vec![Box::new(ch_ok), Box::new(ch_fail)]);
    let result = mgr.start_all().await;

    // Should fail because one channel failed to start
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("fail_channel"),
        "error should mention the failed channel: {err_msg}"
    );

    // The successful channel should have been started and then rolled back (stopped)
    assert_eq!(start_ok.load(Ordering::SeqCst), 1, "ok_channel was started");
}

#[tokio::test]
async fn test_stop_all_stops_channels() {
    let stop_a = Arc::new(AtomicU32::new(0));
    let stop_b = Arc::new(AtomicU32::new(0));

    let ch_a = SupervisorMockChannel {
        channel_name: "a".to_string(),
        healthy: Arc::new(AtomicBool::new(true)),
        start_count: Arc::new(AtomicU32::new(0)),
        stop_count: stop_a.clone(),
        start_fails: false,
    };
    let ch_b = SupervisorMockChannel {
        channel_name: "b".to_string(),
        healthy: Arc::new(AtomicBool::new(true)),
        start_count: Arc::new(AtomicU32::new(0)),
        stop_count: stop_b.clone(),
        start_fails: false,
    };

    let mut mgr = ChannelManager::with_channels(vec![Box::new(ch_a), Box::new(ch_b)]);
    mgr.start_all().await.unwrap();
    mgr.stop_all().await.unwrap();

    assert_eq!(stop_a.load(Ordering::SeqCst), 1, "channel a stopped");
    assert_eq!(stop_b.load(Ordering::SeqCst), 1, "channel b stopped");
}

#[tokio::test]
async fn test_enabled_channels_reflects_added_channels() {
    let ch = SupervisorMockChannel::new("telegram");
    let mgr = ChannelManager::with_channels(vec![Box::new(ch)]);
    assert_eq!(mgr.enabled_channels(), &["telegram"]);
}

// --- is_retryable_channel_error tests ---

#[test]
fn test_retryable_generic_error() {
    let err = anyhow::anyhow!("connection reset by peer");
    assert!(is_retryable_channel_error(&err));
}

#[test]
fn test_non_retryable_not_found() {
    let err = anyhow::anyhow!("channel not found");
    assert!(!is_retryable_channel_error(&err));
}

#[test]
fn test_non_retryable_unauthorized() {
    let err = anyhow::anyhow!("Unauthorized access");
    assert!(!is_retryable_channel_error(&err));
}

#[test]
fn test_non_retryable_forbidden() {
    let err = anyhow::anyhow!("Forbidden: insufficient permissions");
    assert!(!is_retryable_channel_error(&err));
}

#[test]
fn test_non_retryable_invalid() {
    let err = anyhow::anyhow!("Invalid request body");
    assert!(!is_retryable_channel_error(&err));
}

#[test]
fn test_non_retryable_bad_request() {
    let err = anyhow::anyhow!("400 Bad Request");
    assert!(!is_retryable_channel_error(&err));
}

#[test]
fn test_non_retryable_permission() {
    let err = anyhow::anyhow!("Permission denied");
    assert!(!is_retryable_channel_error(&err));
}
