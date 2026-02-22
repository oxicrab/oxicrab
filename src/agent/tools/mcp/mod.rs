pub mod proxy;

use crate::agent::tools::Tool;
use crate::config::{McpConfig, SandboxConfig};
use anyhow::Result;
use rmcp::ServiceExt;
use rmcp::transport::TokioChildProcess;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

use proxy::McpProxyTool;

/// A running MCP server connection.
struct RunningMcpServer {
    client: rmcp::service::RunningService<rmcp::RoleClient, ()>,
    server_name: String,
    trust_level: String,
}

/// Manages connections to MCP servers and discovers their tools.
pub struct McpManager {
    servers: Vec<RunningMcpServer>,
    workspace: PathBuf,
}

impl McpManager {
    /// Connect to all enabled MCP servers defined in config.
    pub async fn new(config: &McpConfig, workspace: &Path) -> Result<Self> {
        let mut servers = Vec::new();

        for (name, server_cfg) in &config.servers {
            if !server_cfg.enabled {
                info!("MCP server '{}' is disabled, skipping", name);
                continue;
            }

            match Self::connect_server(
                name,
                &server_cfg.command,
                &server_cfg.args,
                &server_cfg.env,
                &server_cfg.trust,
                &server_cfg.sandbox,
                workspace,
            )
            .await
            {
                Ok(server) => {
                    info!(
                        "MCP server '{}' connected (trust: {})",
                        name, server.trust_level
                    );
                    servers.push(server);
                }
                Err(e) => {
                    warn!("Failed to connect MCP server '{}': {}", name, e);
                }
            }
        }

        Ok(Self {
            servers,
            workspace: workspace.to_path_buf(),
        })
    }

    async fn connect_server(
        name: &str,
        command: &str,
        args: &[String],
        env: &std::collections::HashMap<String, String>,
        trust: &str,
        sandbox: &SandboxConfig,
        workspace: &Path,
    ) -> Result<RunningMcpServer> {
        let mut cmd = crate::utils::subprocess::scrubbed_command(command);
        cmd.args(args);
        for (k, v) in env {
            // Warn if an env var looks like it might contain a secret
            let v_lower = v.to_lowercase();
            if v.len() > 20
                && (k.to_lowercase().contains("key")
                    || k.to_lowercase().contains("secret")
                    || k.to_lowercase().contains("token")
                    || k.to_lowercase().contains("password")
                    || v_lower.starts_with("sk-")
                    || v_lower.starts_with("ghp_")
                    || v_lower.starts_with("xoxb-"))
            {
                warn!(
                    "MCP server '{}' env var '{}' may contain a secret — consider using a credential helper instead",
                    name, k
                );
            }
            cmd.env(k, v);
        }

        // Apply Landlock sandbox (same rules as shell tool)
        if sandbox.enabled {
            let rules = crate::utils::sandbox::SandboxRules::for_shell(workspace, sandbox);
            if let Err(e) = crate::utils::sandbox::apply_to_command(&mut cmd, &rules) {
                warn!(
                    "failed to apply sandbox to MCP server '{}': {}, continuing without",
                    name, e
                );
            }
        }

        // Pipe stdin/stdout for MCP communication, inherit stderr for logging
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::inherit());

        let transport = TokioChildProcess::new(cmd)?;
        let client = tokio::time::timeout(std::time::Duration::from_secs(30), ().serve(transport))
            .await
            .map_err(|_| anyhow::anyhow!("MCP handshake timed out for server '{}' (30s)", name))?
            .map_err(|e| anyhow::anyhow!("MCP handshake failed for server '{}': {}", name, e))?;

        Ok(RunningMcpServer {
            client,
            server_name: name.to_string(),
            trust_level: trust.to_string(),
        })
    }

    /// Discover all tools across all connected MCP servers and wrap them as `impl Tool`.
    /// Returns `(trust_level, tool)` tuples so callers can apply trust-based filtering.
    pub async fn discover_tools(&self) -> Vec<(String, Arc<dyn Tool>)> {
        let mut tools: Vec<(String, Arc<dyn Tool>)> = Vec::new();

        for server in &self.servers {
            let Ok(mcp_tools_result) = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                server.client.peer().list_all_tools(),
            )
            .await
            else {
                warn!(
                    "Tool discovery timed out for MCP server '{}' (10s)",
                    server.server_name
                );
                continue;
            };
            match mcp_tools_result {
                Ok(mcp_tools) => {
                    for mcp_tool in mcp_tools {
                        let description = mcp_tool.description.as_deref().unwrap_or("").to_string();

                        // Convert the input_schema Arc<Map> to a Value
                        let input_schema =
                            serde_json::Value::Object((*mcp_tool.input_schema).clone());

                        let proxy = McpProxyTool::new(
                            server.client.peer().clone(),
                            &server.server_name,
                            mcp_tool.name.to_string(),
                            description,
                            input_schema,
                            Some(self.workspace.clone()),
                        );
                        tools.push((server.trust_level.clone(), Arc::new(proxy)));
                        info!(
                            "Discovered MCP tool '{}' from server '{}' (trust: {})",
                            mcp_tool.name, server.server_name, server.trust_level
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to list tools from MCP server '{}': {}",
                        server.server_name, e
                    );
                }
            }
        }

        tools
    }

    /// Gracefully shut down all MCP server connections.
    pub async fn shutdown(self) {
        for server in self.servers {
            if let Err(e) = server.client.cancel().await {
                warn!(
                    "Error shutting down MCP server '{}': {}",
                    server.server_name, e
                );
            }
        }
    }
}
