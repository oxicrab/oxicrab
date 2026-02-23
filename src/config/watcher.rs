use crate::config::Config;
use anyhow::{Context, Result};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Debounce window for file system events.
const DEBOUNCE_MS: u64 = 400;

/// Start watching a config file for changes. Returns a `watch::Receiver` that
/// broadcasts validated config updates and a handle to the background task.
///
/// The watcher monitors the **parent directory** to handle editor write-to-temp-
/// then-rename patterns (e.g. vim, emacs). Only events matching the config
/// filename trigger a reload attempt.
pub fn start_watching(
    config_path: &Path,
    initial: Config,
) -> Result<(watch::Receiver<Config>, JoinHandle<()>)> {
    let config_path = config_path
        .canonicalize()
        .with_context(|| format!("cannot canonicalize config path: {}", config_path.display()))?;
    let parent = config_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("config path has no parent directory"))?
        .to_path_buf();
    let filename = config_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("config path has no filename"))?
        .to_os_string();

    let (watch_tx, watch_rx) = watch::channel(initial);

    // Bridge notify's std mpsc to a tokio mpsc so we can await events
    let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel(64);
    let mut watcher: RecommendedWatcher = Watcher::new(
        move |res| {
            let _ = bridge_tx.blocking_send(res);
        },
        notify::Config::default(),
    )
    .context("failed to create file watcher")?;
    watcher
        .watch(&parent, RecursiveMode::NonRecursive)
        .with_context(|| format!("failed to watch directory: {}", parent.display()))?;

    let handle = tokio::spawn(async move {
        // Keep watcher alive for the duration of the task
        let _watcher = watcher;

        loop {
            let event = match bridge_rx.recv().await {
                Some(Ok(event)) => event,
                Some(Err(e)) => {
                    warn!("file watcher error: {}", e);
                    continue;
                }
                None => {
                    debug!("file watcher channel closed, stopping");
                    break;
                }
            };

            // Only react to modify/create events for our config file
            if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                continue;
            }

            let is_our_file = event
                .paths
                .iter()
                .any(|p| p.file_name().is_some_and(|f| f == filename));
            if !is_our_file {
                continue;
            }

            // Debounce: wait a bit for the write to settle
            tokio::time::sleep(tokio::time::Duration::from_millis(DEBOUNCE_MS)).await;

            // Drain any additional events that arrived during debounce
            while bridge_rx.try_recv().is_ok() {}

            // Attempt reload
            match reload_config(&config_path) {
                Ok(new_config) => {
                    info!("config reloaded successfully");
                    let _ = watch_tx.send(new_config);
                }
                Err(e) => {
                    warn!("config reload failed (keeping previous config): {}", e);
                }
            }
        }
    });

    info!("config watcher started");
    Ok((watch_rx, handle))
}

fn reload_config(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config file: {}", path.display()))?;
    let config: Config =
        serde_yaml_ng::from_str(&content).context("failed to parse config file")?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reload_config_invalid_path() {
        let result = reload_config(Path::new("/nonexistent/config.yaml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_reload_config_valid_yaml() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "{}").unwrap();
        let result = reload_config(tmp.path());
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_start_watching_valid_path() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "{}").unwrap();
        let initial = Config::default();
        let result = start_watching(tmp.path(), initial);
        assert!(result.is_ok());
        let (_rx, handle) = result.unwrap();
        handle.abort();
    }
}
