# Tool Crates Design

**Goal:** Extract 23 of 29 tools from the binary crate into 7 independent tool crates, reducing the binary crate from 28K lines of tools to ~5K.

## Interface

Each tool crate exports registration functions that take the specific params each tool needs. The binary crate's `setup/mod.rs` wires them with the right arguments.

```rust
// In oxicrab-tools-api crate
pub fn register_github(registry: &mut ToolRegistry, config: &GitHubConfig) { ... }
pub fn register_weather(registry: &mut ToolRegistry, config: &WeatherConfig) { ... }
```

No shared `ToolBuildContext` across crate boundaries. Each register function declares exactly what it needs.

## Crate Structure

### oxicrab-tools-web (4 tools, ~2,500 lines)
- HttpTool, RedditTool, WebSearchTool, WebFetchTool
- Depends on: `oxicrab-core`

### oxicrab-tools-api (5 tools, ~5,600 lines)
- GitHubTool, WeatherTool, TodoistTool, MediaTool, ImageGenTool
- Depends on: `oxicrab-core`

### oxicrab-tools-google (3 tools + common, ~2,800 lines)
- GoogleMailTool, GoogleCalendarTool, GoogleTasksTool, GoogleApiClient, google_common
- Depends on: `oxicrab-core`, `oxicrab-memory`

### oxicrab-tools-system (7 tools, ~3,500 lines)
- ReadFileTool, WriteFileTool, EditFileTool, ListDirTool, ExecTool, TmuxTool, WorkspaceTool
- Depends on: `oxicrab-core`

### oxicrab-tools-rss (1 tool, ~4,100 lines)
- RssTool (mod, articles, scanner, feeds, onboard, model, stats)
- Depends on: `oxicrab-core`, `oxicrab-memory`
- Feature-gated: `tool-rss`

### oxicrab-tools-browser (1 tool, ~1,100 lines)
- BrowserTool
- Depends on: `oxicrab-core`
- Feature-gated: `browser`

### oxicrab-tools-obsidian (1 tool + cache, ~1,900 lines)
- ObsidianTool, cache module
- Depends on: `oxicrab-core`, `oxicrab-memory`

### Stays in binary (~5,000 lines)
- MemorySearchTool, AddButtonsTool, ToolSearchTool, StashRetrieveTool
- SpawnTool, SubagentControlTool, CronTool
- MCP proxy + manager, ToolRegistry, setup/mod.rs, read_only_wrapper

## Execution Order

1. oxicrab-tools-web (simplest)
2. oxicrab-tools-system
3. oxicrab-tools-api
4. oxicrab-tools-google
5. oxicrab-tools-rss
6. oxicrab-tools-browser
7. oxicrab-tools-obsidian
8. Clean up setup/mod.rs
