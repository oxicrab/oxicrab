use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default, Debug, Clone, Copy)]
pub struct RouterMetricsSnapshot {
    pub direct_dispatch: u64,
    pub guided_llm: u64,
    pub semantic_filter: u64,
    pub full_llm: u64,
    pub blocked_tool_attempts: u64,
    pub semantic_histogram: [u64; 10],
}

static DIRECT_DISPATCH: AtomicU64 = AtomicU64::new(0);
static GUIDED_LLM: AtomicU64 = AtomicU64::new(0);
static SEMANTIC_FILTER: AtomicU64 = AtomicU64::new(0);
static FULL_LLM: AtomicU64 = AtomicU64::new(0);
static BLOCKED_TOOL_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
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
    RouterMetricsSnapshot {
        direct_dispatch: DIRECT_DISPATCH.load(Ordering::Relaxed),
        guided_llm: GUIDED_LLM.load(Ordering::Relaxed),
        semantic_filter: SEMANTIC_FILTER.load(Ordering::Relaxed),
        full_llm: FULL_LLM.load(Ordering::Relaxed),
        blocked_tool_attempts: BLOCKED_TOOL_ATTEMPTS.load(Ordering::Relaxed),
        semantic_histogram: histogram,
    }
}
