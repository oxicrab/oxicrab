use crate::agent::memory::MemoryStore;
use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::subagent::{SubagentConfig, SubagentManager};
use crate::agent::tools::mcp::McpManager;
use crate::agent::tools::mcp::proxy::AttenuatedMcpTool;
use crate::agent::tools::{Tool, ToolRegistry};
use crate::bus::{MessageBus, OutboundMessage};
use crate::config;
use crate::cron::service::CronService;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// All configuration and shared state needed to construct tools.
/// Built once during `AgentLoop::new()` and passed to each module's `register()`.
pub struct ToolBuildContext {
    pub workspace: PathBuf,
    pub restrict_to_workspace: bool,
    pub exec_timeout: u64,
    pub allowed_commands: Vec<String>,
    pub sandbox_config: config::SandboxConfig,
    pub outbound_tx: Arc<tokio::sync::mpsc::Sender<OutboundMessage>>,
    pub bus: Arc<Mutex<MessageBus>>,
    pub brave_api_key: Option<String>,
    pub web_search_config: Option<config::WebSearchConfig>,
    pub cron_service: Option<Arc<CronService>>,
    pub channels_config: Option<config::ChannelsConfig>,
    pub google_config: Option<config::GoogleConfig>,
    pub github_config: Option<config::GitHubConfig>,
    pub weather_config: Option<config::WeatherConfig>,
    pub todoist_config: Option<config::TodoistConfig>,
    pub media_config: Option<config::MediaConfig>,
    pub obsidian_config: Option<config::ObsidianConfig>,
    pub browser_config: Option<config::BrowserConfig>,
    pub image_gen_config: Option<config::ImageGenConfig>,
    pub memory: Arc<MemoryStore>,
    pub subagent_config: SubagentConfig,
    pub mcp_config: Option<config::McpConfig>,
    pub memory_db: Option<Arc<MemoryDB>>,
    pub workspace_manager: Option<Arc<crate::agent::workspace::WorkspaceManager>>,
    pub workspace_ttl: config::WorkspaceTtlConfig,
}

/// Register all tools into the registry using decentralized per-module `register()` functions.
/// Returns `(ToolRegistry, SubagentManager, Option<McpManager>)`.
pub async fn register_all_tools(
    ctx: &ToolBuildContext,
) -> Result<(ToolRegistry, Arc<SubagentManager>, Option<McpManager>)> {
    let mut tools = ToolRegistry::new();

    register_filesystem(&mut tools, ctx);
    register_shell(&mut tools, ctx)?;
    register_web(&mut tools, ctx);
    let subagents = register_subagents(&mut tools, ctx);
    register_tmux(&mut tools);
    register_browser(&mut tools, ctx);
    register_image_gen(&mut tools, ctx);
    register_cron(&mut tools, ctx);
    register_github(&mut tools, ctx);
    register_weather(&mut tools, ctx);
    register_todoist(&mut tools, ctx);
    register_media(&mut tools, ctx);
    register_obsidian(&mut tools, ctx);
    register_http(&mut tools);
    register_reddit(&mut tools);
    register_memory_search(&mut tools, ctx);
    register_workspace(&mut tools, ctx);

    // Slow async registrations — run in parallel
    let (google_tools, mcp_result) = tokio::join!(create_google_tools(ctx), create_mcp(ctx),);

    for tool in google_tools {
        tools.register(tool);
    }

    let mcp_manager = if let Some((mcp_tools, manager)) = mcp_result {
        for tool in mcp_tools {
            let name = tool.name().to_string();
            // Reject MCP tools that shadow built-in tools (capability-based)
            if let Some(existing) = tools.get(&name)
                && existing.capabilities().built_in
            {
                warn!("MCP tool '{}' rejected: shadows a built-in tool", name);
                continue;
            }
            tools.register(tool);
        }
        Some(manager)
    } else {
        None
    };

    Ok((tools, subagents, mcp_manager))
}

fn register_filesystem(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::filesystem::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool};

    let allowed_roots = if ctx.restrict_to_workspace {
        let mut roots = vec![ctx.workspace.clone()];
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".oxicrab"));
        }
        Some(roots)
    } else {
        None
    };

    let backup_dir = dirs::home_dir().map(|h| h.join(".oxicrab/backups"));
    let workspace = Some(ctx.workspace.clone());
    let ws_mgr = ctx.workspace_manager.clone();

    let mut read_tool = ReadFileTool::new(allowed_roots.clone(), workspace.clone());
    if let Some(ref mgr) = ws_mgr {
        read_tool = read_tool.with_workspace_manager(mgr.clone());
    }
    registry.register(Arc::new(read_tool));

    let mut write_tool =
        WriteFileTool::new(allowed_roots.clone(), backup_dir.clone(), workspace.clone());
    if let Some(ref mgr) = ws_mgr {
        write_tool = write_tool.with_workspace_manager(mgr.clone());
    }
    registry.register(Arc::new(write_tool));

    registry.register(Arc::new(EditFileTool::new(
        allowed_roots.clone(),
        backup_dir,
        workspace.clone(),
    )));
    registry.register(Arc::new(ListDirTool::new(allowed_roots, workspace)));
}

fn register_shell(registry: &mut ToolRegistry, ctx: &ToolBuildContext) -> Result<()> {
    use crate::agent::tools::shell::ExecTool;

    registry.register(Arc::new(ExecTool::new(
        ctx.exec_timeout,
        Some(ctx.workspace.clone()),
        ctx.restrict_to_workspace,
        ctx.allowed_commands.clone(),
        ctx.sandbox_config.clone(),
    )?));
    Ok(())
}

fn register_web(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::web::{WebFetchTool, WebSearchTool};

    if let Some(ref ws_cfg) = ctx.web_search_config {
        registry.register(Arc::new(WebSearchTool::from_config(ws_cfg)));
    } else {
        registry.register(Arc::new(WebSearchTool::new(ctx.brave_api_key.clone(), 5)));
    }
    if let Ok(fetch) = WebFetchTool::new(50000) {
        registry.register(Arc::new(fetch));
    }
}

fn register_subagents(registry: &mut ToolRegistry, ctx: &ToolBuildContext) -> Arc<SubagentManager> {
    use crate::agent::tools::spawn::SpawnTool;
    use crate::agent::tools::subagent_control::SubagentControlTool;

    let subagents = Arc::new(SubagentManager::new(
        ctx.subagent_config.clone(),
        ctx.bus.clone(),
    ));

    registry.register(Arc::new(SpawnTool::new(subagents.clone())));
    registry.register(Arc::new(SubagentControlTool::new(subagents.clone())));

    subagents
}

fn register_tmux(registry: &mut ToolRegistry) {
    use crate::agent::tools::tmux::TmuxTool;

    registry.register(Arc::new(TmuxTool::new()));
}

fn register_browser(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::browser::BrowserTool;

    if let Some(ref browser_cfg) = ctx.browser_config
        && browser_cfg.enabled
    {
        registry.register(Arc::new(BrowserTool::new(browser_cfg)));
        info!("Browser tool registered");
    }
}

fn register_image_gen(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::image_gen::ImageGenTool;

    if let Some(ref ig_cfg) = ctx.image_gen_config
        && ig_cfg.enabled
    {
        registry.register(Arc::new(ImageGenTool::new(
            ig_cfg.openai_api_key.clone(),
            ig_cfg.google_api_key.clone(),
            ig_cfg.default_provider.clone(),
        )));
        info!("Image generation tool registered");
    }
}

fn register_cron(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::cron::CronTool;

    if let Some(ref cron_svc) = ctx.cron_service {
        registry.register(Arc::new(CronTool::new(
            cron_svc.clone(),
            ctx.channels_config.clone(),
            ctx.memory_db.clone(),
        )));
    }
}

async fn create_google_tools(ctx: &ToolBuildContext) -> Vec<Arc<dyn Tool>> {
    use crate::agent::tools::google_calendar::GoogleCalendarTool;
    use crate::agent::tools::google_mail::GoogleMailTool;

    let mut result: Vec<Arc<dyn Tool>> = Vec::new();

    if let Some(ref google_cfg) = ctx.google_config
        && google_cfg.enabled
        && !google_cfg.client_id.is_empty()
        && !google_cfg.client_secret.is_empty()
    {
        match crate::auth::google::get_credentials(
            &google_cfg.client_id,
            &google_cfg.client_secret,
            Some(&google_cfg.scopes),
            None,
        )
        .await
        {
            Ok(creds) => {
                result.push(Arc::new(GoogleMailTool::new(creds.clone())));
                result.push(Arc::new(GoogleCalendarTool::new(creds)));
                info!("Google tools registered (gmail, calendar)");
            }
            Err(e) => {
                warn!("Google tools not available: {}", e);
            }
        }
    }

    result
}

fn register_github(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::github::GitHubTool;

    if let Some(ref gh_cfg) = ctx.github_config
        && gh_cfg.enabled
        && !gh_cfg.token.is_empty()
    {
        registry.register(Arc::new(GitHubTool::new(gh_cfg.token.clone())));
        info!("GitHub tool registered");
    }
}

fn register_weather(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::weather::WeatherTool;

    if let Some(ref weather_cfg) = ctx.weather_config
        && weather_cfg.enabled
        && !weather_cfg.api_key.is_empty()
    {
        registry.register(Arc::new(WeatherTool::new(weather_cfg.api_key.clone())));
        info!("Weather tool registered");
    }
}

fn register_todoist(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::todoist::TodoistTool;

    if let Some(ref todoist_cfg) = ctx.todoist_config
        && todoist_cfg.enabled
        && !todoist_cfg.token.is_empty()
    {
        registry.register(Arc::new(TodoistTool::new(todoist_cfg.token.clone())));
        info!("Todoist tool registered");
    }
}

fn register_media(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::media::MediaTool;

    if let Some(ref media_cfg) = ctx.media_config
        && media_cfg.enabled
    {
        registry.register(Arc::new(MediaTool::new(media_cfg)));
        info!("Media tool registered (Radarr/Sonarr)");
    }
}

fn register_obsidian(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::obsidian::{ObsidianSyncService, ObsidianTool};

    if let Some(ref obsidian_cfg) = ctx.obsidian_config
        && obsidian_cfg.enabled
        && !obsidian_cfg.api_url.is_empty()
        && !obsidian_cfg.api_key.is_empty()
    {
        match ObsidianTool::new(
            &obsidian_cfg.api_url,
            &obsidian_cfg.api_key,
            &obsidian_cfg.vault_name,
            obsidian_cfg.timeout,
        ) {
            Ok((tool, cache)) => {
                registry.register(Arc::new(tool));
                let sync_svc = ObsidianSyncService::new(cache, obsidian_cfg.sync_interval);
                tokio::spawn(async move {
                    if let Err(e) = sync_svc.start().await {
                        error!("Obsidian sync failed to start: {}", e);
                    }
                });
                info!("Obsidian tool registered");
            }
            Err(e) => {
                warn!("Obsidian tool not available: {}", e);
            }
        }
    }
}

fn register_http(registry: &mut ToolRegistry) {
    use crate::agent::tools::http::HttpTool;

    registry.register(Arc::new(HttpTool::new()));
}

fn register_reddit(registry: &mut ToolRegistry) {
    use crate::agent::tools::reddit::RedditTool;

    registry.register(Arc::new(RedditTool::new()));
}

fn register_memory_search(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::memory_search::MemorySearchTool;

    registry.register(Arc::new(MemorySearchTool::new(ctx.memory.clone())));
}

fn register_workspace(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::workspace_tool::WorkspaceTool;

    if let Some(ref mgr) = ctx.workspace_manager {
        registry.register(Arc::new(WorkspaceTool::new(
            mgr.clone(),
            ctx.workspace_ttl.clone(),
        )));
        info!("Workspace tool registered");
    }
}

/// Check whether a tool name is safe for community-trust MCP servers.
/// Uses word-boundary matching (camelCase → segments) to avoid substring
/// false positives like "breadcrumb" containing "read".
fn is_community_safe(tool_name: &str) -> bool {
    const SAFE_KEYWORDS: &[&str] = &[
        "read", "list", "get", "search", "find", "query", "fetch", "view", "show", "count",
    ];
    // Normalize camelCase to snake_case, then check word segments
    let mut normalized = String::with_capacity(tool_name.len() + 10);
    for (i, ch) in tool_name.char_indices() {
        if ch.is_ascii_uppercase() && i > 0 {
            normalized.push('_');
        }
        normalized.push(ch.to_ascii_lowercase());
    }
    normalized
        .split(|c: char| !c.is_alphanumeric())
        .any(|seg| SAFE_KEYWORDS.contains(&seg))
}

async fn create_mcp(ctx: &ToolBuildContext) -> Option<(Vec<Arc<dyn Tool>>, McpManager)> {
    let mcp_cfg = ctx.mcp_config.as_ref()?;
    if mcp_cfg.servers.is_empty() {
        return None;
    }
    match McpManager::new(mcp_cfg, &ctx.workspace).await {
        Ok(manager) => {
            let discovered = manager.discover_tools().await;
            let mut accepted: Vec<Arc<dyn Tool>> = Vec::new();
            for (trust, tool) in discovered {
                let name = tool.name().to_string();
                match trust.as_str() {
                    "local" => {
                        accepted.push(tool);
                    }
                    "verified" => {
                        accepted.push(Arc::new(AttenuatedMcpTool::new(tool)));
                    }
                    "community" => {
                        if is_community_safe(&name) {
                            accepted.push(Arc::new(AttenuatedMcpTool::new(tool)));
                        } else {
                            warn!(
                                "MCP tool '{}' rejected: community trust, name does not contain a safe keyword",
                                name
                            );
                        }
                    }
                    other => {
                        warn!(
                            "MCP tool '{}' rejected: unknown trust level '{}'",
                            name, other
                        );
                    }
                }
            }
            if !accepted.is_empty() {
                info!("Registered {} MCP tool(s)", accepted.len());
            }
            Some((accepted, manager))
        }
        Err(e) => {
            error!("MCP initialization failed: {}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_community_safe_keyword_matching() {
        // Read-only tool names should pass (snake_case, camelCase, PascalCase)
        assert!(is_community_safe("list_users"));
        assert!(is_community_safe("get_document"));
        assert!(is_community_safe("search_records"));
        assert!(is_community_safe("ReadConfig"));
        assert!(is_community_safe("fetchData"));
        assert!(is_community_safe("showStatus"));
        assert!(is_community_safe("count-items"));

        // Mutating tool names should be rejected
        assert!(!is_community_safe("delete_users"));
        assert!(!is_community_safe("create_record"));
        assert!(!is_community_safe("execute_command"));
        assert!(!is_community_safe("send_email"));

        // Substring false positives must be rejected (word-boundary check)
        assert!(!is_community_safe("breadcrumb")); // contains "read" substring
        assert!(!is_community_safe("overwrite")); // contains "view" substring
        assert!(!is_community_safe("altogether")); // contains "get" substring
    }

    #[test]
    fn test_builtin_tools_have_builtin_capability() {
        // Verify that all built-in tool types declare built_in: true
        use crate::agent::tools::filesystem::ReadFileTool;
        use crate::agent::tools::shell::ExecTool;
        use crate::agent::tools::web::WebSearchTool;

        assert!(ReadFileTool::new(None, None).capabilities().built_in);
        assert!(
            ExecTool::new(10, None, false, vec![], config::SandboxConfig::default())
                .unwrap()
                .capabilities()
                .built_in
        );
        assert!(WebSearchTool::new(None, 5).capabilities().built_in);
    }
}
