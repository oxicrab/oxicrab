use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use crate::utils::regex::{compile_regex, RegexPatterns};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};
use serde_json::Value;
use std::time::Duration;

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

    /// Fallback search using DuckDuckGo HTML when no Brave API key is configured.
    async fn search_duckduckgo(&self, query: &str, count: usize) -> Result<ToolResult> {
        let resp = self
            .client
            .get("https://html.duckduckgo.com/html/")
            .query(&[("q", query)])
            .header("User-Agent", USER_AGENT)
            .timeout(Duration::from_secs(10))
            .send()
            .await;

        match resp {
            Ok(resp) => {
                let html = resp.text().await?;
                let document = Html::parse_document(&html);

                let result_sel = Selector::parse(".result")
                    .map_err(|e| anyhow::anyhow!("Failed to parse selector: {:?}", e))?;
                let title_sel = Selector::parse(".result__a")
                    .map_err(|e| anyhow::anyhow!("Failed to parse selector: {:?}", e))?;
                let snippet_sel = Selector::parse(".result__snippet")
                    .map_err(|e| anyhow::anyhow!("Failed to parse selector: {:?}", e))?;

                let mut lines = vec![format!("Results for: {} (via DuckDuckGo)\n", query)];
                let mut found = 0;

                for result in document.select(&result_sel) {
                    if found >= count {
                        break;
                    }

                    let title = result
                        .select(&title_sel)
                        .next()
                        .map(|e| e.text().collect::<String>())
                        .unwrap_or_default();
                    let url = result
                        .select(&title_sel)
                        .next()
                        .and_then(|e| e.value().attr("href"))
                        .unwrap_or("");
                    let snippet = result
                        .select(&snippet_sel)
                        .next()
                        .map(|e| e.text().collect::<String>())
                        .unwrap_or_default();

                    let title = title.trim();
                    let snippet = snippet.trim();
                    if title.is_empty() {
                        continue;
                    }

                    found += 1;
                    lines.push(format!("{}. {}\n   {}", found, title, url));
                    if !snippet.is_empty() {
                        lines.push(format!("   {}", snippet));
                    }
                }

                if found == 0 {
                    return Ok(ToolResult::new(format!("No results for: {}", query)));
                }

                Ok(ToolResult::new(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!("DuckDuckGo search error: {}", e))),
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web. Returns titles, URLs, and snippets. Uses Brave Search if API key is configured, otherwise falls back to DuckDuckGo."
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
        let query = params["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?;

        let count = params["count"]
            .as_u64()
            .map(|n| n.clamp(1, 10) as usize)
            .unwrap_or(self.max_results);

        if self.api_key.is_empty() {
            return self.search_duckduckgo(query, count).await;
        }

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

    /// Core fetch logic (without SSRF validation).
    /// Separated from `execute()` so tests can call it directly with wiremock URLs.
    async fn fetch_url(&self, params: &Value) -> Result<ToolResult> {
        let url_str = params["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

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

        // Validate URL scheme and block SSRF to internal networks
        if let Err(e) = crate::utils::url_security::validate_url(url_str) {
            return Ok(ToolResult::error(e));
        }

        self.fetch_url(&params).await
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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // --- Unit tests for HTML processing functions ---

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
        assert!(result.contains("bold text"));
        assert!(result.contains("strong text"));
    }

    #[test]
    fn test_html_to_markdown_converts_headings() {
        let html = "<h1>Main Title</h1><h2>Subtitle</h2><p>Body text</p>";
        let result = html_to_markdown(html);
        assert!(result.contains("Main Title"));
        assert!(result.contains("Subtitle"));
        assert!(result.contains("Body text"));
    }

    #[test]
    fn test_html_to_markdown_converts_list_items() {
        let html = "<ul><li>First</li><li>Second</li></ul>";
        let result = html_to_markdown(html);
        assert!(result.contains("First"));
        assert!(result.contains("Second"));
    }

    #[test]
    fn test_extract_html_prefers_article() {
        let html = r#"<html><body><nav>Nav stuff</nav><article><p>Article content</p></article><footer>Footer</footer></body></html>"#;
        let result = extract_html(html, false).unwrap();
        assert!(result.contains("Article content"));
    }

    #[test]
    fn test_extract_html_title_in_markdown_mode() {
        let html = "<html><head><title>My Page</title></head><body><p>Body text</p></body></html>";
        let result = extract_html(html, true).unwrap();
        assert!(result.contains("# My Page"));
    }

    #[test]
    fn test_strip_tags_decodes_entities() {
        let html = "<p>5 &gt; 3 &amp; 2 &lt; 4</p>";
        let result = strip_tags(html);
        assert!(result.contains("5 > 3 & 2 < 4"));
    }

    // --- Wiremock tests for WebFetchTool ---

    fn parse_fetch_result(result: &ToolResult) -> Value {
        serde_json::from_str(&result.content).expect("fetch result should be JSON")
    }

    #[tokio::test]
    async fn test_fetch_html_page() {
        let server = MockServer::start().await;
        let html = r#"<!DOCTYPE html><html><head><title>Test Page</title></head><body><article><p>Hello from the article</p></article></body></html>"#;
        Mock::given(method("GET"))
            .and(path("/page"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html")
                    .set_body_string(html),
            )
            .mount(&server)
            .await;

        let tool = WebFetchTool::new(50000).unwrap();
        let result = tool
            .fetch_url(&serde_json::json!({"url": format!("{}/page", server.uri())}))
            .await
            .unwrap();

        assert!(!result.is_error);
        let json = parse_fetch_result(&result);
        assert_eq!(json["status"], 200);
        assert_eq!(json["extractor"], "readability");
        assert!(json["text"]
            .as_str()
            .unwrap()
            .contains("Hello from the article"));
    }

    #[tokio::test]
    async fn test_fetch_html_extracts_in_text_mode() {
        let server = MockServer::start().await;
        let html = r#"<html><body><h1>Heading</h1><p>Paragraph text</p></body></html>"#;
        Mock::given(method("GET"))
            .and(path("/text"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html")
                    .set_body_string(html),
            )
            .mount(&server)
            .await;

        let tool = WebFetchTool::new(50000).unwrap();
        let result = tool
            .fetch_url(&serde_json::json!({
                "url": format!("{}/text", server.uri()),
                "extractMode": "text"
            }))
            .await
            .unwrap();

        let json = parse_fetch_result(&result);
        assert_eq!(json["extractor"], "readability");
        let text = json["text"].as_str().unwrap();
        assert!(text.contains("Heading"));
        assert!(text.contains("Paragraph text"));
        // In text mode, should NOT have markdown heading markers
        assert!(!text.contains("# Heading"));
    }

    #[tokio::test]
    async fn test_fetch_json_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"key": "value", "num": 42})),
            )
            .mount(&server)
            .await;

        let tool = WebFetchTool::new(50000).unwrap();
        let result = tool
            .fetch_url(&serde_json::json!({"url": format!("{}/api", server.uri())}))
            .await
            .unwrap();

        let json = parse_fetch_result(&result);
        assert_eq!(json["extractor"], "json");
        let text = json["text"].as_str().unwrap();
        // Should be pretty-printed
        assert!(text.contains("\"key\": \"value\""));
        assert!(text.contains("\"num\": 42"));
    }

    #[tokio::test]
    async fn test_fetch_raw_text() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/raw"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_string("Just plain text content"),
            )
            .mount(&server)
            .await;

        let tool = WebFetchTool::new(50000).unwrap();
        let result = tool
            .fetch_url(&serde_json::json!({"url": format!("{}/raw", server.uri())}))
            .await
            .unwrap();

        let json = parse_fetch_result(&result);
        assert_eq!(json["extractor"], "raw");
        assert_eq!(json["text"], "Just plain text content");
        assert_eq!(json["truncated"], false);
    }

    #[tokio::test]
    async fn test_fetch_truncates_long_content() {
        let server = MockServer::start().await;
        let long_text = "x".repeat(1000);
        Mock::given(method("GET"))
            .and(path("/long"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_string(&long_text),
            )
            .mount(&server)
            .await;

        let tool = WebFetchTool::new(50000).unwrap();
        let result = tool
            .fetch_url(&serde_json::json!({
                "url": format!("{}/long", server.uri()),
                "maxChars": 100
            }))
            .await
            .unwrap();

        let json = parse_fetch_result(&result);
        assert_eq!(json["truncated"], true);
        assert_eq!(json["length"], 100);
    }

    #[tokio::test]
    async fn test_fetch_uses_default_max_chars() {
        let server = MockServer::start().await;
        let text = "short text";
        Mock::given(method("GET"))
            .and(path("/short"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_string(text),
            )
            .mount(&server)
            .await;

        let tool = WebFetchTool::new(500).unwrap();
        let result = tool
            .fetch_url(&serde_json::json!({"url": format!("{}/short", server.uri())}))
            .await
            .unwrap();

        let json = parse_fetch_result(&result);
        assert_eq!(json["truncated"], false);
        assert_eq!(json["text"], "short text");
    }

    #[tokio::test]
    async fn test_fetch_html_without_content_type_header() {
        let server = MockServer::start().await;
        // No content-type header, but body starts with <!DOCTYPE
        Mock::given(method("GET"))
            .and(path("/noheader"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "<!DOCTYPE html><html><body><p>Detected as HTML</p></body></html>",
            ))
            .mount(&server)
            .await;

        let tool = WebFetchTool::new(50000).unwrap();
        let result = tool
            .fetch_url(&serde_json::json!({"url": format!("{}/noheader", server.uri())}))
            .await
            .unwrap();

        let json = parse_fetch_result(&result);
        assert_eq!(json["extractor"], "readability");
        assert!(json["text"].as_str().unwrap().contains("Detected as HTML"));
    }

    #[tokio::test]
    async fn test_fetch_html_strips_scripts() {
        let server = MockServer::start().await;
        let html =
            r#"<html><body><p>Visible text</p><script>alert('evil');</script></body></html>"#;
        Mock::given(method("GET"))
            .and(path("/scripts"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html")
                    .set_body_string(html),
            )
            .mount(&server)
            .await;

        let tool = WebFetchTool::new(50000).unwrap();
        let result = tool
            .fetch_url(&serde_json::json!({"url": format!("{}/scripts", server.uri())}))
            .await
            .unwrap();

        let json = parse_fetch_result(&result);
        let text = json["text"].as_str().unwrap();
        assert!(text.contains("Visible text"));
        assert!(!text.contains("alert"));
        assert!(!text.contains("evil"));
    }

    #[tokio::test]
    async fn test_fetch_reports_final_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/final"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_string("arrived"),
            )
            .mount(&server)
            .await;

        let tool = WebFetchTool::new(50000).unwrap();
        let url = format!("{}/final", server.uri());
        let result = tool
            .fetch_url(&serde_json::json!({"url": &url}))
            .await
            .unwrap();

        let json = parse_fetch_result(&result);
        assert_eq!(json["url"], url);
        assert_eq!(json["finalUrl"], format!("{}/final", server.uri()));
    }

    #[tokio::test]
    async fn test_fetch_reports_status_code() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/not-found"))
            .respond_with(
                ResponseTemplate::new(404)
                    .insert_header("content-type", "text/plain")
                    .set_body_string("page not found"),
            )
            .mount(&server)
            .await;

        let tool = WebFetchTool::new(50000).unwrap();
        let result = tool
            .fetch_url(&serde_json::json!({"url": format!("{}/not-found", server.uri())}))
            .await
            .unwrap();

        let json = parse_fetch_result(&result);
        assert_eq!(json["status"], 404);
    }

    #[tokio::test]
    async fn test_fetch_ssrf_blocked() {
        let tool = WebFetchTool::new(50000).unwrap();
        let result = tool
            .execute(serde_json::json!({"url": "http://127.0.0.1/secret"}))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_fetch_markdown_mode_includes_title() {
        let server = MockServer::start().await;
        let html = r#"<html><head><title>My Article</title></head><body><article><h2>Section</h2><p>Content here</p></article></body></html>"#;
        Mock::given(method("GET"))
            .and(path("/md"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html")
                    .set_body_string(html),
            )
            .mount(&server)
            .await;

        let tool = WebFetchTool::new(50000).unwrap();
        let result = tool
            .fetch_url(&serde_json::json!({
                "url": format!("{}/md", server.uri()),
                "extractMode": "markdown"
            }))
            .await
            .unwrap();

        let json = parse_fetch_result(&result);
        let text = json["text"].as_str().unwrap();
        assert!(text.contains("# My Article"));
    }
}
