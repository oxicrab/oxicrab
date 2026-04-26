pub mod proxy;

use crate::agent::tools::Tool;
use crate::config::{McpConfig, McpTrust, SandboxConfig};
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
    trust_level: McpTrust,
    /// PID of the MCP server child process. Captured before
    /// `serve(transport)` consumed the transport so shutdown can
    /// send SIGTERM first and only fall back to SIGKILL after a
    /// grace period — rmcp's drop path SIGKILLs immediately.
    #[cfg(unix)]
    child_pid: Option<u32>,
}

/// Manages connections to MCP servers and discovers their tools.
///
/// Timeouts: 30s on the initial JSON-RPC handshake, 10s on per-server
/// tool discovery. Both bound startup so a single dead MCP server can't
/// stall the agent boot.
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
                    warn!("failed to connect MCP server '{}': {}", name, e);
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
        trust: &McpTrust,
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
            if k.contains('\r') || k.contains('\n') || v.contains('\r') || v.contains('\n') {
                warn!(
                    "MCP server '{}' env var '{}' contains CR/LF characters, skipping",
                    name, k
                );
                continue;
            }
            cmd.env(k, v);
        }

        // Apply Landlock sandbox (same rules as shell tool)
        if sandbox.enabled {
            let rules = crate::utils::sandbox::SandboxRules::for_shell(workspace, sandbox);
            if let Err(e) = crate::utils::sandbox::apply_to_command(&mut cmd, &rules) {
                anyhow::bail!(
                    "sandbox is required but failed to apply for MCP server '{name}': {e}"
                );
            }
        }

        // Pipe stdin/stdout for MCP communication, inherit stderr for logging
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::inherit());

        let transport = TokioChildProcess::new(cmd)?;
        // Capture the PID before `serve` consumes the transport so
        // shutdown can SIGTERM the child first.
        #[cfg(unix)]
        let child_pid = transport.id();
        let client = tokio::time::timeout(std::time::Duration::from_secs(30), ().serve(transport))
            .await
            .map_err(|_| anyhow::anyhow!("MCP handshake timed out for server '{name}' (30s)"))?
            .map_err(|e| anyhow::anyhow!("MCP handshake failed for server '{name}': {e}"))?;

        Ok(RunningMcpServer {
            client,
            server_name: name.to_string(),
            trust_level: trust.clone(),
            #[cfg(unix)]
            child_pid,
        })
    }

    /// Discover all tools across all connected MCP servers and wrap them as `impl Tool`.
    /// Returns `(trust_level, tool)` tuples so callers can apply trust-based filtering.
    pub async fn discover_tools(&self) -> Vec<(McpTrust, Arc<dyn Tool>)> {
        let mut tools: Vec<(McpTrust, Arc<dyn Tool>)> = Vec::new();

        for server in &self.servers {
            let Ok(mcp_tools_result) = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                server.client.peer().list_all_tools(),
            )
            .await
            else {
                warn!(
                    "tool discovery timed out for MCP server '{}' (10s)",
                    server.server_name
                );
                continue;
            };
            match mcp_tools_result {
                Ok(mcp_tools) => {
                    for mcp_tool in mcp_tools {
                        let description = mcp_tool
                            .description
                            .as_deref()
                            .unwrap_or_default()
                            .to_string();

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
                            "discovered MCP tool '{}' from server '{}' (trust: {})",
                            mcp_tool.name, server.server_name, server.trust_level
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "failed to list tools from MCP server '{}': {}",
                        server.server_name, e
                    );
                }
            }
        }

        tools
    }

    /// Gracefully shut down all MCP server connections.
    ///
    /// On Unix, send SIGTERM to each child first and give it a 3s
    /// grace period to flush logs and write final state before the
    /// rmcp Drop path SIGKILLs. Without this, every shutdown is a
    /// hard kill — fine for stateless servers but rude to ones that
    /// flush a journal.
    pub async fn shutdown(self) {
        for server in self.servers {
            #[cfg(unix)]
            if let Some(pid) = server.child_pid {
                // SAFETY: kill(2) is async-signal-safe. SIGTERM
                // to a process group is the standard graceful-stop
                // signal; the child is free to ignore it.
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGTERM);
                }
            }
            if let Err(e) = server.client.cancel().await {
                warn!(
                    "error shutting down MCP server '{}': {}",
                    server.server_name, e
                );
            }
            #[cfg(unix)]
            if server.child_pid.is_some() {
                // Brief grace period before rmcp's Drop SIGKILLs.
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests;
