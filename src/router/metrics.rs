use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default, Debug, Clone, Copy)]
pub struct RouterMetricsSnapshot {
    pub direct_dispatch: u64,
    pub guided_llm: u64,
    pub semantic_filter: u64,
    pub full_llm: u64,
    pub blocked_tool_attempts: u64,
    pub policy_drift_events: u64,
    pub semantic_turns: u64,
    pub semantic_zero_hit_turns: u64,
    /// Mean precision across semantic-filtered turns as basis points [0..10000].
    pub semantic_avg_precision_bps: u64,
    /// Mean recall across semantic-filtered turns as basis points [0..10000].
    pub semantic_avg_recall_bps: u64,
    pub semantic_histogram: [u64; 10],
}

static DIRECT_DISPATCH: AtomicU64 = AtomicU64::new(0);
static GUIDED_LLM: AtomicU64 = AtomicU64::new(0);
static SEMANTIC_FILTER: AtomicU64 = AtomicU64::new(0);
static FULL_LLM: AtomicU64 = AtomicU64::new(0);
static BLOCKED_TOOL_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static POLICY_DRIFT_EVENTS: AtomicU64 = AtomicU64::new(0);
static SEMANTIC_TURNS: AtomicU64 = AtomicU64::new(0);
static SEMANTIC_ZERO_HIT_TURNS: AtomicU64 = AtomicU64::new(0);
static SEMANTIC_PRECISION_BPS_SUM: AtomicU64 = AtomicU64::new(0);
static SEMANTIC_RECALL_BPS_SUM: AtomicU64 = AtomicU64::new(0);
static SEM_BUCKETS: [AtomicU64; 10] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

pub fn record_direct_dispatch() {
    DIRECT_DISPATCH.fetch_add(1, Ordering::Relaxed);
}

pub fn record_guided_llm() {
    GUIDED_LLM.fetch_add(1, Ordering::Relaxed);
}

pub fn record_semantic_filter() {
    SEMANTIC_FILTER.fetch_add(1, Ordering::Relaxed);
}

pub fn record_full_llm() {
    FULL_LLM.fetch_add(1, Ordering::Relaxed);
}

pub fn record_blocked_tool_attempt() {
    BLOCKED_TOOL_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_policy_drift() {
    POLICY_DRIFT_EVENTS.fetch_add(1, Ordering::Relaxed);
}

/// Record semantic selection quality against executed tools for one turn.
///
/// Precision = hits / used, Recall = hits / allowed.
/// Metrics are aggregated in basis points to avoid floating-point atomics.
pub fn record_semantic_turn_quality(allowed_tools: &[String], used_tools: &[String]) {
    let helper = |name: &str| name == "add_buttons" || name == "tool_search";
    let allowed: std::collections::HashSet<&str> = allowed_tools
        .iter()
        .map(String::as_str)
        .filter(|name| !helper(name))
        .collect();
    let used: std::collections::HashSet<&str> = used_tools
        .iter()
        .map(String::as_str)
        .filter(|name| !helper(name))
        .collect();

    let hits = used.intersection(&allowed).count() as u64;
    let precision_bps = if used.is_empty() {
        0
    } else {
        ((hits * 10_000) / used.len() as u64).min(10_000)
    };
    let recall_bps = if allowed.is_empty() {
        0
    } else {
        ((hits * 10_000) / allowed.len() as u64).min(10_000)
    };

    SEMANTIC_TURNS.fetch_add(1, Ordering::Relaxed);
    if hits == 0 {
        SEMANTIC_ZERO_HIT_TURNS.fetch_add(1, Ordering::Relaxed);
    }
    SEMANTIC_PRECISION_BPS_SUM.fetch_add(precision_bps, Ordering::Relaxed);
    SEMANTIC_RECALL_BPS_SUM.fetch_add(recall_bps, Ordering::Relaxed);
}

/// Record score distribution in 10 fixed buckets over [-1.0, 1.0].
pub fn record_semantic_scores(scores: &[f32]) {
    for score in scores {
        let normalized = ((*score + 1.0) / 2.0).clamp(0.0, 1.0);
        let mut idx = (normalized * 10.0).floor() as usize;
        if idx >= 10 {
            idx = 9;
        }
        SEM_BUCKETS[idx].fetch_add(1, Ordering::Relaxed);
    }
}

pub fn snapshot() -> RouterMetricsSnapshot {
    let mut histogram = [0u64; 10];
    for (idx, bucket) in SEM_BUCKETS.iter().enumerate() {
        histogram[idx] = bucket.load(Ordering::Relaxed);
    }
    let semantic_turns = SEMANTIC_TURNS.load(Ordering::Relaxed);
    let precision_sum = SEMANTIC_PRECISION_BPS_SUM.load(Ordering::Relaxed);
    let recall_sum = SEMANTIC_RECALL_BPS_SUM.load(Ordering::Relaxed);
    RouterMetricsSnapshot {
        direct_dispatch: DIRECT_DISPATCH.load(Ordering::Relaxed),
        guided_llm: GUIDED_LLM.load(Ordering::Relaxed),
        semantic_filter: SEMANTIC_FILTER.load(Ordering::Relaxed),
        full_llm: FULL_LLM.load(Ordering::Relaxed),
        blocked_tool_attempts: BLOCKED_TOOL_ATTEMPTS.load(Ordering::Relaxed),
        policy_drift_events: POLICY_DRIFT_EVENTS.load(Ordering::Relaxed),
        semantic_turns,
        semantic_zero_hit_turns: SEMANTIC_ZERO_HIT_TURNS.load(Ordering::Relaxed),
        semantic_avg_precision_bps: if semantic_turns == 0 {
            0
        } else {
            precision_sum / semantic_turns
        },
        semantic_avg_recall_bps: if semantic_turns == 0 {
            0
        } else {
            recall_sum / semantic_turns
        },
        semantic_histogram: histogram,
    }
}
