#![no_main]

use libfuzzer_sys::fuzz_target;
use oxicrab::config::Config;

fuzz_target!(|data: &[u8]| {
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) {
        let _ = serde_json::from_value::<Config>(v);
    }
});
