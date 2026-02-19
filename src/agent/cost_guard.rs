use crate::config::CostGuardConfig;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tracing::{info, warn};

/// Per-million-token pricing for a model.
#[derive(Debug, Clone)]
pub struct ModelCost {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

/// How to match a model name against a pricing entry.
#[derive(Debug, Clone)]
enum ModelMatcher {
    StartsWith(String),
}

/// Embedded pricing snapshot covering common models.
const PRICING_DATA: &str = include_str!("pricing_data.json");

/// Default pricing for unknown models ($10 input / $30 output per 1M tokens).
const DEFAULT_INPUT_PER_MILLION: f64 = 10.0;
const DEFAULT_OUTPUT_PER_MILLION: f64 = 30.0;

struct DailyCost {
    total_cents: f64,
    date: chrono::NaiveDate,
}

pub struct CostGuard {
    config: CostGuardConfig,
    budget_exceeded: AtomicBool,
    daily_cost: Mutex<DailyCost>,
    hourly_actions: Mutex<VecDeque<Instant>>,
    /// Parsed pricing lookup: config overrides first, then embedded data.
    pricing_lookup: Vec<(ModelMatcher, ModelCost)>,
}

impl CostGuard {
    pub fn new(config: CostGuardConfig) -> Self {
        let mut pricing_lookup = Vec::new();

        // Config overrides take priority
        for (pattern, cost) in &config.model_costs {
            pricing_lookup.push((
                ModelMatcher::StartsWith(pattern.clone()),
                ModelCost {
                    input_per_million: cost.input_per_million,
                    output_per_million: cost.output_per_million,
                },
            ));
        }

        // Parse embedded pricing data
        if let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(PRICING_DATA) {
            for entry in entries {
                let pattern = entry["pattern"].as_str().unwrap_or_default();
                let input = entry["input_mtok"]
                    .as_f64()
                    .unwrap_or(DEFAULT_INPUT_PER_MILLION);
                let output = entry["output_mtok"]
                    .as_f64()
                    .unwrap_or(DEFAULT_OUTPUT_PER_MILLION);
                if !pattern.is_empty() {
                    pricing_lookup.push((
                        ModelMatcher::StartsWith(pattern.to_string()),
                        ModelCost {
                            input_per_million: input,
                            output_per_million: output,
                        },
                    ));
                }
            }
        } else {
            warn!("failed to parse embedded pricing data");
        }

        Self {
            config,
            budget_exceeded: AtomicBool::new(false),
            daily_cost: Mutex::new(DailyCost {
                total_cents: 0.0,
                date: chrono::Utc::now().date_naive(),
            }),
            hourly_actions: Mutex::new(VecDeque::new()),
            pricing_lookup,
        }
    }

    /// Pre-flight check before an LLM call. Returns `Err(message)` if blocked.
    pub fn check_allowed(&self) -> Result<(), String> {
        // Fast-path: if budget was already exceeded, skip the mutex unless it's a new day
        if self.budget_exceeded.load(Ordering::Acquire) && self.config.daily_budget_cents.is_some()
        {
            // Still need to check for date rollover — take the lock
            let Ok(daily) = self.daily_cost.lock() else {
                return Ok(());
            };
            let today = chrono::Utc::now().date_naive();
            if daily.date == today {
                return Err(format!(
                    "Daily budget exceeded ({:.1} cents spent, limit {} cents). Try again tomorrow.",
                    daily.total_cents,
                    self.config.daily_budget_cents.unwrap_or(0)
                ));
            }
            // Day rolled over — fall through to the full check which will reset
            drop(daily);
        }

        // Check daily budget
        if let Some(budget) = self.config.daily_budget_cents {
            let Ok(mut daily) = self.daily_cost.lock() else {
                warn!("cost guard daily_cost mutex poisoned — budget enforcement bypassed");
                return Ok(());
            };
            let today = chrono::Utc::now().date_naive();
            if daily.date != today {
                // Day rolled over — reset
                daily.total_cents = 0.0;
                daily.date = today;
                self.budget_exceeded.store(false, Ordering::Release);
            } else if daily.total_cents >= budget as f64 {
                self.budget_exceeded.store(true, Ordering::Release);
                return Err(format!(
                    "Daily budget exceeded ({:.1} cents spent, limit {} cents). Try again tomorrow.",
                    daily.total_cents, budget
                ));
            }
        }

        // Check hourly rate limit
        if let Some(max_actions) = self.config.max_actions_per_hour {
            let Ok(mut actions) = self.hourly_actions.lock() else {
                warn!("cost guard hourly_actions mutex poisoned — rate limit bypassed");
                return Ok(());
            };
            // Only prune old entries if we can compute the cutoff.
            // If checked_sub fails (e.g. system just booted), keep all entries
            // so the rate limiter is not bypassed.
            if let Some(cutoff) = Instant::now().checked_sub(std::time::Duration::from_hours(1)) {
                while actions.front().is_some_and(|t| *t < cutoff) {
                    actions.pop_front();
                }
            }
            if actions.len() as u64 >= max_actions {
                return Err(format!(
                    "Hourly rate limit reached ({} actions in the last hour, limit {}). Please wait.",
                    actions.len(),
                    max_actions
                ));
            }
        }

        Ok(())
    }

    /// Record an LLM call for cost tracking and rate limiting.
    pub fn record_llm_call(
        &self,
        model: &str,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) {
        let cost_cents =
            self.estimate_cost_cents(model, input_tokens.unwrap_or(0), output_tokens.unwrap_or(0));

        // Update daily cost
        if let Ok(mut daily) = self.daily_cost.lock() {
            let today = chrono::Utc::now().date_naive();
            if daily.date != today {
                daily.total_cents = 0.0;
                daily.date = today;
                self.budget_exceeded.store(false, Ordering::Release);
            }
            daily.total_cents += cost_cents;

            if let Some(budget) = self.config.daily_budget_cents
                && daily.total_cents >= budget as f64
            {
                self.budget_exceeded.store(true, Ordering::Release);
                warn!(
                    "daily budget exceeded: {:.1} cents spent (limit: {} cents)",
                    daily.total_cents, budget
                );
            }
        }

        // Record action for rate limiting
        if self.config.max_actions_per_hour.is_some()
            && let Ok(mut actions) = self.hourly_actions.lock()
        {
            actions.push_back(Instant::now());
        }

        if cost_cents > 0.0 {
            info!(
                "LLM call cost: {:.4} cents (model={}, input={}, output={})",
                cost_cents,
                model,
                input_tokens.unwrap_or(0),
                output_tokens.unwrap_or(0)
            );
        }
    }

    /// Look up pricing for a model name.
    pub fn lookup_cost(&self, model: &str) -> ModelCost {
        for (matcher, cost) in &self.pricing_lookup {
            match matcher {
                ModelMatcher::StartsWith(prefix) => {
                    if model.starts_with(prefix.as_str()) {
                        return cost.clone();
                    }
                }
            }
        }
        // Default for unknown models
        ModelCost {
            input_per_million: DEFAULT_INPUT_PER_MILLION,
            output_per_million: DEFAULT_OUTPUT_PER_MILLION,
        }
    }

    /// Calculate cost in cents for a given model and token counts.
    pub fn estimate_cost_cents(&self, model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
        let cost = self.lookup_cost(model);
        let input_cost = (input_tokens as f64 / 1_000_000.0) * cost.input_per_million;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * cost.output_per_million;
        // Convert from dollars to cents
        (input_cost + output_cost) * 100.0
    }
}

#[cfg(test)]
mod tests {
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
        guard.record_llm_call("claude-sonnet-4-5-20250929", Some(100_000), Some(50_000));
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
        guard.record_llm_call("claude-sonnet-4-5-20250929", Some(10_000), Some(10_000));

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

        guard.record_llm_call("test-model", None, None);
        guard.record_llm_call("test-model", None, None);
        guard.record_llm_call("test-model", None, None);

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
}
