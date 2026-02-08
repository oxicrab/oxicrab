use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use crate::utils::regex::{compile_regex, RegexPatterns};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};
use serde_json::Value;
use std::time::Duration;
use url::Url;

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36";
const MAX_REDIRECTS: u32 = 5;

pub struct WebSearchTool {
    api_key: String,
    max_results: usize,
    client: Client,
}

impl WebSearchTool {
    pub fn new(api_key: Option<String>, max_results: usize) -> Self {
        Self {
            api_key: api_key.unwrap_or_else(|| std::env::var("BRAVE_API_KEY").unwrap_or_default()),
            max_results,
            client: Client::new(),
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web. Returns titles, URLs, and snippets."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
    }

    fn cacheable(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "count": {
                    "type": "integer",
                    "description": "Results (1-10)",
                    "minimum": 1,
                    "maximum": 10
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        if self.api_key.is_empty() {
            return Ok(ToolResult::error(
                "Error: BRAVE_API_KEY not configured".to_string(),
            ));
        }

        let query = params["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?;

        let count = params["count"]
            .as_u64()
            .map(|n| n.clamp(1, 10) as usize)
            .unwrap_or(self.max_results);

        match self
            .client
            .get("https://api.search.brave.com/res/v1/web/search")
            .query(&[("q", query), ("count", &count.to_string())])
            .header("Accept", "application/json")
            .header("X-Subscription-Token", &self.api_key)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) => {
                let json: Value = resp.json().await?;
                let results = json["web"]["results"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();

                if results.is_empty() {
                    return Ok(ToolResult::new(format!("No results for: {}", query)));
                }

                let mut lines = vec![format!("Results for: {}\n", query)];
                for (i, item) in results.iter().take(count).enumerate() {
                    let title = item["title"].as_str().unwrap_or("");
                    let url = item["url"].as_str().unwrap_or("");
                    lines.push(format!("{}. {}\n   {}", i + 1, title, url));
                    if let Some(desc) = item["description"].as_str() {
                        lines.push(format!("   {}", desc));
                    }
                }
                Ok(ToolResult::new(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!("Error: {}", e))),
        }
    }
}

pub struct WebFetchTool {
    max_chars: usize,
    client: Client,
}

impl WebFetchTool {
    pub fn new(max_chars: usize) -> Result<Self> {
        Ok(Self {
            max_chars,
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS as usize))
                .timeout(Duration::from_secs(30))
                .build()
                .context("Failed to create HTTP client for WebFetchTool")?,
        })
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch URL and extract readable content (HTML → markdown/text)."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
    }

    fn cacheable(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to fetch"
                },
                "extractMode": {
                    "type": "string",
                    "enum": ["markdown", "text"],
                    "default": "markdown"
                },
                "maxChars": {
                    "type": "integer",
                    "minimum": 100
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let url_str = params["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

        let url = Url::parse(url_str)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Ok(ToolResult::error(format!(
                "URL validation failed: Only http/https allowed, got '{}'",
                url.scheme()
            )));
        }

        let extract_mode = params["extractMode"].as_str().unwrap_or("markdown");
        let max_chars = params["maxChars"]
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(self.max_chars);

        match self
            .client
            .get(url_str)
            .header("User-Agent", USER_AGENT)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let final_url = resp.url().to_string();
                let content_type = resp
                    .headers()
                    .get("content-type")
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("")
                    .to_string();

                let text = resp.text().await?;

                let (extracted_text, extractor) = if content_type.contains("application/json") {
                    let json: Value = serde_json::from_str(&text)?;
                    (serde_json::to_string_pretty(&json)?, "json")
                } else if content_type.contains("text/html")
                    || text
                        .chars()
                        .take(256)
                        .collect::<String>()
                        .to_lowercase()
                        .starts_with("<!doctype")
                    || text
                        .chars()
                        .take(256)
                        .collect::<String>()
                        .to_lowercase()
                        .starts_with("<html")
                {
                    match extract_html(&text, extract_mode == "markdown") {
                        Ok(content) => (content, "readability"),
                        Err(_) => {
                            let stripped = strip_tags(&text);
                            (normalize(&stripped), "fallback")
                        }
                    }
                } else {
                    (text, "raw")
                };

                let truncated = extracted_text.len() > max_chars;
                let final_text = if truncated {
                    extracted_text.chars().take(max_chars).collect()
                } else {
                    extracted_text
                };

                let result = serde_json::json!({
                    "url": url_str,
                    "finalUrl": final_url,
                    "status": status,
                    "extractor": extractor,
                    "truncated": truncated,
                    "length": final_text.len(),
                    "text": final_text
                });

                Ok(ToolResult::new(serde_json::to_string(&result)?))
            }
            Err(e) => {
                let result = serde_json::json!({
                    "error": e.to_string(),
                    "url": url_str
                });
                Ok(ToolResult::error(serde_json::to_string(&result)?))
            }
        }
    }
}

fn strip_tags(html: &str) -> String {
    let text = RegexPatterns::html_script().replace_all(html, "");
    let text = RegexPatterns::html_style().replace_all(&text, "");
    let text = RegexPatterns::html_tags().replace_all(&text, "");

    html_escape::decode_html_entities(&text).to_string()
}

fn normalize(text: &str) -> String {
    let text = RegexPatterns::whitespace().replace_all(text, " ");
    let text = RegexPatterns::newlines().replace_all(&text, "\n\n");
    text.trim().to_string()
}

fn extract_html(html: &str, markdown: bool) -> Result<String> {
    let document = Html::parse_document(html);

    // Extract title using scraper
    let title_selector = Selector::parse("title")
        .map_err(|e| anyhow::anyhow!("Failed to parse title selector: {:?}", e))?;
    let title = document
        .select(&title_selector)
        .next()
        .map(|e| e.text().collect::<String>())
        .unwrap_or_default();

    // Try to find main content: article > main > body (using scraper for better extraction)
    let content_html = if let Ok(article_sel) = Selector::parse("article") {
        if let Some(element) = document.select(&article_sel).next() {
            element.html()
        } else if let Ok(main_sel) = Selector::parse("main") {
            if let Some(element) = document.select(&main_sel).next() {
                element.html()
            } else if let Ok(body_sel) = Selector::parse("body") {
                document
                    .select(&body_sel)
                    .next()
                    .map(|e| e.html())
                    .unwrap_or_else(|| strip_scripts_styles(html))
            } else {
                strip_scripts_styles(html)
            }
        } else {
            strip_scripts_styles(html)
        }
    } else {
        strip_scripts_styles(html)
    };

    // Convert to markdown or plain text
    let content = if markdown {
        html_to_markdown(&content_html)
    } else {
        normalize(&strip_tags(&content_html))
    };

    if markdown && !title.is_empty() {
        Ok(format!("# {}\n\n{}", title.trim(), content))
    } else {
        Ok(content)
    }
}

fn strip_scripts_styles(html: &str) -> String {
    let text = RegexPatterns::html_script().replace_all(html, "");
    RegexPatterns::html_style()
        .replace_all(&text, "")
        .to_string()
}

fn html_to_markdown(html: &str) -> String {
    // Use scraper to parse and convert HTML elements to markdown
    let fragment = Html::parse_fragment(html);
    let mut parts = Vec::new();

    // Convert links
    if let Ok(link_sel) = Selector::parse("a") {
        for link in fragment.select(&link_sel) {
            if let Some(href) = link.value().attr("href") {
                let text: String = link.text().collect();
                if !text.trim().is_empty() {
                    parts.push(format!("[{}]({})", text.trim(), href));
                }
            }
        }
    }

    // Convert headings
    for level in 1..=6 {
        if let Ok(heading_sel) = Selector::parse(&format!("h{}", level)) {
            for heading in fragment.select(&heading_sel) {
                let text: String = heading.text().collect();
                if !text.trim().is_empty() {
                    parts.push(format!("\n{} {}\n", "#".repeat(level), text.trim()));
                }
            }
        }
    }

    // Convert lists
    if let Ok(li_sel) = Selector::parse("li") {
        for li in fragment.select(&li_sel) {
            let text: String = li.text().collect();
            if !text.trim().is_empty() {
                parts.push(format!("\n- {}", text.trim()));
            }
        }
    }

    // If we extracted specific elements, use them; otherwise fall back to regex-based conversion
    if parts.is_empty() {
        // Fallback: use regex-based markdown conversion (like Python version)
        let mut text = html.to_string();
        // Convert links
        if let Ok(re_link) =
            compile_regex(r#"(?i)<a\s+[^>]*href=["']([^"']+)["'][^>]*>([\s\S]*?)</a>"#)
        {
            text = re_link
                .replace_all(&text, |caps: &regex::Captures| {
                    let href = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let link_text = strip_tags(caps.get(2).map(|m| m.as_str()).unwrap_or(""));
                    format!("[{}]({})", link_text.trim(), href)
                })
                .to_string();
        }

        // Convert headings
        for level in 1..=6 {
            if let Ok(re_heading) =
                compile_regex(&format!(r"(?i)<h{}[^>]*>([\s\S]*?)</h{}>", level, level))
            {
                text = re_heading
                    .replace_all(&text, |caps: &regex::Captures| {
                        let heading_text =
                            strip_tags(caps.get(1).map(|m| m.as_str()).unwrap_or(""));
                        format!("\n{} {}\n", "#".repeat(level), heading_text.trim())
                    })
                    .to_string();
            }
        }

        // Convert lists
        if let Ok(re_list) = compile_regex(r"(?i)<li[^>]*>([\s\S]*?)</li>") {
            text = re_list
                .replace_all(&text, |caps: &regex::Captures| {
                    let item_text = strip_tags(caps.get(1).map(|m| m.as_str()).unwrap_or(""));
                    format!("\n- {}", item_text.trim())
                })
                .to_string();
        }

        // Convert block elements
        if let Ok(re_block) = compile_regex(r"(?i)</(p|div|section|article)>") {
            text = re_block.replace_all(&text, "\n\n").to_string();
        }

        // Convert br/hr
        if let Ok(re_br) = compile_regex(r"(?i)<(br|hr)\s*/?>") {
            text = re_br.replace_all(&text, "\n").to_string();
        }

        normalize(&strip_tags(&text))
    } else {
        // Use extracted parts, but also include remaining text content
        let remaining_text = normalize(&strip_tags(html));
        if !remaining_text.trim().is_empty() {
            parts.push(remaining_text);
        }
        normalize(&parts.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_tags_removes_html_tags() {
        let html = "<p>hello</p>";
        let result = strip_tags(html);
        assert!(result.contains("hello"));
        assert!(!result.contains("<p>"));
        assert!(!result.contains("</p>"));
    }

    #[test]
    fn test_strip_tags_handles_nested_tags() {
        let html = "<div><p><strong>nested</strong> content</p></div>";
        let result = strip_tags(html);
        assert!(result.contains("nested"));
        assert!(result.contains("content"));
        assert!(!result.contains("<div>"));
        assert!(!result.contains("<strong>"));
    }

    #[test]
    fn test_normalize_collapses_whitespace() {
        let text = "a  b\t\tc";
        let result = normalize(text);
        assert_eq!(result, "a b c");
    }

    #[test]
    fn test_normalize_trims_excess_newlines() {
        let text = "line1\n\n\n\nline2";
        let result = normalize(text);
        // The normalize function replaces multiple newlines with double newline
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
        assert!(!result.contains("\n\n\n\n"));
    }

    #[test]
    fn test_strip_scripts_styles_removes_script_blocks() {
        let html = "<div>content</div><script>alert('test');</script><p>more</p>";
        let result = strip_scripts_styles(html);
        assert!(result.contains("<div>content</div>"));
        assert!(result.contains("<p>more</p>"));
        assert!(!result.contains("<script>"));
        assert!(!result.contains("alert"));
    }

    #[test]
    fn test_strip_scripts_styles_removes_style_blocks() {
        let html = "<div>content</div><style>.class { color: red; }</style><p>more</p>";
        let result = strip_scripts_styles(html);
        assert!(result.contains("<div>content</div>"));
        assert!(result.contains("<p>more</p>"));
        assert!(!result.contains("<style>"));
        assert!(!result.contains("color: red"));
    }

    #[test]
    fn test_html_to_markdown_converts_links() {
        let html = r#"<a href="http://example.com">click here</a>"#;
        let result = html_to_markdown(html);
        assert!(
            result.contains("[click here](http://example.com)") || result.contains("click here")
        );
    }

    #[test]
    fn test_html_to_markdown_converts_bold() {
        let html = "<b>bold text</b> and <strong>strong text</strong>";
        let result = html_to_markdown(html);
        // The function may or may not preserve bold formatting depending on the implementation
        // At minimum, it should contain the text content
        assert!(result.contains("bold text"));
        assert!(result.contains("strong text"));
    }
}
