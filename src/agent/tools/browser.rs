use crate::agent::tools::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tracing::debug;

pub struct BrowserTool {
    browser_path: String,
    timeout: u64,
}

impl BrowserTool {
    pub fn new(config: &crate::config::BrowserConfig) -> Result<Self> {
        let browser_path = if let Some(ref path) = config.agent_browser_path {
            if std::path::Path::new(path.as_str()).exists() {
                path.clone()
            } else {
                anyhow::bail!("configured agent_browser_path does not exist: {}", path);
            }
        } else {
            // Auto-detect via `which`
            which::which("agent-browser")
                .map_err(|_| {
                    anyhow::anyhow!(
                        "agent-browser not found in PATH; install it or set tools.browser.agentBrowserPath"
                    )
                })?
                .to_string_lossy()
                .to_string()
        };

        Ok(Self {
            browser_path,
            timeout: config.timeout,
        })
    }

    #[cfg(test)]
    fn with_path(path: String, timeout: u64) -> Self {
        Self {
            browser_path: path,
            timeout,
        }
    }

    async fn run_browser(&self, args: &[&str]) -> Result<(i32, String, String)> {
        let output = Command::new(&self.browser_path)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output();

        let result = tokio::time::timeout(Duration::from_secs(self.timeout), output).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                Ok((output.status.code().unwrap_or(1), stdout, stderr))
            }
            Ok(Err(e)) => Err(anyhow::anyhow!("failed to execute agent-browser: {}", e)),
            Err(_) => Err(anyhow::anyhow!(
                "agent-browser command timed out after {}s",
                self.timeout
            )),
        }
    }

    fn format_result(code: i32, stdout: &str, stderr: &str) -> ToolResult {
        if code == 0 {
            let output = if stdout.trim().is_empty() {
                "Command completed successfully (no output)".to_string()
            } else {
                stdout.trim().to_string()
            };
            ToolResult::new(output)
        } else {
            let error = if !stderr.trim().is_empty() {
                stderr.trim().to_string()
            } else if !stdout.trim().is_empty() {
                stdout.trim().to_string()
            } else {
                format!("agent-browser exited with code {}", code)
            };
            ToolResult::error(error)
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

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["open", "click", "type", "fill", "screenshot", "snapshot", "eval", "get", "scroll", "wait", "close", "navigate"],
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

    async fn execute(&self, params: Value) -> Result<ToolResult> {
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
                    _ => return Ok(ToolResult::error("'open' action requires 'url' parameter".to_string())),
                };

                // SSRF validation
                if let Err(e) = crate::utils::url_security::validate_url(url) {
                    return Ok(ToolResult::error(format!("URL blocked by security policy: {}", e)));
                }

                let (code, stdout, stderr) = self
                    .run_browser(&["--session", "nanobot", "open", url])
                    .await?;
                Ok(Self::format_result(code, &stdout, &stderr))
            }
            "click" => {
                let selector = match params["selector"].as_str() {
                    Some(s) if !s.trim().is_empty() => s,
                    _ => return Ok(ToolResult::error("'click' action requires 'selector' parameter".to_string())),
                };
                let (code, stdout, stderr) = self
                    .run_browser(&["--session", "nanobot", "click", selector])
                    .await?;
                Ok(Self::format_result(code, &stdout, &stderr))
            }
            "type" => {
                let selector = match params["selector"].as_str() {
                    Some(s) if !s.trim().is_empty() => s,
                    _ => return Ok(ToolResult::error("'type' action requires 'selector' parameter".to_string())),
                };
                let Some(text) = params["text"].as_str() else {
                    return Ok(ToolResult::error("'type' action requires 'text' parameter".to_string()));
                };
                let (code, stdout, stderr) = self
                    .run_browser(&["--session", "nanobot", "type", selector, text])
                    .await?;
                Ok(Self::format_result(code, &stdout, &stderr))
            }
            "fill" => {
                let selector = match params["selector"].as_str() {
                    Some(s) if !s.trim().is_empty() => s,
                    _ => return Ok(ToolResult::error("'fill' action requires 'selector' parameter".to_string())),
                };
                let Some(text) = params["text"].as_str() else {
                    return Ok(ToolResult::error("'fill' action requires 'text' parameter".to_string()));
                };
                let (code, stdout, stderr) = self
                    .run_browser(&["--session", "nanobot", "fill", selector, text])
                    .await?;
                Ok(Self::format_result(code, &stdout, &stderr))
            }
            "screenshot" => {
                let (code, stdout, stderr) = self
                    .run_browser(&["--session", "nanobot", "screenshot"])
                    .await?;
                Ok(Self::format_result(code, &stdout, &stderr))
            }
            "snapshot" => {
                let (code, stdout, stderr) = self
                    .run_browser(&["--session", "nanobot", "snapshot", "-i"])
                    .await?;
                Ok(Self::format_result(code, &stdout, &stderr))
            }
            "eval" => {
                let js = match params["javascript"].as_str() {
                    Some(j) if !j.trim().is_empty() => j,
                    _ => return Ok(ToolResult::error("'eval' action requires 'javascript' parameter".to_string())),
                };
                let (code, stdout, stderr) = self
                    .run_browser(&["--session", "nanobot", "eval", js])
                    .await?;
                Ok(Self::format_result(code, &stdout, &stderr))
            }
            "get" => {
                let what = match params["what"].as_str() {
                    Some(w) if !w.trim().is_empty() => w,
                    _ => return Ok(ToolResult::error("'get' action requires 'what' parameter".to_string())),
                };
                let mut args = vec!["--session", "nanobot", "get", what];
                let selector_str;
                if let Some(sel) = params["selector"].as_str() {
                    selector_str = sel.to_string();
                    args.push(&selector_str);
                }
                let (code, stdout, stderr) = self.run_browser(&args).await?;
                Ok(Self::format_result(code, &stdout, &stderr))
            }
            "scroll" => {
                let Some(direction) = params["direction"].as_str() else {
                    return Ok(ToolResult::error("'scroll' action requires 'direction' parameter".to_string()));
                };
                let mut args = vec!["--session", "nanobot", "scroll", direction];
                let pixels_str;
                if let Some(px) = params["pixels"].as_u64() {
                    pixels_str = px.to_string();
                    args.push(&pixels_str);
                }
                let (code, stdout, stderr) = self.run_browser(&args).await?;
                Ok(Self::format_result(code, &stdout, &stderr))
            }
            "wait" => {
                let selector_or_ms = match params["selector"].as_str() {
                    Some(s) if !s.trim().is_empty() => s.to_string(),
                    _ => match params["pixels"].as_u64() {
                        // reuse pixels field for ms if no selector
                        Some(ms) => ms.to_string(),
                        None => return Ok(ToolResult::error(
                            "'wait' action requires 'selector' (CSS selector) or a timeout".to_string(),
                        )),
                    },
                };
                let (code, stdout, stderr) = self
                    .run_browser(&["--session", "nanobot", "wait", &selector_or_ms])
                    .await?;
                Ok(Self::format_result(code, &stdout, &stderr))
            }
            "close" => {
                let (code, stdout, stderr) = self
                    .run_browser(&["--session", "nanobot", "close"])
                    .await?;
                Ok(Self::format_result(code, &stdout, &stderr))
            }
            "navigate" => {
                let Some(nav) = params["navigation"].as_str() else {
                    return Ok(ToolResult::error("'navigate' action requires 'navigation' parameter (back/forward/reload)".to_string()));
                };
                let (code, stdout, stderr) = self
                    .run_browser(&["--session", "nanobot", nav])
                    .await?;
                Ok(Self::format_result(code, &stdout, &stderr))
            }
            unknown => Ok(ToolResult::error(format!(
                "unknown browser action '{}'. Valid actions: open, click, type, fill, screenshot, snapshot, eval, get, scroll, wait, close, navigate",
                unknown
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_result_success() {
        let result = BrowserTool::format_result(0, "page loaded", "");
        assert!(!result.is_error);
        assert_eq!(result.content, "page loaded");
    }

    #[test]
    fn test_format_result_success_empty() {
        let result = BrowserTool::format_result(0, "", "");
        assert!(!result.is_error);
        assert!(result.content.contains("no output"));
    }

    #[test]
    fn test_format_result_error() {
        let result = BrowserTool::format_result(1, "", "element not found");
        assert!(result.is_error);
        assert!(result.content.contains("element not found"));
    }

    #[test]
    fn test_format_result_error_exit_code_only() {
        let result = BrowserTool::format_result(1, "", "");
        assert!(result.is_error);
        assert!(result.content.contains("exited with code 1"));
    }

    #[tokio::test]
    async fn test_open_ssrf_blocked() {
        let tool = BrowserTool::with_path("/bin/echo".to_string(), 5);
        let params = serde_json::json!({
            "action": "open",
            "url": "http://169.254.169.254/latest/meta-data"
        });
        let result = tool.execute(params).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("security policy") || result.content.contains("blocked"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let tool = BrowserTool::with_path("/bin/echo".to_string(), 5);
        let params = serde_json::json!({"action": "destroy"});
        let result = tool.execute(params).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("unknown browser action"));
    }

    #[tokio::test]
    async fn test_missing_action() {
        let tool = BrowserTool::with_path("/bin/echo".to_string(), 5);
        let params = serde_json::json!({});
        let result = tool.execute(params).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("action"));
    }

    #[tokio::test]
    async fn test_open_missing_url() {
        let tool = BrowserTool::with_path("/bin/echo".to_string(), 5);
        let params = serde_json::json!({"action": "open"});
        let result = tool.execute(params).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("url"));
    }

    #[tokio::test]
    async fn test_click_missing_selector() {
        let tool = BrowserTool::with_path("/bin/echo".to_string(), 5);
        let params = serde_json::json!({"action": "click"});
        let result = tool.execute(params).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("selector"));
    }

    #[test]
    fn test_new_missing_binary() {
        let config = crate::config::BrowserConfig {
            enabled: true,
            agent_browser_path: Some("/nonexistent/agent-browser".to_string()),
            timeout: 30,
        };
        let result = BrowserTool::new(&config);
        assert!(result.is_err());
    }
}
