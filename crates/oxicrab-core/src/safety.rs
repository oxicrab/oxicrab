/// Trait for redacting leaked secrets from text.
///
/// The gateway crate uses this trait so it can accept any leak detector
/// implementation without depending on the concrete `LeakDetector` type.
pub trait LeakRedactor: Send + Sync {
    /// Scan `text` and return a copy with any detected secrets replaced
    /// by `[REDACTED]` placeholders.
    fn redact(&self, text: &str) -> String;
}
