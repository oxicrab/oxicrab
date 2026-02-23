<p align="center">
  <img src="docs/oxicrab.png" alt="oxicrab" width="200">
</p>

<h1 align="center">Oxicrab</h1>

<p align="center">A high-performance Rust multi-channel AI assistant framework.</p>

**[Documentation](https://oxicrab.github.io/oxicrab/)** | [Config](https://oxicrab.github.io/oxicrab/config.html) | [Channel Setup](https://oxicrab.github.io/oxicrab/channels.html) | [Tool Reference](https://oxicrab.github.io/oxicrab/tools.html) | [CLI Reference](https://oxicrab.github.io/oxicrab/cli.html) | [Deployment](https://oxicrab.github.io/oxicrab/deploy.html)

## Motives

This is largely a personal toy with features I want or care about. For example, I only included channels that matter to me. The inspiration was playing with OpenClaw and deciding that Rust made more sense as a platform for this. I was also curious how easy it would be to harden the bot. So the normal caveats apply ... no warranties, no guarantees, etc. 

## Features

- **Multi-channel**: Telegram, Discord (slash commands, embeds, buttons), Slack, WhatsApp, Twilio SMS/MMS
- **LLM providers**: Anthropic (Claude), OpenAI, Google (Gemini), plus 8 OpenAI-compatible providers (OpenRouter, DeepSeek, Groq, Ollama, etc.), with OAuth and local model fallback
- **Prompt caching**: Automatic Anthropic `cache_control` injection for up to 90% input token cost reduction
- **23 built-in tools**: Filesystem, shell, web, HTTP, browser, image generation, Google Workspace, GitHub, scheduling, memory, media, and more
- **MCP support**: Connect external tool servers via the Model Context Protocol
- **Subagents**: Background task execution with concurrency limiting and context injection
- **Cron scheduling**: Recurring jobs, one-shot timers, cron expressions, echo mode, multi-channel targeting
- **Memory system**: SQLite FTS5 with background indexing, optional hybrid vector+keyword search (local ONNX embeddings), configurable fusion strategy (weighted score or reciprocal rank fusion), knowledge directory for RAG document ingestion, and automatic memory hygiene
- **Group chat isolation**: Personal memory (MEMORY.md, daily notes) automatically excluded from group chat contexts; knowledge shared across all contexts
- **Session management**: Persistent sessions with automatic compaction and context summarization
- **Voice transcription**: Local whisper.cpp with cloud API fallback
- **CostGuard**: Daily budget cap and hourly rate limiting with embedded pricing for 50+ models
- **HTTP gateway**: REST API (`POST /api/chat`, `GET /api/health`) and named webhook receivers with HMAC-SHA256 validation, template formatting, and multi-channel delivery
- **JSON mode**: Per-request structured output (JSON object and JSON schema) across all providers
- **PDF/document support**: Native PDF document support in Anthropic, OpenAI, and Gemini providers
- **Security**: Default-deny allowlists, DM pairing, leak detection, DNS rebinding defense, kernel-level sandbox (Landlock/Seatbelt), shell AST analysis, prompt injection detection, capability-based filesystem confinement

## Installation

### Pre-built binaries

Download from [GitHub Releases](https://github.com/oxicrab/oxicrab/releases/latest):

| Platform | Archive |
|----------|---------|
| Linux x86_64 | `oxicrab-*-linux-x86_64.tar.gz` |
| Linux ARM64 | `oxicrab-*-linux-arm64.tar.gz` |
| macOS ARM64 | `oxicrab-*-macos-arm64.tar.gz` |
| Debian/Ubuntu | `oxicrab_*_amd64.deb` / `oxicrab_*_arm64.deb` |
| Fedora/RHEL | `oxicrab-*.x86_64.rpm` / `oxicrab-*.aarch64.rpm` |
| macOS DMG | `oxicrab-*-arm64.dmg` |

### Docker

```bash
docker pull ghcr.io/oxicrab/oxicrab:latest
docker run -v ~/.oxicrab:/home/oxicrab/.oxicrab ghcr.io/oxicrab/oxicrab
```

## Building

Each channel is a Cargo feature flag for slim builds:

```bash
# Full build (all channels)
cargo build --release

# Slim build — only Telegram and Slack
cargo build --release --no-default-features --features channel-telegram,channel-slack

# No channels (agent CLI only)
cargo build --release --no-default-features
```

Features: `channel-telegram`, `channel-discord`, `channel-slack`, `channel-whatsapp`, `channel-twilio`, `keyring-store` (all default-on).

## Quick Start

```bash
# First-time setup
oxicrab onboard

# Start the multi-channel gateway
oxicrab gateway

# Single message (CLI mode)
oxicrab agent -m "What's the weather?"
```

> **Full CLI reference:** [oxicrab.github.io/oxicrab/cli.html](https://oxicrab.github.io/oxicrab/cli.html)

## Configuration

Configuration lives at `~/.oxicrab/config.json`. Run `oxicrab onboard` for guided setup, or see the [full config reference](https://oxicrab.github.io/oxicrab/config.html).

Minimal example:

```json
{
  "agents": {
    "defaults": {
      "model": "claude-sonnet-4-5-20250929"
    }
  },
  "providers": {
    "anthropic": { "apiKey": "sk-ant-..." }
  },
  "channels": {
    "telegram": {
      "enabled": true,
      "token": "your-bot-token",
      "allowFrom": ["your-user-id"]
    }
  }
}
```

### Credential Management

Resolution order: env vars (`OXICRAB_*`) > credential helper (1Password, Bitwarden) > OS keyring > config.json.

```bash
oxicrab credentials set anthropic-api-key
oxicrab credentials list
oxicrab credentials import   # bulk-migrate config.json to keyring
```

> **Full credential reference:** [oxicrab.github.io/oxicrab/config.html#credentials](https://oxicrab.github.io/oxicrab/config.html#credentials)

## Channels

> **Step-by-step setup guides:** [oxicrab.github.io/oxicrab/channels.html](https://oxicrab.github.io/oxicrab/channels.html)

| Channel | What you need |
|---------|--------------|
| **Telegram** | Bot token from [@BotFather](https://t.me/botfather) + user ID |
| **Discord** | Bot token + Message Content Intent + server invite |
| **Slack** | Bot token (`xoxb-`) + Socket Mode app token (`xapp-`) |
| **WhatsApp** | Just enable — scan QR code on first run |
| **Twilio** | Account SID + Auth Token + phone number + webhook URL |

Access control: `allowFrom` (pre-authorized senders), `dmPolicy` (`"allowlist"`, `"pairing"`, or `"open"`). Empty `allowFrom` = deny all.

## Tools

> **Full tool reference:** [oxicrab.github.io/oxicrab/tools.html](https://oxicrab.github.io/oxicrab/tools.html)

23 built-in tools with timeout protection, panic isolation, result caching, and truncation middleware.

**Core**: `read_file`, `write_file`, `edit_file`, `list_dir`, `exec`, `tmux`, `web_search`, `web_fetch`, `http`, `spawn`, `subagent_control`, `cron`, `memory_search`, `reddit`

**Configurable**: `google_mail`, `google_calendar`, `github`, `weather`, `todoist`, `media`, `obsidian`, `browser`, `image_gen`

**MCP**: Connect external tool servers via [Model Context Protocol](https://modelcontextprotocol.io/). See [MCP reference](https://oxicrab.github.io/oxicrab/tools.html#mcp).

## Workspace

```
~/.oxicrab/
├── config.json              # Main configuration
├── workspace/
│   ├── AGENTS.md            # Bot identity and behavioral rules
│   ├── USER.md              # User preferences
│   ├── TOOLS.md             # Tool usage guide
│   ├── memory/
│   │   ├── MEMORY.md        # Long-term memory
│   │   ├── memory.sqlite3   # FTS5 search index + embeddings
│   │   └── YYYY-MM-DD.md    # Daily notes (auto-extracted facts)
│   ├── knowledge/           # RAG document ingestion (.md, .txt, .html)
│   ├── sessions/            # Conversation sessions
│   └── skills/              # Custom skills (SKILL.md per skill)
├── models/                  # Whisper model files
└── cron/jobs.json           # Scheduled jobs
```

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed implementation documentation.

```
Channel → MessageBus → AgentLoop (LLM ↔ tools) → MessageBus → Channel
```

## Development

Requires **Rust nightly** and `cmake`. Voice transcription also requires `ffmpeg`.

```bash
cargo test --lib                    # unit tests
cargo fmt -- --check                # formatting
cargo clippy --all-targets --all-features -- -D warnings   # linting
```

See [CLAUDE.md](CLAUDE.md) for development conventions.

## License

MIT
