use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

static METRICS_INIT: OnceLock<()> = OnceLock::new();
static BASELINE_LOOP_INIT: OnceLock<()> = OnceLock::new();
static PROCESS_START_MONO: OnceLock<Instant> = OnceLock::new();
static PROCESS_START_UNIX: OnceLock<f64> = OnceLock::new();

#[derive(Default)]
struct ProcessMetrics {
    open_fds: Option<u64>,
    rss_bytes: Option<u64>,
    virtual_memory_bytes: Option<u64>,
    threads: Option<u64>,
    cpu_seconds: Option<f64>,
}

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

    if !addr.ip().is_loopback() {
        warn!(
            "metrics exporter is binding to {} without authentication; expose it only on a trusted network or behind a reverse proxy",
            addr
        );
    }

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

    metrics::gauge!(
        "oxicrab_build_info",
        "version" => env!("CARGO_PKG_VERSION"),
        "profile" => env!("OXICRAB_BUILD_PROFILE"),
        "target" => env!("OXICRAB_BUILD_TARGET"),
        "git_sha" => env!("OXICRAB_GIT_SHA")
    )
    .set(1.0);
    metrics::gauge!("oxicrab_up").set(1.0);
    metrics::gauge!("oxicrab_start_time_seconds").set(start_unix);
    metrics::gauge!("oxicrab_metrics_exporter_info", "bind" => bind).set(1.0);
    metrics::gauge!("oxicrab_runtime_threads_available")
        .set(std::thread::available_parallelism().map_or(0.0, |n| n.get() as f64));

    let support = collect_process_metrics();
    publish_support_metric("open_fds", support.open_fds.is_some());
    publish_support_metric("resident_memory", support.rss_bytes.is_some());
    publish_support_metric("virtual_memory", support.virtual_memory_bytes.is_some());
    publish_support_metric("threads", support.threads.is_some());
    publish_support_metric("cpu_seconds", support.cpu_seconds.is_some());

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

fn publish_support_metric(metric: &'static str, supported: bool) {
    metrics::gauge!("oxicrab_process_metric_supported", "metric" => metric).set(if supported {
        1.0
    } else {
        0.0
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

    let stats = collect_process_metrics();
    if let Some(fd_count) = stats.open_fds {
        metrics::gauge!("oxicrab_process_open_fds").set(fd_count as f64);
    }
    if let Some(rss_bytes) = stats.rss_bytes {
        metrics::gauge!("oxicrab_process_resident_memory_bytes").set(rss_bytes as f64);
    }
    if let Some(vmsize_bytes) = stats.virtual_memory_bytes {
        metrics::gauge!("oxicrab_process_virtual_memory_bytes").set(vmsize_bytes as f64);
    }
    if let Some(threads) = stats.threads {
        metrics::gauge!("oxicrab_process_threads").set(threads as f64);
    }
    if let Some(cpu_seconds) = stats.cpu_seconds {
        metrics::gauge!("oxicrab_process_cpu_seconds_total").set(cpu_seconds);
    }
}

#[cfg(target_os = "linux")]
fn collect_process_metrics() -> ProcessMetrics {
    let open_fds = std::fs::read_dir("/proc/self/fd")
        .ok()
        .map(|rd| rd.count() as u64);

    let Some((rss_bytes, virtual_memory_bytes, threads, cpu_seconds)) = read_linux_proc_metrics()
    else {
        return ProcessMetrics {
            open_fds,
            ..ProcessMetrics::default()
        };
    };

    ProcessMetrics {
        open_fds,
        rss_bytes: Some(rss_bytes),
        virtual_memory_bytes: Some(virtual_memory_bytes),
        threads: Some(threads),
        cpu_seconds: Some(cpu_seconds),
    }
}

#[cfg(target_os = "linux")]
fn read_linux_proc_metrics() -> Option<(u64, u64, u64, f64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let rss_kb = parse_status_u64(&status, "VmRSS:")?;
    let vm_kb = parse_status_u64(&status, "VmSize:")?;
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

#[cfg(target_os = "macos")]
fn collect_process_metrics() -> ProcessMetrics {
    use libproc::libproc::bsd_info::BSDInfo;
    use libproc::libproc::file_info::ListFDs;
    use libproc::libproc::pid_rusage::{RUsageInfoV4, pidrusage};
    use libproc::libproc::proc_pid::{listpidinfo, pidinfo};
    use libproc::libproc::task_info::TaskInfo;

    let pid = std::process::id() as i32;
    let task = pidinfo::<TaskInfo>(pid, 0).ok();
    let bsd = pidinfo::<BSDInfo>(pid, 0).ok();
    let usage = pidrusage::<RUsageInfoV4>(pid).ok();

    let open_fds = bsd.and_then(|info| {
        listpidinfo::<ListFDs>(pid, info.pbi_nfiles as usize)
            .ok()
            .map(|fds| fds.len() as u64)
    });

    let (virtual_memory_bytes, threads, cpu_seconds) = if let Some(task) = task {
        (
            Some(task.pti_virtual_size),
            Some(task.pti_threadnum as u64),
            Some((task.pti_total_user + task.pti_total_system) as f64 / 1_000_000_000.0),
        )
    } else {
        (None, None, None)
    };

    ProcessMetrics {
        open_fds,
        rss_bytes: usage.map(|usage| usage.ri_resident_size),
        virtual_memory_bytes,
        threads,
        cpu_seconds,
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn collect_process_metrics() -> ProcessMetrics {
    ProcessMetrics::default()
}

fn parse_status_u64(input: &str, key: &str) -> Option<u64> {
    let line = input.lines().find(|line| line.starts_with(key))?;
    line.split_whitespace().nth(1)?.parse::<u64>().ok()
}
