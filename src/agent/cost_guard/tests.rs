use super::*;
use std::collections::HashMap;

fn default_config() -> CostGuardConfig {
    CostGuardConfig {
        daily_budget_cents: None,
        max_actions_per_hour: None,
        model_costs: HashMap::new(),
    }
}

#[test]
fn test_no_limits_always_allowed() {
    let guard = CostGuard::new(default_config());
    assert!(guard.check_allowed().is_ok());
    guard.record_llm_call(
        "claude-sonnet-4-5-20250929",
        Some(100_000),
        Some(50_000),
        None,
        None,
    );
    assert!(guard.check_allowed().is_ok());
}

#[test]
fn test_daily_budget_exceeded() {
    let config = CostGuardConfig {
        daily_budget_cents: Some(1), // 1 cent budget
        max_actions_per_hour: None,
        model_costs: HashMap::new(),
    };
    let guard = CostGuard::new(config);
    assert!(guard.check_allowed().is_ok());

    // Record a call that will exceed the budget
    // claude-sonnet-4: $3/1M input, $15/1M output
    // 10000 input + 10000 output = $0.03 + $0.15 = $0.18 = 18 cents >> 1 cent budget
    guard.record_llm_call(
        "claude-sonnet-4-5-20250929",
        Some(10_000),
        Some(10_000),
        None,
        None,
    );

    assert!(guard.check_allowed().is_err());
    let err = guard.check_allowed().unwrap_err();
    assert!(err.contains("Daily budget exceeded"));
}

#[test]
fn test_daily_reset_at_midnight() {
    let config = CostGuardConfig {
        daily_budget_cents: Some(1),
        max_actions_per_hour: None,
        model_costs: HashMap::new(),
    };
    let guard = CostGuard::new(config);

    // Manually set the date to yesterday to simulate midnight rollover
    {
        let mut daily = guard.daily_cost.lock().unwrap();
        daily.total_cents = 100.0;
        daily.date = chrono::Utc::now().date_naive() - chrono::Duration::days(1);
    }
    guard.budget_exceeded.store(true, Ordering::Relaxed);

    // Should reset and allow
    assert!(guard.check_allowed().is_ok());
}

#[test]
fn test_hourly_rate_limit() {
    let config = CostGuardConfig {
        daily_budget_cents: None,
        max_actions_per_hour: Some(3),
        model_costs: HashMap::new(),
    };
    let guard = CostGuard::new(config);

    guard.record_llm_call("test-model", None, None, None, None);
    guard.record_llm_call("test-model", None, None, None, None);
    guard.record_llm_call("test-model", None, None, None, None);

    let result = guard.check_allowed();
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Hourly rate limit"));
}

#[test]
fn test_hourly_sliding_window_expiry() {
    let config = CostGuardConfig {
        daily_budget_cents: None,
        max_actions_per_hour: Some(2),
        model_costs: HashMap::new(),
    };
    let guard = CostGuard::new(config);

    // Add old entries that are already expired
    {
        let mut actions = guard.hourly_actions.lock().unwrap();
        let old = Instant::now()
            .checked_sub(std::time::Duration::from_secs(3601))
            .unwrap();
        actions.push_back(old);
        actions.push_back(old);
    }

    // Should allow since old entries are expired
    assert!(guard.check_allowed().is_ok());
}

#[test]
fn test_pricing_data_parses() {
    let entries: Vec<serde_json::Value> =
        serde_json::from_str(PRICING_DATA).expect("embedded pricing data should parse");
    assert!(!entries.is_empty(), "pricing data should have entries");
}

#[test]
fn test_lookup_known_models() {
    let guard = CostGuard::new(default_config());

    let claude_sonnet = guard.lookup_cost("claude-sonnet-4-5-20250929");
    assert!(
        (claude_sonnet.input_per_million - 3.0).abs() < 0.01,
        "expected claude-sonnet input ~$3, got {}",
        claude_sonnet.input_per_million
    );

    let gpt4o = guard.lookup_cost("gpt-4o-2024-08-06");
    assert!(
        (gpt4o.input_per_million - 2.5).abs() < 0.01,
        "expected gpt-4o input ~$2.5, got {}",
        gpt4o.input_per_million
    );

    let gemini = guard.lookup_cost("gemini-2.0-flash-001");
    assert!(
        (gemini.input_per_million - 0.10).abs() < 0.01,
        "expected gemini flash input ~$0.10, got {}",
        gemini.input_per_million
    );
}

#[test]
fn test_lookup_config_override() {
    let mut model_costs = HashMap::new();
    model_costs.insert(
        "my-custom-model".to_string(),
        crate::config::ModelCost {
            input_per_million: 1.0,
            output_per_million: 5.0,
        },
    );
    let config = CostGuardConfig {
        daily_budget_cents: None,
        max_actions_per_hour: None,
        model_costs,
    };
    let guard = CostGuard::new(config);

    let cost = guard.lookup_cost("my-custom-model-v2");
    assert!(
        (cost.input_per_million - 1.0).abs() < 0.01,
        "config override should take priority"
    );
}

#[test]
fn test_lookup_unknown_model_uses_default() {
    let guard = CostGuard::new(default_config());
    let cost = guard.lookup_cost("totally-unknown-model-xyz");
    assert!((cost.input_per_million - DEFAULT_INPUT_PER_MILLION).abs() < 0.01);
    assert!((cost.output_per_million - DEFAULT_OUTPUT_PER_MILLION).abs() < 0.01);
}

#[test]
fn test_budget_exceeded_blocks() {
    let config = CostGuardConfig {
        daily_budget_cents: Some(1),
        max_actions_per_hour: None,
        model_costs: HashMap::new(),
    };
    let guard = CostGuard::new(config);

    // Simulate spending over budget
    if let Ok(mut daily) = guard.daily_cost.lock() {
        daily.total_cents = 2.0; // exceeds 1 cent budget
    }

    let result = guard.check_allowed();
    assert!(result.is_err());
}

#[test]
fn test_cost_tracked_without_budget_limits() {
    // Even with no daily budget or rate limit, costs should be accumulated
    let guard = CostGuard::new(default_config());

    assert!(guard.daily_cost.lock().unwrap().total_cents.abs() < f64::EPSILON);

    // Record a call with known pricing (claude-sonnet-4: $3/1M input, $15/1M output)
    guard.record_llm_call(
        "claude-sonnet-4-5-20250929",
        Some(1_000_000),
        Some(100_000),
        None,
        None,
    );

    // $3.00 input + $1.50 output = $4.50 = 450 cents
    let tracked = guard.daily_cost.lock().unwrap().total_cents;
    assert!(
        (tracked - 450.0).abs() < 0.1,
        "expected ~450 cents tracked, got {:.4}",
        tracked
    );
}

// --- cache-aware pricing tests ---

#[test]
fn test_cache_read_discount() {
    let guard = CostGuard::new(default_config());
    // Use a known model: claude-sonnet-4 at $3/1M input
    // 1M cache read tokens should cost 10% of $3 = $0.30 = 30 cents
    let cost = guard.estimate_cost_cents("claude-sonnet-4-5-20250929", 0, 0, 0, 1_000_000);
    let expected = 3.0 * 0.1 * 100.0; // $0.30 = 30 cents
    assert!(
        (cost - expected).abs() < 0.01,
        "cache read should be 10% of input rate, got {:.4} expected {:.4}",
        cost,
        expected
    );
}

#[test]
fn test_cache_creation_surcharge() {
    let guard = CostGuard::new(default_config());
    // Use a known model: claude-sonnet-4 at $3/1M input
    // 1M cache creation tokens should cost 125% of $3 = $3.75 = 375 cents
    let cost = guard.estimate_cost_cents("claude-sonnet-4-5-20250929", 0, 0, 1_000_000, 0);
    let expected = 3.0 * 1.25 * 100.0; // $3.75 = 375 cents
    assert!(
        (cost - expected).abs() < 0.01,
        "cache creation should be 125% of input rate, got {:.4} expected {:.4}",
        cost,
        expected
    );
}

#[test]
fn test_cache_tokens_combined_with_regular() {
    let guard = CostGuard::new(default_config());
    // 500k regular input + 500k cache read + 200k cache creation + 100k output
    // at claude-sonnet-4: $3/1M input, $15/1M output
    let cost = guard.estimate_cost_cents(
        "claude-sonnet-4-5-20250929",
        500_000,
        100_000,
        200_000,
        500_000,
    );
    let input_cost = 0.5 * 3.0; // $1.50
    let output_cost = 0.1 * 15.0; // $1.50
    let cache_read_cost = 0.5 * 3.0 * 0.1; // $0.15
    let cache_creation_cost = 0.2 * 3.0 * 1.25; // $0.75
    let expected_cents = (input_cost + output_cost + cache_read_cost + cache_creation_cost) * 100.0;
    assert!(
        (cost - expected_cents).abs() < 0.1,
        "combined cost mismatch: got {:.4} expected {:.4}",
        cost,
        expected_cents
    );
}
