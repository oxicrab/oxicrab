use crate::config::CognitiveConfig;
use std::collections::VecDeque;

/// Tracks tool call volume during an agent loop run and emits escalating
/// pressure messages that nudge the LLM to self-checkpoint its progress.
///
/// Local to each `run_agent_loop()` invocation (not persisted).
pub struct CheckpointTracker {
    config: CognitiveConfig,
    total_tool_calls: u32,
    recent_tools: VecDeque<String>,
    /// Tracks which pressure levels have already been emitted so each fires only once.
    emitted_gentle: bool,
    emitted_firm: bool,
    emitted_urgent: bool,
}

impl CheckpointTracker {
    pub fn new(mut config: CognitiveConfig) -> Self {
        // Ensure thresholds are properly ordered: gentle <= firm <= urgent
        let mut sorted = [
            config.gentle_threshold,
            config.firm_threshold,
            config.urgent_threshold,
        ];
        sorted.sort_unstable();
        if sorted[0] != config.gentle_threshold
            || sorted[1] != config.firm_threshold
            || sorted[2] != config.urgent_threshold
        {
            tracing::warn!(
                "cognitive thresholds reordered: gentle={}, firm={}, urgent={}",
                sorted[0],
                sorted[1],
                sorted[2]
            );
            config.gentle_threshold = sorted[0];
            config.firm_threshold = sorted[1];
            config.urgent_threshold = sorted[2];
        }

        Self {
            config,
            total_tool_calls: 0,
            recent_tools: VecDeque::new(),
            emitted_gentle: false,
            emitted_firm: false,
            emitted_urgent: false,
        }
    }

    /// Record one or more tool calls, incrementing the counter and maintaining
    /// a rolling window of recent tool names.
    pub fn record_tool_calls(&mut self, names: &[&str]) {
        self.total_tool_calls = self.total_tool_calls.saturating_add(names.len() as u32);
        for name in names {
            self.recent_tools.push_back((*name).to_string());
        }
        // Cap the rolling window
        while self.recent_tools.len() > self.config.recent_tools_window {
            self.recent_tools.pop_front();
        }
    }

    /// Reset counters (called when a periodic checkpoint fires).
    pub fn reset(&mut self) {
        self.total_tool_calls = 0;
        self.recent_tools.clear();
        self.emitted_gentle = false;
        self.emitted_firm = false;
        self.emitted_urgent = false;
    }

    /// Returns an escalating pressure message if a new threshold has been crossed,
    /// or `None` if disabled / below threshold / already emitted at this level.
    ///
    /// Checks from lowest to highest so that levels escalate naturally even when
    /// multiple thresholds are crossed in a single batch of tool calls.
    pub fn pressure_message(&mut self) -> Option<String> {
        if !self.config.enabled {
            return None;
        }

        if self.total_tool_calls >= self.config.gentle_threshold && !self.emitted_gentle {
            self.emitted_gentle = true;
            Some(
                "[Cognitive checkpoint hint] You have been working for a while. Consider briefly \
                 noting what you've accomplished and what remains."
                    .to_string(),
            )
        } else if self.total_tool_calls >= self.config.firm_threshold && !self.emitted_firm {
            self.emitted_firm = true;
            Some(
                "[Cognitive checkpoint warning] You have made many tool calls since your last \
                 checkpoint. Please pause and write a brief checkpoint: what you've accomplished \
                 so far and what's next."
                    .to_string(),
            )
        } else if self.total_tool_calls >= self.config.urgent_threshold && !self.emitted_urgent {
            self.emitted_urgent = true;
            Some(
                "[Cognitive checkpoint URGENT] You have made a large number of tool calls \
                 without summarizing progress. STOP and write a detailed progress summary NOW: \
                 what is done, what failed, what remains."
                    .to_string(),
            )
        } else {
            None
        }
    }

    /// Produce a cognitive state summary suitable for injection into compaction
    /// recovery context.
    pub fn breadcrumb(&self) -> String {
        let recent: Vec<&str> = self.recent_tools.iter().map(String::as_str).collect();
        let pressure = if self.emitted_urgent {
            "urgent"
        } else if self.emitted_firm {
            "firm"
        } else if self.emitted_gentle {
            "gentle"
        } else {
            "none"
        };
        format!(
            "[Cognitive state] {} tool calls, pressure: {}. Recent tools: [{}]",
            self.total_tool_calls,
            pressure,
            recent.join(", ")
        )
    }
}

#[cfg(test)]
mod tests;
