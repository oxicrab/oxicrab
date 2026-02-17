pub mod proxy;

use crate::agent::tools::Tool;
use crate::config::McpConfig;
use anyhow::Result;
use rmcp::ServiceExt;
use rmcp::transport::TokioChildProcess;
use std::sync::Arc;
use tokio::process::Command;
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
}

impl McpManager {
    /// Connect to all enabled MCP servers defined in config.
    pub async fn new(config: &McpConfig) -> Result<Self> {
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

        Ok(Self { servers })
    }

    async fn connect_server(
        name: &str,
        command: &str,
        args: &[String],
        env: &std::collections::HashMap<String, String>,
        trust: &str,
    ) -> Result<RunningMcpServer> {
        let mut cmd = Command::new(command);
        cmd.args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        // Pipe stdin/stdout for MCP communication, inherit stderr for logging
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::inherit());

        let transport = TokioChildProcess::new(cmd)?;
        let client = ()
            .serve(transport)
            .await
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
            match server.client.peer().list_all_tools().await {
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
