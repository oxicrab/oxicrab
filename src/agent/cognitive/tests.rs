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
