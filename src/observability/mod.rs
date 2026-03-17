use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

static METRICS_INIT: OnceLock<()> = OnceLock::new();
static BASELINE_LOOP_INIT: OnceLock<()> = OnceLock::new();
static PROCESS_START_MONO: OnceLock<Instant> = OnceLock::new();
static PROCESS_START_UNIX: OnceLock<f64> = OnceLock::new();

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
            init_baseline_metrics(metrics_cfg.bind.clone());
            info!("metrics exporter enabled on http://{addr}/metrics");
        }
        Err(e) => {
            warn!("failed to initialize metrics exporter: {}", e);
        }
    }
}

fn init_baseline_metrics(bind: String) {
    let start_mono = *PROCESS_START_MONO.get_or_init(Instant::now);
    let start_unix = *PROCESS_START_UNIX.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64())
    });

    // Stable one-shot baseline metrics.
    metrics::gauge!(
        "oxicrab_build_info",
        "version" => env!("CARGO_PKG_VERSION"),
        "profile" => option_env!("PROFILE").unwrap_or("unknown"),
        "target" => option_env!("TARGET").unwrap_or("unknown"),
        "git_sha" => option_env!("VERGEN_GIT_SHA").unwrap_or("unknown")
    )
    .set(1.0);
    metrics::gauge!("oxicrab_up").set(1.0);
    metrics::gauge!("oxicrab_start_time_seconds").set(start_unix);
    metrics::gauge!("oxicrab_metrics_exporter_info", "bind" => bind).set(1.0);
    metrics::gauge!("oxicrab_runtime_threads_available")
        .set(std::thread::available_parallelism().map_or(0.0, |n| n.get() as f64));

    if BASELINE_LOOP_INIT.get().is_some() {
        return;
    }

    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        warn!("metrics baseline sampler disabled: no active tokio runtime");
        return;
    };

    let _ = BASELINE_LOOP_INIT.set(());
    handle.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            publish_runtime_metrics(start_mono, start_unix);
        }
    });
}

fn publish_runtime_metrics(start_mono: Instant, start_unix: f64) {
    metrics::counter!("oxicrab_runtime_sampler_runs_total").increment(1);
    metrics::gauge!("oxicrab_uptime_seconds").set(start_mono.elapsed().as_secs_f64());
    metrics::gauge!("oxicrab_runtime_last_sample_unix_seconds").set(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(start_unix, |d| d.as_secs_f64()),
    );
    metrics::gauge!("oxicrab_runtime_executor_up").set(1.0);

    let fd_count = std::fs::read_dir("/proc/self/fd").map_or(0.0, |rd| rd.count() as f64);
    metrics::gauge!("oxicrab_process_open_fds").set(fd_count);

    if let Some((rss_bytes, vmsize_bytes, threads, cpu_seconds)) = read_proc_metrics() {
        metrics::gauge!("oxicrab_process_resident_memory_bytes").set(rss_bytes as f64);
        metrics::gauge!("oxicrab_process_virtual_memory_bytes").set(vmsize_bytes as f64);
        metrics::gauge!("oxicrab_process_threads").set(threads as f64);
        metrics::gauge!("oxicrab_process_cpu_seconds_total").set(cpu_seconds);
    }
}

fn read_proc_metrics() -> Option<(u64, u64, u64, f64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let rss_kb = parse_status_kb(&status, "VmRSS:")?;
    let vm_kb = parse_status_kb(&status, "VmSize:")?;
    let threads = parse_status_u64(&status, "Threads:")?;

    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let rparen = stat.rfind(')')?;
    let after = stat.get(rparen + 2..)?;
    let fields: Vec<&str> = after.split_whitespace().collect();
    if fields.len() <= 12 {
        return None;
    }
    let utime_ticks = fields[11].parse::<u64>().ok()?;
    let stime_ticks = fields[12].parse::<u64>().ok()?;
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let hz = if hz > 0 { hz as f64 } else { 100.0 };
    let cpu_seconds = (utime_ticks + stime_ticks) as f64 / hz;

    Some((rss_kb * 1024, vm_kb * 1024, threads, cpu_seconds))
}

fn parse_status_kb(input: &str, key: &str) -> Option<u64> {
    parse_status_u64(input, key)
}

fn parse_status_u64(input: &str, key: &str) -> Option<u64> {
    let line = input.lines().find(|line| line.starts_with(key))?;
    line.split_whitespace().nth(1)?.parse::<u64>().ok()
}
