#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use oxicrab::fuzz_api::validate_webhook_signature;

#[derive(Arbitrary, Debug)]
struct Input {
    secret: String,
    signature: String,
    body: Vec<u8>,
}

fuzz_target!(|input: Input| {
    let _ = validate_webhook_signature(&input.secret, &input.signature, &input.body);
});
