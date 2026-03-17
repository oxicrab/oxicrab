use std::net::SocketAddr;
use std::sync::OnceLock;
use tracing::{info, warn};

static METRICS_INIT: OnceLock<()> = OnceLock::new();

pub fn init_metrics_exporter(config: &crate::config::Config) {
    let metrics_cfg = &config.observability.metrics;
    if !metrics_cfg.enabled {
        return;
    }

    if METRICS_INIT.get().is_some() {
        return;
    }

    let Ok(addr) = metrics_cfg.bind.parse::<SocketAddr>() else {
        warn!(
            "metrics exporter disabled: invalid observability.metrics.bind='{}'",
            metrics_cfg.bind
        );
        return;
    };

    let builder = metrics_exporter_prometheus::PrometheusBuilder::new().with_http_listener(addr);
    match builder.install() {
        Ok(()) => {
            let _ = METRICS_INIT.set(());
            info!("metrics exporter enabled on http://{addr}/metrics");
        }
        Err(e) => {
            warn!("failed to initialize metrics exporter: {}", e);
        }
    }
}
