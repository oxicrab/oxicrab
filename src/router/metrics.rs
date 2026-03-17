/// Router observability hooks backed by the `metrics` facade.
///
/// Use an external recorder/exporter (e.g. Prometheus/OpenTelemetry) to collect
/// and aggregate these signals.
pub fn record_direct_dispatch() {
    metrics::counter!("router_route_decision_total", "decision" => "direct_dispatch").increment(1);
}

pub fn record_guided_llm() {
    metrics::counter!("router_route_decision_total", "decision" => "guided_llm").increment(1);
}

pub fn record_semantic_filter() {
    metrics::counter!("router_route_decision_total", "decision" => "semantic_filter").increment(1);
}

pub fn record_full_llm() {
    metrics::counter!("router_route_decision_total", "decision" => "full_llm").increment(1);
}

pub fn record_blocked_tool_attempt() {
    metrics::counter!("router_blocked_tool_attempt_total").increment(1);
}

pub fn record_policy_drift() {
    metrics::counter!("router_policy_drift_total").increment(1);
}

/// Record semantic selection proxy quality against executed tools for one turn.
///
/// Precision = hits / used, Recall = hits / allowed.
pub fn record_semantic_turn_proxy_quality(allowed_tools: &[String], used_tools: &[String]) {
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
    let precision = if used.is_empty() {
        0.0
    } else {
        hits as f64 / used.len() as f64
    };
    let recall = if allowed.is_empty() {
        0.0
    } else {
        hits as f64 / allowed.len() as f64
    };

    metrics::counter!("router_semantic_turn_total").increment(1);
    if hits == 0 {
        metrics::counter!("router_semantic_zero_hit_turn_total").increment(1);
    }
    metrics::histogram!("router_semantic_proxy_precision").record(precision);
    metrics::histogram!("router_semantic_proxy_recall").record(recall);
    metrics::counter!("router_semantic_proxy_hits_total").increment(hits);
    metrics::counter!("router_semantic_proxy_used_total").increment(used.len() as u64);
    metrics::counter!("router_semantic_proxy_allowed_total").increment(allowed.len() as u64);
}

pub fn record_semantic_low_confidence_fallback() {
    metrics::counter!("router_semantic_low_confidence_fallback_total").increment(1);
}

/// Record score distribution input for external histogram aggregation.
pub fn record_semantic_scores(scores: &[f32]) {
    for score in scores {
        metrics::histogram!("router_semantic_score").record(f64::from(*score));
    }
}
