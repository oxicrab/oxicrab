use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use crate::utils::media::{extension_from_content_type, save_media_file};
use crate::utils::regex::{RegexPatterns, compile_regex};
#[cfg(test)]
use anyhow::Context;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};
use serde_json::Value;
use std::time::Duration;

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36";
#[cfg(test)]
const MAX_REDIRECTS: u32 = 5;

pub struct WebSearchTool {
    provider: String,
    api_key: String,
    max_results: usize,
    client: Client,
}

impl WebSearchTool {
    pub fn new(api_key: Option<String>, max_results: usize) -> Self {
        Self {
            provider: "brave".to_string(),
            api_key: api_key.unwrap_or_else(|| std::env::var("BRAVE_API_KEY").unwrap_or_default()),
            max_results,
            client: crate::utils::http::default_http_client(),
        }
    }

    pub fn from_config(config: &crate::config::WebSearchConfig) -> Self {
        let api_key = if config.api_key.is_empty() {
            std::env::var("BRAVE_API_KEY").unwrap_or_default()
        } else {
            config.api_key.clone()
        };
        Self {
            provider: config.provider.clone(),
            api_key,
            max_results: config.max_results,
            client: crate::utils::http::default_http_client(),
        }
    }

    /// Fallback search using `DuckDuckGo` HTML when no Brave API key is configured.
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
                let html = crate::utils::http::limited_text(
                    resp,
                    crate::utils::http::DEFAULT_MAX_BODY_BYTES,
                )
                .await?;
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
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn description(&self) -> &'static str {
        "Search the web. Returns titles, URLs, and snippets. Uses Brave Search if API key is configured, otherwise falls back to DuckDuckGo."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
    }

    fn cacheable(&self) -> bool {
        true
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::Full,
            actions: vec![],
        }
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

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let query = params["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?;

        let count = params["count"]
            .as_u64()
            .map_or(self.max_results, |n| n.clamp(1, 10) as usize);

        // Use DuckDuckGo if explicitly configured or if no Brave API key
        if self.provider == "duckduckgo" || self.api_key.is_empty() {
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
            Err(e) => Ok(ToolResult::error(format!("search failed: {}", e))),
        }
    }
}

pub struct WebFetchTool {
    max_chars: usize,
    /// Only used by test helpers (`fetch_url`); production path builds a
    /// per-request pinned client in `execute()`.
    #[cfg(test)]
    client: Client,
}

impl WebFetchTool {
    pub fn new(max_chars: usize) -> Result<Self> {
        Ok(Self {
            max_chars,
            #[cfg(test)]
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS as usize))
                .timeout(Duration::from_secs(30))
                .build()
                .context("Failed to create HTTP client for WebFetchTool")?,
        })
    }

    /// Core fetch logic (without SSRF validation).
    /// Separated from `execute()` so tests can call it directly with wiremock URLs.
    #[cfg(test)]
    async fn fetch_url(&self, params: &Value) -> Result<ToolResult> {
        self.fetch_url_with_client(params, &self.client).await
    }

    async fn fetch_url_with_client(&self, params: &Value, client: &Client) -> Result<ToolResult> {
        let url_str = params["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

        let extract_mode = params["extractMode"].as_str().unwrap_or("markdown");
        let max_chars = params["maxChars"]
            .as_u64()
            .map_or(self.max_chars, |n| n as usize);

        match client
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

                // Handle binary content (images, etc.) — save to disk
                if let Some(ext) = extension_from_content_type(&content_type) {
                    let (bytes, _truncated) = crate::utils::http::limited_body(
                        resp,
                        crate::utils::http::DEFAULT_MAX_BODY_BYTES,
                    )
                    .await?;
                    match save_media_file(&bytes, "fetch", ext) {
                        Ok(path) => {
                            let result = serde_json::json!({
                                "url": url_str,
                                "finalUrl": final_url,
                                "status": status,
                                "mediaPath": path,
                                "mediaSize": bytes.len(),
                                "contentType": content_type,
                            });
                            return Ok(ToolResult::new(serde_json::to_string(&result)?));
                        }
                        Err(e) => {
                            return Ok(ToolResult::error(format!(
                                "failed to save media from {}: {}",
                                url_str, e
                            )));
                        }
                    }
                }

                let text = crate::utils::http::limited_text(
                    resp,
                    crate::utils::http::DEFAULT_MAX_BODY_BYTES,
                )
                .await?;

                let (extracted_text, extractor) = if content_type.contains("application/json") {
                    match serde_json::from_str::<Value>(&text) {
                        Ok(json) => (
                            serde_json::to_string_pretty(&json).unwrap_or_else(|_| text.clone()),
                            "json",
                        ),
                        Err(_) => (text.clone(), "raw"),
                    }
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
                    if let Ok(content) = extract_html(&text, extract_mode == "markdown") {
                        (content, "readability")
                    } else {
                        let stripped = strip_tags(&text);
                        (normalize(&stripped), "fallback")
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
    fn name(&self) -> &'static str {
        "web_fetch"
    }

    fn description(&self) -> &'static str {
        "Fetch URL and extract readable content (HTML → markdown/text)."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
    }

    fn cacheable(&self) -> bool {
        true
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::Full,
            actions: vec![],
        }
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

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let url_str = params["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

        // Validate URL and resolve DNS for pinning (prevents TOCTOU rebinding)
        let resolved = match crate::utils::url_security::validate_and_resolve(url_str).await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::error(e)),
        };

        // Build a pinned client to prevent DNS rebinding.
        // Redirects are disabled: an attacker could redirect to an internal IP.
        let pinned = {
            let mut builder = Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30));
            for addr in &resolved.addrs {
                builder = builder.resolve(&resolved.host, *addr);
            }
            match builder.build() {
                Ok(c) => c,
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "failed to build pinned HTTP client: {}",
                        e
                    )));
                }
            }
        };

        self.fetch_url_with_client(&params, &pinned).await
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
                    .map_or_else(|| strip_scripts_styles(html), |e| e.html())
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
                    let href = caps.get(1).map_or("", |m| m.as_str());
                    let link_text = strip_tags(caps.get(2).map_or("", |m| m.as_str()));
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
                        let heading_text = strip_tags(caps.get(1).map_or("", |m| m.as_str()));
                        format!("\n{} {}\n", "#".repeat(level), heading_text.trim())
                    })
                    .to_string();
            }
        }

        // Convert lists
        if let Ok(re_list) = compile_regex(r"(?i)<li[^>]*>([\s\S]*?)</li>") {
            text = re_list
                .replace_all(&text, |caps: &regex::Captures| {
                    let item_text = strip_tags(caps.get(1).map_or("", |m| m.as_str()));
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
mod tests;
