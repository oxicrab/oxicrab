#![no_main]

use libfuzzer_sys::fuzz_target;
use oxicrab::safety::LeakDetector;

fuzz_target!(|data: &str| {
    let detector = LeakDetector::new();
    let _ = detector.scan(data);
    let _ = detector.redact(data);
});
