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
        if config.gentle_threshold > config.firm_threshold {
            tracing::warn!(
                "cognitive gentle_threshold ({}) > firm_threshold ({}), swapping",
                config.gentle_threshold,
                config.firm_threshold
            );
            std::mem::swap(&mut config.gentle_threshold, &mut config.firm_threshold);
        }
        if config.firm_threshold > config.urgent_threshold {
            tracing::warn!(
                "cognitive firm_threshold ({}) > urgent_threshold ({}), swapping",
                config.firm_threshold,
                config.urgent_threshold
            );
            std::mem::swap(&mut config.firm_threshold, &mut config.urgent_threshold);
        }
        // Re-check gentle after firm/urgent swap
        if config.gentle_threshold > config.firm_threshold {
            std::mem::swap(&mut config.gentle_threshold, &mut config.firm_threshold);
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
        self.total_tool_calls += names.len() as u32;
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
        format!(
            "[Cognitive state] {} tool calls since last checkpoint. Recent tools: [{}]",
            self.total_tool_calls,
            recent.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(enabled: bool) -> CognitiveConfig {
        CognitiveConfig {
            enabled,
            gentle_threshold: 3,
            firm_threshold: 5,
            urgent_threshold: 8,
            recent_tools_window: 4,
        }
    }

    #[test]
    fn test_no_pressure_below_threshold() {
        let mut tracker = CheckpointTracker::new(test_config(true));
        tracker.record_tool_calls(&["shell", "read_file"]);
        assert!(tracker.pressure_message().is_none());
    }

    #[test]
    fn test_gentle_escalation() {
        let mut tracker = CheckpointTracker::new(test_config(true));
        tracker.record_tool_calls(&["a", "b", "c"]);
        let msg = tracker.pressure_message().unwrap();
        assert!(msg.contains("hint"));
    }

    #[test]
    fn test_firm_escalation() {
        let mut tracker = CheckpointTracker::new(test_config(true));
        tracker.record_tool_calls(&["a", "b", "c", "d", "e"]);
        // First call returns gentle (threshold 3 already crossed)
        let msg1 = tracker.pressure_message().unwrap();
        assert!(msg1.contains("hint"));
        // Second call returns firm (threshold 5 crossed)
        let msg2 = tracker.pressure_message().unwrap();
        assert!(msg2.contains("warning"));
    }

    #[test]
    fn test_urgent_escalation() {
        let mut tracker = CheckpointTracker::new(test_config(true));
        tracker.record_tool_calls(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        let _ = tracker.pressure_message(); // gentle
        let _ = tracker.pressure_message(); // firm
        let msg = tracker.pressure_message().unwrap();
        assert!(msg.contains("URGENT"));
    }

    #[test]
    fn test_no_repeat_same_level() {
        let mut tracker = CheckpointTracker::new(test_config(true));
        tracker.record_tool_calls(&["a", "b", "c"]);
        assert!(tracker.pressure_message().is_some()); // gentle
        assert!(tracker.pressure_message().is_none()); // already emitted, below firm
    }

    #[test]
    fn test_reset_clears_state() {
        let mut tracker = CheckpointTracker::new(test_config(true));
        tracker.record_tool_calls(&["a", "b", "c", "d", "e"]);
        let _ = tracker.pressure_message(); // gentle
        let _ = tracker.pressure_message(); // firm
        tracker.reset();
        assert_eq!(tracker.total_tool_calls, 0);
        assert!(tracker.recent_tools.is_empty());
        // After reset, thresholds fire again
        tracker.record_tool_calls(&["x", "y", "z"]);
        let msg = tracker.pressure_message().unwrap();
        assert!(msg.contains("hint"));
    }

    #[test]
    fn test_rolling_window_cap() {
        let mut tracker = CheckpointTracker::new(test_config(true));
        tracker.record_tool_calls(&["a", "b", "c", "d", "e", "f"]);
        // Window is 4, so only last 4 remain
        assert_eq!(tracker.recent_tools.len(), 4);
        assert_eq!(tracker.recent_tools[0], "c");
    }

    #[test]
    fn test_disabled_config() {
        let mut tracker = CheckpointTracker::new(test_config(false));
        tracker.record_tool_calls(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]);
        assert!(tracker.pressure_message().is_none());
    }

    #[test]
    fn test_breadcrumb_format() {
        let mut tracker = CheckpointTracker::new(test_config(true));
        tracker.record_tool_calls(&["shell", "read_file"]);
        let crumb = tracker.breadcrumb();
        assert!(crumb.contains("2 tool calls"));
        assert!(crumb.contains("shell"));
        assert!(crumb.contains("read_file"));
    }
}
