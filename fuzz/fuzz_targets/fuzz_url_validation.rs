#![no_main]

use libfuzzer_sys::fuzz_target;
use oxicrab::fuzz_api::validate_and_resolve;

fuzz_target!(|data: &str| {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    // Run with a timeout so DNS lookups don't stall the fuzzer
    let _ = rt.block_on(async {
        tokio::time::timeout(std::time::Duration::from_millis(100), validate_and_resolve(data)).await
    });
});
