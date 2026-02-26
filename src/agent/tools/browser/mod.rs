use crate::actions;
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult};
use crate::utils::media::save_media_file;
use anyhow::Result;
use async_trait::async_trait;
use chromiumoxide::Page;
use chromiumoxide::browser::{Browser, BrowserConfig as ChromeBrowserConfig};
use chromiumoxide::page::ScreenshotParams;
use futures_util::StreamExt;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, warn};

struct BrowserSession {
    browser: Browser,
    page: Page,
    handler: tokio::task::JoinHandle<()>,
    user_data_dir: PathBuf,
}

impl Drop for BrowserSession {
    fn drop(&mut self) {
        self.handler.abort();
        // Clean up user data directory (Chrome process killed by chromiumoxide's Drop)
        if self.user_data_dir.exists() {
            let _ = std::fs::remove_dir_all(&self.user_data_dir);
        }
    }
}

pub struct BrowserTool {
    session: Arc<Mutex<Option<BrowserSession>>>,
    headless: bool,
    chrome_path: Option<String>,
    timeout: u64,
}

impl BrowserTool {
    pub fn new(config: &crate::config::BrowserConfig) -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            headless: config.headless,
            chrome_path: config.chrome_path.clone(),
            timeout: config.timeout,
        }
    }

    #[cfg(test)]
    fn for_testing() -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            headless: true,
            chrome_path: None,
            timeout: 5,
        }
    }

    async fn ensure_session(
        &self,
        session_guard: &mut tokio::sync::MutexGuard<'_, Option<BrowserSession>>,
    ) -> Result<(), String> {
        if session_guard.is_some() {
            return Ok(());
        }

        // Use a unique temp directory per session to avoid SingletonLock conflicts
        // when previous browser instances crashed without cleanup.
        // Include UUID to prevent PID reuse collisions.
        let user_data_dir = std::env::temp_dir().join(format!(
            "oxicrab-chrome-{}-{}",
            std::process::id(),
            &uuid::Uuid::new_v4().to_string()[..8]
        ));

        let mut builder = ChromeBrowserConfig::builder()
            // no_sandbox is required when running as root (e.g. Docker containers).
            // Chrome refuses to start with sandbox when running as root.
            .no_sandbox()
            .user_data_dir(&user_data_dir)
            .launch_timeout(Duration::from_secs(self.timeout))
            .request_timeout(Duration::from_secs(self.timeout));

        if !self.headless {
            builder = builder.with_head();
        }

        if let Some(ref path) = self.chrome_path {
            builder = builder.chrome_executable(path);
        }

        // Clean up stale SingletonLock files from previous crashes
        for lock_dir in [
            user_data_dir.clone(),
            std::env::temp_dir().join("chromiumoxide-runner"),
        ] {
            let lock_path = lock_dir.join("SingletonLock");
            if lock_path.exists() {
                debug!(
                    "removing stale browser SingletonLock: {}",
                    lock_path.display()
                );
                let _ = std::fs::remove_file(&lock_path);
            }
        }

        let browser_config = builder
            .build()
            .map_err(|e| format!("failed to build browser config: {e}"))?;

        let (browser, mut handler) = Browser::launch(browser_config)
            .await
            .map_err(|e| format!("failed to launch browser: {e}"))?;

        let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });

        let page = browser
            .new_page("about:blank")
            .await
            .map_err(|e| format!("failed to create initial page: {e}"))?;

        **session_guard = Some(BrowserSession {
            browser,
            page,
            handler: handler_task,
            user_data_dir,
        });

        Ok(())
    }

    async fn with_timeout<F, T>(&self, future: F) -> Result<T, String>
    where
        F: std::future::Future<Output = Result<T, String>>,
    {
        tokio::time::timeout(Duration::from_secs(self.timeout), future)
            .await
            .map_err(|_| format!("browser operation timed out after {}s", self.timeout))?
    }

    async fn action_open(&self, url: &str) -> Result<ToolResult> {
        // Validate URL (DNS pinning not applicable to browser — Chrome manages its own DNS).
        // NOTE: This only validates the initial URL. Chrome may follow redirects to internal
        // IPs that would otherwise be blocked. Use network-level firewalling (e.g. Docker
        // network isolation) for defense-in-depth against SSRF via browser redirects.
        if let Err(e) = crate::utils::url_security::validate_and_resolve(url).await {
            return Ok(ToolResult::error(format!(
                "URL blocked by security policy: {e}"
            )));
        }

        let mut guard = self.session.lock().await;

        if let Err(e) = self.with_timeout(self.ensure_session(&mut guard)).await {
            return Ok(ToolResult::error(e));
        }

        let session = guard.as_ref().unwrap();
        let result = self
            .with_timeout(async {
                session
                    .page
                    .goto(url)
                    .await
                    .map_err(|e| format!("navigation failed: {e}"))?;

                let title = session
                    .page
                    .get_title()
                    .await
                    .map_err(|e| format!("failed to get title: {e}"))?
                    .unwrap_or_default();

                let current_url = session
                    .page
                    .url()
                    .await
                    .map_err(|e| format!("failed to get URL: {e}"))?
                    .unwrap_or_default();

                // Post-redirect SSRF check: validate the final URL after redirects
                if !current_url.is_empty()
                    && current_url != url
                    && let Err(e) =
                        crate::utils::url_security::validate_and_resolve(&current_url).await
                {
                    return Err(format!("redirect to blocked URL: {current_url} ({e})"));
                }

                Ok(format!("Navigated to: {current_url}\nTitle: {title}"))
            })
            .await;

        match result {
            Ok(text) => Ok(ToolResult::new(text)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }

    async fn action_click(&self, selector: &str) -> Result<ToolResult> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut() else {
            return Ok(ToolResult::error(
                "no browser session. Use 'open' action first".to_string(),
            ));
        };

        let result = self
            .with_timeout(async {
                session
                    .page
                    .find_element(selector)
                    .await
                    .map_err(|e| format!("element not found '{selector}': {e}"))?
                    .click()
                    .await
                    .map_err(|e| format!("click failed: {e}"))?;
                Ok("Clicked element".to_string())
            })
            .await;

        match result {
            Ok(text) => Ok(ToolResult::new(text)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }

    async fn action_type(&self, selector: &str, text: &str) -> Result<ToolResult> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut() else {
            return Ok(ToolResult::error(
                "no browser session. Use 'open' action first".to_string(),
            ));
        };

        let result = self
            .with_timeout(async {
                session
                    .page
                    .find_element(selector)
                    .await
                    .map_err(|e| format!("element not found '{selector}': {e}"))?
                    .click()
                    .await
                    .map_err(|e| format!("focus failed: {e}"))?
                    .type_str(text)
                    .await
                    .map_err(|e| format!("type failed: {e}"))?;
                Ok("Typed text into element".to_string())
            })
            .await;

        match result {
            Ok(text) => Ok(ToolResult::new(text)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }

    async fn action_fill(&self, selector: &str, text: &str) -> Result<ToolResult> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut() else {
            return Ok(ToolResult::error(
                "no browser session. Use 'open' action first".to_string(),
            ));
        };

        let js = format!(
            r"
            (() => {{
                const el = document.querySelector({selector});
                if (!el) throw new Error('element not found: {selector}');
                el.value = '';
                el.value = {value};
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return 'ok';
            }})()
            ",
            selector = serde_json::to_string(selector).unwrap_or_default(),
            value = serde_json::to_string(text).unwrap_or_default(),
        );

        let result = self
            .with_timeout(async {
                session
                    .page
                    .evaluate(js)
                    .await
                    .map_err(|e| format!("fill failed: {e}"))?;
                Ok("Filled element value".to_string())
            })
            .await;

        match result {
            Ok(text) => Ok(ToolResult::new(text)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }

    async fn action_screenshot(&self) -> Result<ToolResult> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut() else {
            return Ok(ToolResult::error(
                "no browser session. Use 'open' action first".to_string(),
            ));
        };

        let result = self
            .with_timeout(async {
                let bytes = session
                    .page
                    .screenshot(
                        ScreenshotParams::builder()
                            .full_page(true)
                            // Clip height to prevent OOM on pathologically tall pages
                            .clip(chromiumoxide::cdp::browser_protocol::page::Viewport {
                                x: 0.0,
                                y: 0.0,
                                width: 1920.0,
                                height: 10080.0, // ~5x 1080p
                                scale: 1.0,
                            })
                            .build(),
                    )
                    .await
                    .map_err(|e| format!("screenshot failed: {e}"))?;

                let path = save_media_file(&bytes, "screenshot", "png")
                    .map_err(|e| format!("failed to save screenshot: {e}"))?;
                Ok(format!(
                    "Screenshot saved to: {path}\nSize: {} bytes\nThe screenshot will be attached to your response automatically.",
                    bytes.len()
                ))
            })
            .await;

        match result {
            Ok(data) => Ok(ToolResult::new(data)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }

    async fn action_snapshot(&self) -> Result<ToolResult> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut() else {
            return Ok(ToolResult::error(
                "no browser session. Use 'open' action first".to_string(),
            ));
        };

        let js = r"
        (() => {
            const title = document.title || '';
            const url = location.href || '';
            const text = document.body ? document.body.innerText.substring(0, 5000) : '';
            const links = Array.from(document.querySelectorAll('a[href]')).slice(0, 50).map(a => ({
                text: (a.innerText || '').trim().substring(0, 80),
                href: a.href
            })).filter(l => l.text);
            const forms = Array.from(document.querySelectorAll('form')).slice(0, 10).map(f => ({
                action: f.action || '',
                method: f.method || 'get',
                inputs: Array.from(f.querySelectorAll('input,textarea,select')).slice(0, 20).map(i => ({
                    type: i.type || i.tagName.toLowerCase(),
                    name: i.name || '',
                    id: i.id || '',
                    placeholder: i.placeholder || ''
                }))
            }));
            return JSON.stringify({ title, url, text, links, forms });
        })()
        ";

        let result = self
            .with_timeout(async {
                let eval_result = session
                    .page
                    .evaluate(js)
                    .await
                    .map_err(|e| format!("snapshot failed: {e}"))?;

                let value: String = eval_result
                    .into_value()
                    .map_err(|e| format!("failed to parse snapshot result: {e}"))?;

                // Pretty-print the JSON for readability
                if let Ok(parsed) = serde_json::from_str::<Value>(&value) {
                    Ok(serde_json::to_string_pretty(&parsed).unwrap_or(value))
                } else {
                    Ok(value)
                }
            })
            .await;

        match result {
            Ok(text) => Ok(ToolResult::new(text)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }

    async fn action_eval(&self, js: &str) -> Result<ToolResult> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut() else {
            return Ok(ToolResult::error(
                "no browser session. Use 'open' action first".to_string(),
            ));
        };

        // Always wrap in an IIFE to provide a fresh scope — prevents
        // "already been declared" errors for const/let across multiple evals
        // on the same page, and allows `return` statements to work
        let js = format!("(() => {{ {js} }})()");
        let result = self
            .with_timeout(async {
                let eval_result = session
                    .page
                    .evaluate(js)
                    .await
                    .map_err(|e| format!("eval failed: {e}"))?;

                let value: Value = eval_result.into_value().unwrap_or(Value::Null);

                match value {
                    Value::String(s) => Ok(s),
                    Value::Null => Ok("null".to_string()),
                    other => Ok(serde_json::to_string_pretty(&other)
                        .unwrap_or_else(|_| format!("{other:?}"))),
                }
            })
            .await;

        match result {
            Ok(text) => Ok(ToolResult::new(text)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }

    async fn action_get(&self, what: &str, selector: Option<&str>) -> Result<ToolResult> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut() else {
            return Ok(ToolResult::error(
                "no browser session. Use 'open' action first".to_string(),
            ));
        };

        let result = self
            .with_timeout(async {
                match what {
                    "title" => {
                        let title = session
                            .page
                            .get_title()
                            .await
                            .map_err(|e| format!("failed to get title: {e}"))?
                            .unwrap_or_default();
                        Ok(title)
                    }
                    "url" => {
                        let url = session
                            .page
                            .url()
                            .await
                            .map_err(|e| format!("failed to get URL: {e}"))?
                            .unwrap_or_default();
                        Ok(url)
                    }
                    "text" => {
                        let js = "document.body ? document.body.innerText : ''";
                        let eval = session
                            .page
                            .evaluate(js)
                            .await
                            .map_err(|e| format!("failed to get text: {e}"))?;
                        let text: String = eval.into_value().unwrap_or_default();
                        Ok(text)
                    }
                    "html" => {
                        let html = session
                            .page
                            .content()
                            .await
                            .map_err(|e| format!("failed to get HTML: {e}"))?;
                        // Cap HTML to 500KB to prevent oversized responses
                        if html.len() > 500 * 1024 {
                            Ok(format!(
                                "{}... [truncated at 500KB, full page is {} bytes]",
                                &html[..html
                                    .char_indices()
                                    .take_while(|(i, _)| *i < 500 * 1024)
                                    .last()
                                    .map_or(0, |(i, _)| i)],
                                html.len()
                            ))
                        } else {
                            Ok(html)
                        }
                    }
                    "value" => {
                        let sel = selector.ok_or_else(|| {
                            "'get' with 'value' requires a 'selector' parameter".to_string()
                        })?;
                        let element = session
                            .page
                            .find_element(sel)
                            .await
                            .map_err(|e| format!("element not found '{sel}': {e}"))?;
                        let value = element
                            .attribute("value")
                            .await
                            .map_err(|e| format!("failed to get value: {e}"))?
                            .unwrap_or_default();
                        Ok(value)
                    }
                    other => Err(format!(
                        "unknown 'what' value '{other}'. Valid: title, url, text, html, value"
                    )),
                }
            })
            .await;

        match result {
            Ok(text) => Ok(ToolResult::new(text)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }

    async fn action_scroll(&self, direction: &str, pixels: Option<u64>) -> Result<ToolResult> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut() else {
            return Ok(ToolResult::error(
                "no browser session. Use 'open' action first".to_string(),
            ));
        };

        let px = pixels.unwrap_or(500) as i64;
        let (dx, dy) = match direction {
            "up" => (0, -px),
            "down" => (0, px),
            "left" => (-px, 0),
            "right" => (px, 0),
            other => {
                return Ok(ToolResult::error(format!(
                    "unknown direction '{other}'. Valid: up, down, left, right"
                )));
            }
        };

        let js = format!("window.scrollBy({dx}, {dy})");
        let result = self
            .with_timeout(async {
                session
                    .page
                    .evaluate(js)
                    .await
                    .map_err(|e| format!("scroll failed: {e}"))?;
                Ok(format!("Scrolled {direction} by {px}px"))
            })
            .await;

        match result {
            Ok(text) => Ok(ToolResult::new(text)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }

    async fn action_wait(&self, selector: Option<&str>, ms: Option<u64>) -> Result<ToolResult> {
        if let Some(sel) = selector {
            let mut guard = self.session.lock().await;
            let Some(session) = guard.as_mut() else {
                return Ok(ToolResult::error(
                    "no browser session. Use 'open' action first".to_string(),
                ));
            };

            let sel = sel.to_string();
            let result = self
                .with_timeout(async {
                    session
                        .page
                        .find_element(&sel)
                        .await
                        .map_err(|e| format!("wait for element failed '{sel}': {e}"))?;
                    Ok(format!("Element '{sel}' found"))
                })
                .await;

            match result {
                Ok(text) => Ok(ToolResult::new(text)),
                Err(e) => Ok(ToolResult::error(e)),
            }
        } else if let Some(duration_ms) = ms {
            let capped = duration_ms.min(self.timeout * 1000);
            tokio::time::sleep(Duration::from_millis(capped)).await;
            Ok(ToolResult::new(format!("Waited {capped}ms")))
        } else {
            Ok(ToolResult::error(
                "'wait' action requires 'selector' (CSS selector) or 'pixels' (ms to wait)"
                    .to_string(),
            ))
        }
    }

    async fn action_close(&self) -> Result<ToolResult> {
        let mut guard = self.session.lock().await;
        if let Some(mut session) = guard.take() {
            if let Err(e) = session.browser.close().await {
                warn!("error closing browser: {e}");
            }
            // Drop handles handler.abort() and user_data_dir cleanup
            drop(session);
            Ok(ToolResult::new("Browser session closed".to_string()))
        } else {
            Ok(ToolResult::new("No browser session to close".to_string()))
        }
    }

    async fn action_navigate(&self, navigation: &str) -> Result<ToolResult> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut() else {
            return Ok(ToolResult::error(
                "no browser session. Use 'open' action first".to_string(),
            ));
        };

        let js = match navigation {
            "back" => "window.history.back()",
            "forward" => "window.history.forward()",
            "reload" => "location.reload()",
            other => {
                return Ok(ToolResult::error(format!(
                    "unknown navigation '{other}'. Valid: back, forward, reload"
                )));
            }
        };

        let result = self
            .with_timeout(async {
                session
                    .page
                    .evaluate(js)
                    .await
                    .map_err(|e| format!("navigation failed: {e}"))?;
                Ok(format!("Navigated: {navigation}"))
            })
            .await;

        match result {
            Ok(text) => Ok(ToolResult::new(text)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &'static str {
        "browser"
    }

    fn description(&self) -> &'static str {
        "Control a headless browser for web automation. Open pages, click elements, type text, take screenshots, and extract content."
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: actions![
                open,
                click,
                type_text,
                fill,
                screenshot,
                snapshot: ro,
                eval,
                get: ro,
                scroll,
                wait,
                close,
                navigate,
            ],
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["open", "click", "type_text", "fill", "screenshot", "snapshot", "eval", "get", "scroll", "wait", "close", "navigate"],
                    "description": "The browser action to perform"
                },
                "url": {
                    "type": "string",
                    "description": "URL to open (required for 'open' action)"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector for the target element (for click/type/fill/get/wait)"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type or fill (for type/fill actions)"
                },
                "javascript": {
                    "type": "string",
                    "description": "JavaScript to evaluate (for eval action)"
                },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"],
                    "description": "Scroll direction (for scroll action)"
                },
                "what": {
                    "type": "string",
                    "description": "What to get: text, html, title, url, value (for get action)"
                },
                "pixels": {
                    "type": "integer",
                    "description": "Number of pixels to scroll (default varies by direction)"
                },
                "navigation": {
                    "type": "string",
                    "enum": ["back", "forward", "reload"],
                    "description": "Navigation direction (for navigate action)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let Some(action) = params["action"].as_str() else {
            return Ok(ToolResult::error(
                "missing required 'action' parameter".to_string(),
            ));
        };

        debug!("browser action: {}", action);

        match action {
            "open" => {
                let url = match params["url"].as_str() {
                    Some(u) if !u.trim().is_empty() => u,
                    _ => {
                        return Ok(ToolResult::error(
                            "'open' action requires 'url' parameter".to_string(),
                        ));
                    }
                };
                self.action_open(url).await
            }
            "click" => {
                let selector = match params["selector"].as_str() {
                    Some(s) if !s.trim().is_empty() => s,
                    _ => {
                        return Ok(ToolResult::error(
                            "'click' action requires 'selector' parameter".to_string(),
                        ));
                    }
                };
                self.action_click(selector).await
            }
            "type_text" => {
                let selector = match params["selector"].as_str() {
                    Some(s) if !s.trim().is_empty() => s,
                    _ => {
                        return Ok(ToolResult::error(
                            "'type_text' action requires 'selector' parameter".to_string(),
                        ));
                    }
                };
                let Some(text) = params["text"].as_str() else {
                    return Ok(ToolResult::error(
                        "'type_text' action requires 'text' parameter".to_string(),
                    ));
                };
                self.action_type(selector, text).await
            }
            "fill" => {
                let selector = match params["selector"].as_str() {
                    Some(s) if !s.trim().is_empty() => s,
                    _ => {
                        return Ok(ToolResult::error(
                            "'fill' action requires 'selector' parameter".to_string(),
                        ));
                    }
                };
                let Some(text) = params["text"].as_str() else {
                    return Ok(ToolResult::error(
                        "'fill' action requires 'text' parameter".to_string(),
                    ));
                };
                self.action_fill(selector, text).await
            }
            "screenshot" => self.action_screenshot().await,
            "snapshot" => self.action_snapshot().await,
            "eval" => {
                let js = match params["javascript"].as_str() {
                    Some(j) if !j.trim().is_empty() => j,
                    _ => {
                        return Ok(ToolResult::error(
                            "'eval' action requires 'javascript' parameter".to_string(),
                        ));
                    }
                };
                self.action_eval(js).await
            }
            "get" => {
                let what = match params["what"].as_str() {
                    Some(w) if !w.trim().is_empty() => w,
                    _ => {
                        return Ok(ToolResult::error(
                            "'get' action requires 'what' parameter".to_string(),
                        ));
                    }
                };
                let selector = params["selector"].as_str();
                self.action_get(what, selector).await
            }
            "scroll" => {
                let Some(direction) = params["direction"].as_str() else {
                    return Ok(ToolResult::error(
                        "'scroll' action requires 'direction' parameter".to_string(),
                    ));
                };
                let pixels = params["pixels"].as_u64();
                self.action_scroll(direction, pixels).await
            }
            "wait" => {
                let selector = params["selector"].as_str().filter(|s| !s.trim().is_empty());
                let ms = params["pixels"].as_u64(); // reuse pixels field for ms
                self.action_wait(selector, ms).await
            }
            "close" => self.action_close().await,
            "navigate" => {
                let Some(nav) = params["navigation"].as_str() else {
                    return Ok(ToolResult::error(
                        "'navigate' action requires 'navigation' parameter (back/forward/reload)"
                            .to_string(),
                    ));
                };
                self.action_navigate(nav).await
            }
            unknown => Ok(ToolResult::error(format!(
                "unknown browser action '{}'. Valid actions: open, click, type_text, fill, screenshot, snapshot, eval, get, scroll, wait, close, navigate",
                unknown
            ))),
        }
    }
}

#[cfg(test)]
mod tests;
