#![no_main]

use libfuzzer_sys::fuzz_target;
use oxicrab::safety::PromptGuard;

fuzz_target!(|data: &str| {
    let guard = PromptGuard::new();
    let _ = guard.scan(data);
    let _ = guard.should_block(data);
});
