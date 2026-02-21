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
const PRICING_DATA: &str = include_str!("../pricing_data.json");

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
    ///
    /// Cache token counts are used for Anthropic prompt caching:
    /// - `cache_read_input_tokens`: billed at 10% of input rate
    /// - `cache_creation_input_tokens`: billed at 125% of input rate
    pub fn record_llm_call(
        &self,
        model: &str,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cache_creation_input_tokens: Option<u64>,
        cache_read_input_tokens: Option<u64>,
    ) {
        let cost_cents = self.estimate_cost_cents(
            model,
            input_tokens.unwrap_or(0),
            output_tokens.unwrap_or(0),
            cache_creation_input_tokens.unwrap_or(0),
            cache_read_input_tokens.unwrap_or(0),
        );

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
                "LLM call cost: {:.4} cents (model={}, input={}, output={}, cache_create={}, cache_read={})",
                cost_cents,
                model,
                input_tokens.unwrap_or(0),
                output_tokens.unwrap_or(0),
                cache_creation_input_tokens.unwrap_or(0),
                cache_read_input_tokens.unwrap_or(0),
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
    ///
    /// Cache-aware pricing (Anthropic):
    /// - `cache_read_input_tokens`: billed at 10% of input rate
    /// - `cache_creation_input_tokens`: billed at 125% of input rate
    /// - Regular `input_tokens` are billed at the standard rate
    pub fn estimate_cost_cents(
        &self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_input_tokens: u64,
        cache_read_input_tokens: u64,
    ) -> f64 {
        let cost = self.lookup_cost(model);
        let input_cost = (input_tokens as f64 / 1_000_000.0) * cost.input_per_million;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * cost.output_per_million;
        // Cache read tokens at 10% of input rate
        let cache_read_cost =
            (cache_read_input_tokens as f64 / 1_000_000.0) * cost.input_per_million * 0.1;
        // Cache creation tokens at 125% of input rate
        let cache_creation_cost =
            (cache_creation_input_tokens as f64 / 1_000_000.0) * cost.input_per_million * 1.25;
        // Convert from dollars to cents
        (input_cost + output_cost + cache_read_cost + cache_creation_cost) * 100.0
    }
}

#[cfg(test)]
mod tests;
