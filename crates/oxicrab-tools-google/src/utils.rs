//! Private utilities for Google tools.

use regex::Regex;
use std::sync::LazyLock;

pub use oxicrab_core::utils::http::default_http_client;

/// Regex for matching HTML tags.
pub fn html_tags_regex() -> &'static Regex {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<[^>]+>").expect("Failed to compile HTML tags regex"));
    &RE
}

/// Strip `<style>...</style>` blocks (including content).
fn style_block_regex() -> &'static Regex {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<style[^>]*>.*?</style>").expect("style block regex"));
    &RE
}

/// Strip `<script>...</script>` blocks (including content).
fn script_block_regex() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?is)<script[^>]*>.*?</script>").expect("script block regex")
    });
    &RE
}

/// Strip HTML to plain text: removes `<style>` and `<script>` blocks entirely
/// (including their content), then strips remaining HTML tags (replacing with
/// spaces to preserve word boundaries).
pub fn strip_html_to_text(html: &str) -> String {
    let without_style = style_block_regex().replace_all(html, "");
    let without_script = script_block_regex().replace_all(&without_style, "");
    html_tags_regex()
        .replace_all(&without_script, " ")
        .to_string()
}
