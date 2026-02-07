use crate::providers::base::LLMProvider;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

const HEARTBEAT_PROMPT: &str = "Read HEARTBEAT.md in your workspace (if it exists).\nFollow any instructions or tasks listed there.\nIf nothing needs attention, reply with just: HEARTBEAT_OK";

pub struct HeartbeatService {
    workspace: PathBuf,
    on_heartbeat: Option<Arc<dyn Fn(String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>> + Send + Sync>>,
    interval_s: u64,
    enabled: bool,
    _triage_provider: Option<Arc<dyn LLMProvider>>,
    _triage_model: Option<String>,
    strategy_file: String,
    _cooldown_s: u64,
    running: Arc<tokio::sync::Mutex<bool>>,
}

impl HeartbeatService {
    pub fn new(
        workspace: PathBuf,
        on_heartbeat: Option<Arc<dyn Fn(String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>> + Send + Sync>>,
        interval_s: u64,
        enabled: bool,
        triage_provider: Option<Arc<dyn LLMProvider>>,
        triage_model: Option<String>,
        strategy_file: String,
        cooldown_after_action: u64,
    ) -> Self {
        Self {
            workspace,
            on_heartbeat,
            interval_s,
            enabled,
            _triage_provider: triage_provider,
            _triage_model: triage_model,
            strategy_file,
            _cooldown_s: cooldown_after_action,
            running: Arc::new(tokio::sync::Mutex::new(false)),
        }
    }

    pub async fn start(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        *self.running.lock().await = true;
        let running = self.running.clone();
        let interval = self.interval_s;
        let on_heartbeat = self.on_heartbeat.clone();
        let workspace = self.workspace.clone();
        let strategy_file = self.strategy_file.clone();

        tokio::spawn(async move {
            loop {
                if !*running.lock().await {
                    break;
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(interval)).await;

                if let Some(ref callback) = on_heartbeat {
                    let prompt = format!("{}\n\nRead {}/{}", HEARTBEAT_PROMPT, workspace.display(), strategy_file);
                    let _ = callback(prompt).await;
                }
            }
        });

        tracing::info!("Heartbeat service started (every {}s)", interval);
        Ok(())
    }

    pub async fn stop(&self) {
        *self.running.lock().await = false;
    }
}
