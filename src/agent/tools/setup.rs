use crate::agent::memory::MemoryStore;
use crate::agent::subagent::{SubagentConfig, SubagentManager};
use crate::agent::tools::ToolRegistry;
use crate::agent::tools::mcp::McpManager;
use crate::agent::tools::mcp::proxy::AttenuatedMcpTool;
use crate::bus::{MessageBus, OutboundMessage};
use crate::config;
use crate::cron::service::CronService;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// Built-in tool names that MCP tools must not shadow.
const PROTECTED_TOOL_NAMES: &[&str] = &[
    "exec",
    "read_file",
    "write_file",
    "edit_file",
    "list_dir",
    "web_search",
    "web_fetch",
    "http",
    "spawn",
    "subagent_control",
    "cron",
    "tmux",
    "browser",
    "memory_search",
];

/// Keywords that indicate a tool is safe for community-trust MCP servers.
const COMMUNITY_SAFE_KEYWORDS: &[&str] = &[
    "read", "list", "get", "search", "find", "query", "fetch", "view", "show", "count",
];

/// All configuration and shared state needed to construct tools.
/// Built once during `AgentLoop::new()` and passed to each module's `register()`.
pub struct ToolBuildContext {
    pub workspace: PathBuf,
    pub restrict_to_workspace: bool,
    pub exec_timeout: u64,
    pub allowed_commands: Vec<String>,
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
}

/// Register all tools into the registry using decentralized per-module `register()` functions.
/// Returns `(ToolRegistry, SubagentManager)`.
pub async fn register_all_tools(
    ctx: &ToolBuildContext,
) -> Result<(ToolRegistry, Arc<SubagentManager>)> {
    let mut tools = ToolRegistry::new();

    register_filesystem(&mut tools, ctx);
    register_shell(&mut tools, ctx)?;
    register_web(&mut tools, ctx);
    let subagents = register_subagents(&mut tools, ctx);
    register_tmux(&mut tools);
    register_browser(&mut tools, ctx);
    register_image_gen(&mut tools, ctx);
    register_cron(&mut tools, ctx);
    register_google(&mut tools, ctx).await;
    register_github(&mut tools, ctx);
    register_weather(&mut tools, ctx);
    register_todoist(&mut tools, ctx);
    register_media(&mut tools, ctx);
    register_obsidian(&mut tools, ctx);
    register_http(&mut tools);
    register_reddit(&mut tools);
    register_memory_search(&mut tools, ctx);
    register_mcp(&mut tools, ctx).await;

    Ok((tools, subagents))
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

    registry.register(Arc::new(ReadFileTool::new(allowed_roots.clone())));
    registry.register(Arc::new(WriteFileTool::new(
        allowed_roots.clone(),
        backup_dir.clone(),
    )));
    registry.register(Arc::new(EditFileTool::new(
        allowed_roots.clone(),
        backup_dir,
    )));
    registry.register(Arc::new(ListDirTool::new(allowed_roots)));
}

fn register_shell(registry: &mut ToolRegistry, ctx: &ToolBuildContext) -> Result<()> {
    use crate::agent::tools::shell::ExecTool;

    registry.register(Arc::new(ExecTool::new(
        ctx.exec_timeout,
        Some(ctx.workspace.clone()),
        ctx.restrict_to_workspace,
        ctx.allowed_commands.clone(),
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
        )));
    }
}

async fn register_google(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::google_calendar::GoogleCalendarTool;
    use crate::agent::tools::google_mail::GoogleMailTool;

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
                registry.register(Arc::new(GoogleMailTool::new(creds.clone())));
                registry.register(Arc::new(GoogleCalendarTool::new(creds)));
                info!("Google tools registered (gmail, calendar)");
            }
            Err(e) => {
                warn!("Google tools not available: {}", e);
            }
        }
    }
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

async fn register_mcp(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    if let Some(ref mcp_cfg) = ctx.mcp_config {
        if mcp_cfg.servers.is_empty() {
            return;
        }
        match McpManager::new(mcp_cfg).await {
            Ok(manager) => {
                let tools = manager.discover_tools().await;
                let mut registered = 0usize;
                for (trust, tool) in tools {
                    let name = tool.name().to_string();

                    // Reject tools that shadow built-in names
                    if PROTECTED_TOOL_NAMES.contains(&name.as_str()) {
                        warn!(
                            "MCP tool '{}' rejected: shadows a protected built-in tool",
                            name
                        );
                        continue;
                    }

                    match trust.as_str() {
                        "local" => {
                            registry.register(tool);
                            registered += 1;
                        }
                        "verified" => {
                            registry.register(Arc::new(AttenuatedMcpTool::new(tool)));
                            registered += 1;
                        }
                        "community" => {
                            let name_lower = name.to_lowercase();
                            if COMMUNITY_SAFE_KEYWORDS
                                .iter()
                                .any(|kw| name_lower.contains(kw))
                            {
                                registry.register(Arc::new(AttenuatedMcpTool::new(tool)));
                                registered += 1;
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
                if registered > 0 {
                    info!("Registered {} MCP tool(s)", registered);
                }
                // Store manager so child processes stay alive.
                // We leak it intentionally — MCP servers run for the process lifetime.
                // The processes will be killed when the oxicrab process exits.
                std::mem::forget(manager);
            }
            Err(e) => {
                error!("MCP initialization failed: {}", e);
            }
        }
    }
}
