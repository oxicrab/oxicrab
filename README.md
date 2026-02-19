<p align="center">
  <img src="docs/oxicrab.png" alt="oxicrab" width="200">
</p>

<h1 align="center">Oxicrab</h1>

<p align="center">A high-performance Rust multi-channel AI assistant framework.</p>

**[Documentation](https://oxicrab.github.io/oxicrab/)** | [Config](https://oxicrab.github.io/oxicrab/config.html) | [Channel Setup](https://oxicrab.github.io/oxicrab/channels.html) | [Tool Reference](https://oxicrab.github.io/oxicrab/tools.html) | [CLI Reference](https://oxicrab.github.io/oxicrab/cli.html) | [Deployment](https://oxicrab.github.io/oxicrab/deploy.html)

## Features

- **Multi-channel support**: Telegram, Discord (slash commands, embeds, button components), Slack, WhatsApp, Twilio (SMS/MMS) — each behind a Cargo feature flag for slim builds
- **LLM providers**: Anthropic (Claude), OpenAI (GPT), Google (Gemini), plus OpenAI-compatible providers (OpenRouter, DeepSeek, Groq, Ollama, Moonshot, Zhipu, DashScope, vLLM), with OAuth support and local model fallback
- **23 built-in tools**: Filesystem, shell, web, HTTP, browser automation, image generation, Google Workspace, GitHub, scheduling, memory, media management, and more — plus MCP (Model Context Protocol) for external tool servers
- **Subagents**: Background task execution with concurrency limiting, context injection, and lifecycle management
- **Cron scheduling**: Recurring jobs, one-shot timers, cron expressions, echo mode (LLM-free delivery), multi-channel targeting, auto-expiry (`expires_at`) and run limits (`max_runs`)
- **Memory system**: SQLite FTS5-backed long-term memory with background indexing, automatic fact extraction, optional hybrid vector+keyword search (local ONNX embeddings via fastembed), and automatic memory hygiene (archive/purge old notes)
- **Session management**: Persistent sessions with automatic compaction and context summarization
- **Hallucination detection**: Action claim detection, false no-tools-claim retry, and tool facts injection
- **Editable status messages**: Tool progress shown as a single message that edits in-place (Telegram, Discord, Slack), with composing indicator and automatic cleanup
- **Connection resilience**: All channels auto-reconnect with exponential backoff
- **Voice transcription**: Local whisper.cpp inference (via whisper-rs) with cloud API fallback, automatic audio conversion via ffmpeg
- **[CostGuard](https://oxicrab.github.io/oxicrab/config.html#cost-guard)**: Daily budget cap (cents) and hourly rate limiting with embedded pricing data for 50+ models, automatic midnight UTC reset
- **[Circuit breaker](https://oxicrab.github.io/oxicrab/config.html#circuit-breaker)**: Three-state circuit breaker (Closed/Open/HalfOpen) wraps the LLM provider, tripping only on transient errors (429, 5xx, timeout), with configurable threshold and recovery
- **[Cognitive routines](https://oxicrab.github.io/oxicrab/config.html#cognitive-routines)**: Escalating checkpoint pressure signals that nudge the LLM to self-summarize progress during long tool-heavy runs, preventing context loss during compaction
- **Doctor command**: Run [`oxicrab doctor`](https://oxicrab.github.io/oxicrab/cli.html#doctor) to check config, workspace, provider connectivity, channels, voice, external tools, and MCP servers
- **Credential management**: OS keyring (macOS Keychain, GNOME Keyring, Windows Credential Manager), external credential helpers (1Password, Bitwarden, custom scripts), and 28 `OXICRAB_*` env var overrides. Resolution: env → helper → keyring → config.json
- **Security**: Default-deny sender allowlists with DM pairing system, three-encoding leak detection (plaintext + base64 + hex), DNS rebinding defense (pinned DNS resolution), exfiltration guard (hide outbound tools from LLM), prompt injection detection (regex-based with warn/block modes), shell AST analysis (structural command validation via brush-parser), Landlock kernel sandbox (Linux filesystem/network isolation), shell command allowlist/blocklist, SSRF protection, path traversal prevention, secret redaction in logs, constant-time webhook signature validation
- **Async-first**: Built on Tokio for high-performance async I/O

## Installation

### Pre-built binaries

Download the latest release from [GitHub Releases](https://github.com/oxicrab/oxicrab/releases/latest):

| Platform | Archive |
|----------|---------|
| Linux x86_64 | `oxicrab-*-linux-x86_64.tar.gz` |
| Linux ARM64 (aarch64) | `oxicrab-*-linux-arm64.tar.gz` |
| macOS ARM64 (Apple Silicon) | `oxicrab-*-macos-arm64.tar.gz` |
| Debian/Ubuntu x86_64 | `oxicrab_*_amd64.deb` |
| Debian/Ubuntu ARM64 | `oxicrab_*_arm64.deb` |
| Fedora/RHEL x86_64 | `oxicrab-*.x86_64.rpm` |
| Fedora/RHEL ARM64 | `oxicrab-*.aarch64.rpm` |
| macOS ARM64 (DMG) | `oxicrab-*-arm64.dmg` |

```bash
# Tarball
tar xzf oxicrab-*-linux-x86_64.tar.gz
sudo cp oxicrab-*/oxicrab /usr/local/bin/

# Debian/Ubuntu
sudo dpkg -i oxicrab_*_amd64.deb

# Fedora/RHEL
sudo rpm -i oxicrab-*.x86_64.rpm

# macOS — open DMG and copy binary to /usr/local/bin
```

### Docker

```bash
docker pull ghcr.io/oxicrab/oxicrab:latest
docker run -v ~/.oxicrab:/home/oxicrab/.oxicrab ghcr.io/oxicrab/oxicrab
```

## Building

Each channel is behind a Cargo feature flag, so you can compile only what you need:

| Feature | Description | Default |
|---------|-------------|---------|
| `channel-telegram` | Telegram (teloxide) | Yes |
| `channel-discord` | Discord (serenity) | Yes |
| `channel-slack` | Slack (tokio-tungstenite) | Yes |
| `channel-whatsapp` | WhatsApp (whatsapp-rust) | Yes |
| `channel-twilio` | Twilio SMS/MMS (axum webhook) | Yes |
| `keyring-store` | OS keychain credential storage (keyring crate) | Yes |

```bash
# Full build (all channels)
cargo build --release

# Slim build — only Telegram and Slack
cargo build --release --no-default-features --features channel-telegram,channel-slack

# No channels (agent CLI only)
cargo build --release --no-default-features
```

## Configuration

Configuration is stored in `~/.oxicrab/config.json`. Create this file with the following structure:

```json
{
  "agents": {
    "defaults": {
      "workspace": "~/.oxicrab/workspace",
      "model": "claude-sonnet-4-5-20250929",
      "maxTokens": 8192,
      "temperature": 0.7,
      "maxToolIterations": 20,
      "sessionTtlDays": 30,
      "memoryIndexerInterval": 300,
      "mediaTtlDays": 7,
      "maxConcurrentSubagents": 5,
      "memory": {
        "archiveAfterDays": 30,
        "purgeAfterDays": 90,
        "embeddingsEnabled": false,
        "embeddingsModel": "BAAI/bge-small-en-v1.5",
        "hybridWeight": 0.5
      },
      "compaction": {
        "enabled": true,
        "thresholdTokens": 40000,
        "keepRecent": 10,
        "extractionEnabled": true
      },
      "costGuard": {
        "dailyBudgetCents": 500,
        "maxActionsPerHour": 100
      },
      "daemon": {
        "enabled": true,
        "interval": 300,
        "strategyFile": "HEARTBEAT.md",
        "maxIterations": 25
      }
    }
  },
  "providers": {
    "anthropic": {
      "apiKey": "your-anthropic-api-key"
    },
    "openai": {
      "apiKey": "your-openai-api-key"
    },
    "gemini": {
      "apiKey": "your-gemini-api-key"
    },
    "deepseek": {
      "apiKey": "your-deepseek-api-key"
    },
    "groq": {
      "apiKey": "your-groq-api-key"
    },
    "openrouter": {
      "apiKey": "your-openrouter-api-key"
    },
    "circuitBreaker": {
      "enabled": false,
      "failureThreshold": 5,
      "recoveryTimeoutSecs": 60,
      "halfOpenProbes": 2
    }
  },
  "channels": {
    "telegram": {
      "enabled": true,
      "token": "your-telegram-bot-token",
      "allowFrom": ["user_id1", "user_id2"]
    },
    "discord": {
      "enabled": true,
      "token": "your-discord-bot-token",
      "allowFrom": ["user_id1", "user_id2"],
      "commands": [
        {
          "name": "ask",
          "description": "Ask the AI assistant",
          "options": [{ "name": "question", "description": "Your question", "required": true }]
        }
      ]
    },
    "slack": {
      "enabled": true,
      "botToken": "xoxb-your-bot-token",
      "appToken": "xapp-your-app-token",
      "allowFrom": ["user_id1", "user_id2"]
    },
    "whatsapp": {
      "enabled": true,
      "allowFrom": ["phone_number1", "phone_number2"]
    },
    "twilio": {
      "enabled": true,
      "accountSid": "your-twilio-account-sid",
      "authToken": "your-twilio-auth-token",
      "phoneNumber": "+15551234567",
      "webhookPort": 8080,
      "webhookPath": "/twilio/webhook",
      "webhookUrl": "https://your-server.example.com/twilio/webhook",
      "allowFrom": []
    }
  },
  "tools": {
    "google": {
      "enabled": true,
      "clientId": "your-google-client-id",
      "clientSecret": "your-google-client-secret"
    },
    "github": {
      "enabled": true,
      "token": "ghp_your-github-token"
    },
    "weather": {
      "enabled": true,
      "apiKey": "your-openweathermap-api-key"
    },
    "todoist": {
      "enabled": true,
      "token": "your-todoist-api-token"
    },
    "web": {
      "search": {
        "provider": "brave",
        "apiKey": "your-brave-search-api-key"
      }
    },
    "media": {
      "enabled": true,
      "radarr": {
        "url": "http://localhost:7878",
        "apiKey": "your-radarr-api-key"
      },
      "sonarr": {
        "url": "http://localhost:8989",
        "apiKey": "your-sonarr-api-key"
      }
    },
    "obsidian": {
      "enabled": true,
      "apiUrl": "http://localhost:27123",
      "apiKey": "your-obsidian-local-rest-api-key",
      "vaultName": "MyVault",
      "syncInterval": 300,
      "timeout": 15
    },
    "browser": {
      "enabled": false,
      "headless": true,
      "chromePath": null,
      "timeout": 30
    },
    "exec": {
      "timeout": 60,
      "allowedCommands": ["ls", "grep", "git", "cargo"]
    },
    "mcp": {
      "servers": {
        "filesystem": {
          "command": "npx",
          "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
          "enabled": true
        }
      }
    },
    "restrictToWorkspace": true
  },
  "voice": {
    "transcription": {
      "enabled": true,
      "localModelPath": "~/.oxicrab/models/ggml-large-v3-turbo-q5_0.bin",
      "preferLocal": true,
      "threads": 4,
      "apiKey": "your-groq-or-openai-api-key",
      "apiBase": "https://api.groq.com/openai/v1/audio/transcriptions",
      "model": "whisper-large-v3-turbo"
    }
  }
}
```

### Credential Management

> **Full credential reference (resolution order, env vars, helpers, keyring):** [oxicrab.github.io/oxicrab/config.html#credentials](https://oxicrab.github.io/oxicrab/config.html#credentials)

Resolution order: env vars (`OXICRAB_*`) → credential helper (1Password, Bitwarden, custom) → OS keyring → config.json. All 28 credential slots have env var overrides. Use `oxicrab credentials list` to see where each credential comes from.

## Channel Setup

> **Step-by-step setup guides for each channel:** [oxicrab.github.io/oxicrab/channels.html](https://oxicrab.github.io/oxicrab/channels.html)

| Channel | What you need | Config key |
|---------|--------------|------------|
| **Telegram** | Bot token from [@BotFather](https://t.me/botfather) + your user ID | `channels.telegram` |
| **Discord** | Bot token + Message Content Intent enabled + server invite | `channels.discord` |
| **Slack** | Bot token (`xoxb-`) + Socket Mode app token (`xapp-`) | `channels.slack` |
| **WhatsApp** | Just enable — scan QR code on first run | `channels.whatsapp` |
| **Twilio** | Account SID + Auth Token + phone number + webhook URL | `channels.twilio` |

Each channel has a `dmPolicy` that controls access: `"allowlist"` (default — silently drop unknown senders), `"pairing"` (send a pairing code so unknown senders can request access), or `"open"` (allow everyone). The `allowFrom` list specifies pre-authorized sender IDs. Empty `allowFrom` defaults to **deny-all** — use `["*"]` to allow all senders, or use `oxicrab pairing approve` to onboard specific users.

## Running

> **Full CLI reference with all flags and examples:** [oxicrab.github.io/oxicrab/cli.html](https://oxicrab.github.io/oxicrab/cli.html)

```bash
# First-time setup — creates config and workspace
oxicrab onboard

# Start the multi-channel gateway daemon
oxicrab gateway

# Interactive agent REPL / single message
oxicrab agent
oxicrab agent -m "What's the weather?"

# Cron scheduling — agent mode or echo mode
oxicrab cron list
oxicrab cron add -n "Briefing" -m "Morning briefing" -c "0 9 * * *" --all-channels
oxicrab cron run --id abc12345 --force

# Channel management
oxicrab channels status
oxicrab channels login           # WhatsApp QR code pairing

# Credentials — resolution: env vars → helper → OS keyring → config.json
oxicrab credentials set anthropic-api-key
oxicrab credentials list
oxicrab credentials import       # bulk-migrate config.json → keyring

# Sender access control (DM pairing)
oxicrab pairing list
oxicrab pairing approve ABC12345
oxicrab pairing revoke telegram 123456789

# Authentication
oxicrab auth google              # OAuth2 for Gmail + Calendar

# Diagnostics
oxicrab status                   # quick setup overview
oxicrab doctor                   # full system health check
```

### Voice Transcription

> **Setup instructions and config options:** [oxicrab.github.io/oxicrab/tools.html#voice-transcription](https://oxicrab.github.io/oxicrab/tools.html#voice-transcription)

Voice messages from channels are automatically transcribed to text. Two backends: **local** (whisper.cpp via `whisper-rs`, requires `ffmpeg` + GGML model) and **cloud** (Whisper API via Groq/OpenAI). Either alone is sufficient; configure both for automatic fallback. Routing controlled by `preferLocal` (default `true`) under `voice.transcription`.

### Logging

> **Logging configuration and RUST_LOG examples:** [oxicrab.github.io/oxicrab/config.html#logging](https://oxicrab.github.io/oxicrab/config.html#logging)

Controlled by `RUST_LOG` env var. Default is info level with noisy dependencies suppressed.

## Model Configuration

> **Full model configuration reference (API keys, OpenAI-compatible providers, local fallback, OAuth):** [oxicrab.github.io/oxicrab/config.html#models](https://oxicrab.github.io/oxicrab/config.html#models)

Set the model in `agents.defaults.model` and the API key under `providers`. Supports Anthropic (Claude), OpenAI (GPT), Google (Gemini), plus 8 OpenAI-compatible providers (OpenRouter, DeepSeek, Groq, Ollama, Moonshot, Zhipu, DashScope, vLLM). Local model fallback available via `localModel`. OAuth supported for `anthropic/`-prefixed models.

## Tools

> **Full tool reference with all actions, parameters, and setup instructions:** [oxicrab.github.io/oxicrab/tools.html](https://oxicrab.github.io/oxicrab/tools.html)

23 built-in tools plus MCP. Every tool has timeout protection, panic isolation, result caching, and truncation middleware.

**Core** (always available): `read_file`, `write_file`, `edit_file`, `list_dir`, `exec`, `tmux`, `web_search`, `web_fetch`, `http`, `spawn`, `subagent_control`, `cron`, `memory_search`, `reddit`

**Configurable** (require setup): `google_mail`, `google_calendar`, `github`, `weather`, `todoist`, `media`, `obsidian`, `browser`, `image_gen`

### MCP (Model Context Protocol)

> **Full MCP reference:** [oxicrab.github.io/oxicrab/tools.html#mcp](https://oxicrab.github.io/oxicrab/tools.html#mcp)

Oxicrab supports connecting to external tool servers via the [Model Context Protocol](https://modelcontextprotocol.io/). Each MCP server's tools are automatically discovered and registered as native tools in the agent.

```json
"tools": {
  "mcp": {
    "servers": {
      "filesystem": {
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user/documents"],
        "enabled": true
      },
      "git": {
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-git"],
        "env": { "GIT_DIR": "/path/to/repo" },
        "enabled": true
      }
    }
  }
}
```

Each server config supports:
- **`command`**: The executable to run (e.g. `npx`, `python`, a binary path)
- **`args`**: Command-line arguments
- **`env`**: Environment variables passed to the child process
- **`enabled`**: Set to `false` to skip without removing the config

### Subagent System

The agent can spawn background subagents to handle complex tasks in parallel:

- **Concurrency limiting**: Configurable max concurrent subagents (default 5) via semaphore
- **Context injection**: Subagents receive the parent conversation's compaction summary so they understand what was discussed
- **Silent mode**: Internal spawns (from cron/daemon) can skip user-facing announcements
- **Lifecycle management**: List running subagents, check capacity, cancel by ID
- **Tool isolation**: Subagents get filesystem, shell, and web tools but cannot spawn more subagents
- **Parallel tool execution**: Subagent tool calls run in parallel (same pattern as the main agent loop)

## Workspace Structure

> **Workspace file reference:** [oxicrab.github.io/oxicrab/workspace.html](https://oxicrab.github.io/oxicrab/workspace.html)

```
~/.oxicrab/
├── config.json              # Main configuration
├── workspace/
│   ├── AGENTS.md            # Bot identity, personality, and behavioral rules
│   ├── USER.md              # User preferences
│   ├── TOOLS.md             # Tool usage guide
│   ├── memory/
│   │   ├── MEMORY.md        # Long-term memory
│   │   ├── memory.sqlite3   # FTS5 search index
│   │   └── YYYY-MM-DD.md    # Daily notes (auto-extracted facts)
│   ├── sessions/            # Conversation sessions (per channel:chat_id)
│   └── skills/              # Custom skills (SKILL.md per skill)
├── models/                  # Whisper model files (e.g. ggml-large-v3-turbo-q5_0.bin)
├── backups/                 # Automatic file backups (up to 14 versions)
├── cron/
│   └── jobs.json            # Scheduled jobs
├── google_tokens.json       # Google OAuth tokens
└── whatsapp/
    └── whatsapp.db          # WhatsApp session storage
```

## Project Structure

```
src/
├── agent/          # Agent loop, context, memory, tools, subagents, compaction, skills
├── auth/           # OAuth authentication (Google)
├── bus/            # Message bus for channel-agent communication
├── channels/       # Channel implementations (Telegram, Discord, Slack, WhatsApp, Twilio)
├── cli/            # Command-line interface
├── config/         # Configuration schema and loader
├── cron/           # Cron job scheduling service
├── heartbeat/      # Heartbeat/daemon service
├── providers/      # LLM provider implementations (Anthropic, OpenAI, Gemini, OpenAI-compatible)
├── session/        # Session management with SQLite backend
├── errors.rs       # OxicrabError typed error enum
└── utils/          # URL security, atomic writes, task tracking, voice transcription, media file handling
```

## Architecture

- **Async-first**: Built on `tokio` for high-performance async I/O
- **Cargo feature flags**: Each channel is a compile-time feature (`channel-telegram`, `channel-discord`, `channel-slack`, `channel-whatsapp`, `channel-twilio`), allowing slim builds without unused dependencies
- **Message bus**: Decoupled channel-agent communication via inbound/outbound message bus
- **Connection resilience**: All channels (Telegram, Discord, Slack, WhatsApp, Twilio) use exponential backoff retry loops for automatic reconnection after disconnects
- **Channel edit/delete**: `BaseChannel` trait provides `send_and_get_id`, `edit_message`, and `delete_message` with default no-ops; implemented for Telegram, Discord, and Slack
- **Discord interactions**: Slash commands (configurable via `commands` config), button component handling, rich embeds, and interaction webhook followups. Metadata keys propagate interaction tokens through the agent loop for deferred responses
- **Session management**: SQLite-backed sessions with automatic TTL cleanup
- **Memory**: SQLite FTS5 for semantic memory indexing with background indexer, automatic fact extraction, optional hybrid vector+keyword search via local ONNX embeddings (fastembed), and automatic memory hygiene (archive old notes, purge expired archives, clean orphaned entries)
- **Compaction**: Automatic conversation summarization when context exceeds token threshold
- **Outbound media**: Browser screenshots, image downloads (`web_fetch`, `http`), and binary responses are saved to `~/.oxicrab/media/` and attached to outbound messages automatically. Supported channels: Telegram (photos/documents), Discord (file attachments), Slack (3-step file upload API). WhatsApp and Twilio log warnings for unsupported outbound media.
- **Tool execution**: Middleware pipeline (`CacheMiddleware` → `TruncationMiddleware` → `LoggingMiddleware`) in `ToolRegistry`, panic-isolated via `tokio::task::spawn`, parallel execution via `join_all`, LRU result caching for read-only tools, pre-execution JSON schema validation
- **MCP integration**: External tool servers connected via Model Context Protocol (`rmcp` crate). Tools auto-discovered at startup and registered as native tools
- **Tool facts injection**: Each agent turn injects a reminder listing all available tools, preventing the LLM from falsely claiming tools are unavailable
- **Editable status messages**: Tool execution progress shown as a single message that edits in-place rather than flooding the chat. Tracks status per (channel, chat_id), accumulates tool status lines with emoji prefixes, adds a "Composing response..." indicator during LLM thinking, and deletes the status message when the final response arrives. Channels without edit support (WhatsApp) fall back to separate messages.
- **Subagents**: Semaphore-limited background task execution with conversation context injection and parallel tool calls
- **Cron**: File-backed job store with multi-channel target delivery, agent mode and echo mode, timezone auto-detection, auto-expiry (`expires_at`), run limits (`max_runs`), and automatic name deduplication
- **Heartbeat/Daemon**: Periodic background check-ins driven by a strategy file (`HEARTBEAT.md`)
- **Voice transcription**: Dual-backend transcription service (local whisper.cpp via `whisper-rs` + cloud Whisper API). Audio converted to 16kHz mono f32 PCM via ffmpeg subprocess; local inference runs on a blocking thread pool. Configurable routing (`preferLocal`) with automatic fallback between backends.
- **Skills**: Extensible via workspace SKILL.md files with YAML frontmatter, dependency checking, and auto-include
- **CostGuard**: Pre-flight budget check (`check_allowed()`) and post-flight cost recording (`record_llm_call()`). Daily budget in cents with midnight UTC reset, hourly rate limiting via sliding window, embedded pricing for 50+ models with config overrides, AtomicBool fast-path for exceeded budgets. Config under `agents.defaults.costGuard`
- **Circuit breaker**: `CircuitBreakerProvider` wraps `Arc<dyn LLMProvider>`. Three states: Closed (normal), Open (rejecting — after N consecutive transient failures), HalfOpen (probing). Non-transient errors (auth, invalid key, permission) do not trip the breaker. Config under `providers.circuitBreaker`
- **Hallucination detection**: Regex-based action claim detection, tool-name mention counting, and false no-tools-claim detection with automatic retry prevent the LLM from fabricating actions or denying tool access; first-iteration forced tool use and tools nudge (up to 2 retries) prevent text-only hallucinations
- **Security**: Shell command allowlist + blocklist with pipe/chain operator parsing; SSRF protection blocking private IPs, loopback, and metadata endpoints; path traversal prevention; OAuth credential file permissions (0o600); config secret redaction in Debug impls; outbound message scanning for leaked API keys with automatic redaction; atomic config file writes to prevent corruption; config file permission checks on startup (warns if world-readable); Twilio webhook signature validation using constant-time comparison

## Development

### Prerequisites

- Rust (nightly toolchain required for WhatsApp support)
- CMake and a C++ compiler (required to build whisper.cpp via `whisper-rs`)
- SQLite (bundled via `rusqlite`)
- ffmpeg (required for voice transcription audio conversion)

### Setting up Nightly Rust

WhatsApp support requires nightly Rust:

```bash
rustup toolchain install nightly
rustup override set nightly
```

### Running Tests

```bash
cargo test
```

## License

MIT
