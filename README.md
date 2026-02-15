# Nanobot Rust

A high-performance Rust implementation of the nanobot AI assistant framework with multi-channel support.

## Features

- **Multi-channel support**: Telegram, Discord, Slack, WhatsApp, Twilio (SMS/MMS) — each behind a Cargo feature flag for slim builds
- **LLM providers**: Anthropic (Claude), OpenAI (GPT), Google (Gemini), plus OpenAI-compatible providers (OpenRouter, DeepSeek, Groq, Ollama, Moonshot, Zhipu, DashScope, vLLM), with OAuth support and local model fallback
- **23 built-in tools**: Filesystem, shell, web, HTTP, browser automation, Google Workspace, GitHub, scheduling, memory, media management, and more
- **Subagents**: Background task execution with concurrency limiting, context injection, and lifecycle management
- **Cron scheduling**: Recurring jobs, one-shot timers, cron expressions, echo mode (LLM-free delivery), multi-channel targeting, auto-expiry (`expires_at`) and run limits (`max_runs`)
- **Memory system**: SQLite FTS5-backed long-term memory with background indexing, automatic fact extraction, optional hybrid vector+keyword search (local ONNX embeddings via fastembed), and automatic memory hygiene (archive/purge old notes)
- **Session management**: Persistent sessions with automatic compaction and context summarization
- **Hallucination detection**: Action claim detection, false no-tools-claim retry, tool facts injection, and reflection turns
- **Editable status messages**: Tool progress shown as a single message that edits in-place (Telegram, Discord, Slack), with composing indicator and automatic cleanup
- **Connection resilience**: All channels auto-reconnect with exponential backoff
- **Voice transcription**: Local whisper.cpp inference (via whisper-rs) with cloud API fallback, automatic audio conversion via ffmpeg
- **Security**: Shell command allowlist/blocklist, SSRF protection, path traversal prevention, secret redaction
- **Async-first**: Built on Tokio for high-performance async I/O

## Building

Each channel is behind a Cargo feature flag, so you can compile only what you need:

| Feature | Channel | Default |
|---------|---------|---------|
| `channel-telegram` | Telegram (teloxide) | Yes |
| `channel-discord` | Discord (serenity) | Yes |
| `channel-slack` | Slack (tokio-tungstenite) | Yes |
| `channel-whatsapp` | WhatsApp (whatsapp-rust) | Yes |
| `channel-twilio` | Twilio SMS/MMS (axum webhook) | Yes |

```bash
# Full build (all channels)
cargo build --release

# Slim build — only Telegram and Slack
cargo build --release --no-default-features --features channel-telegram,channel-slack

# No channels (agent CLI only)
cargo build --release --no-default-features
```

## Configuration

Configuration is stored in `~/.nanobot/config.json`. Create this file with the following structure:

```json
{
  "agents": {
    "defaults": {
      "workspace": "~/.nanobot/workspace",
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
      "allowFrom": ["user_id1", "user_id2"]
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
    "restrictToWorkspace": true
  },
  "voice": {
    "transcription": {
      "enabled": true,
      "localModelPath": "~/.nanobot/models/ggml-large-v3-turbo-q5_0.bin",
      "preferLocal": true,
      "threads": 4,
      "apiKey": "your-groq-or-openai-api-key",
      "apiBase": "https://api.groq.com/openai/v1/audio/transcriptions",
      "model": "whisper-large-v3-turbo"
    }
  }
}
```

## Channel Setup

### Telegram

1. **Create a bot**:
   - Message [@BotFather](https://t.me/botfather) on Telegram
   - Use `/newbot` command and follow instructions
   - Copy the bot token

2. **Get your user ID**:
   - Message [@userinfobot](https://t.me/userinfobot) to get your Telegram user ID
   - Or use [@getidsbot](https://t.me/getidsbot)

3. **Configure**:
   ```json
   "telegram": {
     "enabled": true,
     "token": "123456789:ABCdefGHIjklMNOpqrsTUVwxyz",
     "allowFrom": ["123456789"],
     "proxy": null
   }
   ```

### Discord

1. **Create a bot**:
   - Go to https://discord.com/developers/applications
   - Click "New Application"
   - Go to "Bot" section
   - Click "Add Bot"
   - Under "Token", click "Reset Token" and copy it
   - Enable "Message Content Intent" under "Privileged Gateway Intents"

2. **Invite bot to server**:
   - Go to "OAuth2" > "URL Generator"
   - Select scopes: `bot`, `applications.commands`
   - Select bot permissions: `Send Messages`, `Read Message History`
   - Copy the generated URL and open it in browser
   - Select your server and authorize

3. **Get user/channel IDs**:
   - Enable Developer Mode in Discord (Settings > Advanced > Developer Mode)
   - Right-click on users/channels and select "Copy ID"

4. **Configure**:
   ```json
   "discord": {
     "enabled": true,
     "token": "your-discord-bot-token",
     "allowFrom": ["123456789012345678"]
   }
   ```

### Slack

1. **Create a Slack app**:
   - Go to https://api.slack.com/apps
   - Click "Create New App" > "From scratch"
   - Name your app and select your workspace

2. **Enable Socket Mode**:
   - Go to "Socket Mode" in the left sidebar
   - Toggle "Enable Socket Mode" to ON
   - Click "Generate Token" under "App-Level Tokens"
   - Name it (e.g., "Socket Mode Token") and generate
   - Copy the token (starts with `xapp-`)

3. **Get Bot Token**:
   - Go to "OAuth & Permissions" in the left sidebar
   - Scroll to "Scopes" > "Bot Token Scopes"
   - Add the following scopes:

   | Scope | Purpose |
   |-------|---------|
   | `chat:write` | Send and edit messages |
   | `channels:history` | Read messages in public channels |
   | `groups:history` | Read messages in private channels |
   | `im:history` | Read direct messages |
   | `mpim:history` | Read group direct messages |
   | `users:read` | Look up usernames from user IDs |
   | `files:read` | Download image attachments from messages |
   | `reactions:write` | Add emoji reactions to acknowledge messages |

   Optional (not required but recommended):
   | `users:write` | Set bot presence to "active" on startup |

   - Scroll up and click "Install to Workspace"
   - Copy the "Bot User OAuth Token" (starts with `xoxb-`)

4. **Enable App Home messaging**:
   - Go to "App Home" in the left sidebar
   - Under "Show Tabs", enable the **Messages Tab**
   - Check **"Allow users to send Slash commands and messages from the messages tab"**

   Without this, users will see "Sending messages to this app has been turned off."

5. **Subscribe to events**:
   - Go to "Event Subscriptions"
   - Enable "Enable Events"
   - Subscribe to bot events: `app_mention`, `message.channels`, `message.groups`, `message.im`

6. **Get user IDs**:
   - Click on a user's profile in Slack, click the three dots menu, select "Copy member ID"

7. **Configure**:
   ```json
   "slack": {
     "enabled": true,
     "botToken": "xoxb-1234567890-1234567890123-abcdefghijklmnopqrstuvwx",
     "appToken": "xapp-1-A1234567890-1234567890123-abcdefghijklmnopqrstuvwxyz1234567890",
     "allowFrom": ["U01234567"]
   }
   ```

**Note**: The `appToken` must be a Socket Mode token (starts with `xapp-`), not a bot token. Socket Mode allows your app to receive events without exposing a public HTTP endpoint.

### WhatsApp

1. **First-time setup**:
   - Run `./nanobot gateway` with WhatsApp enabled in config
   - Scan the QR code displayed in the terminal with your phone (WhatsApp > Settings > Linked Devices > Link a Device)
   - Session is automatically stored in `~/.nanobot/whatsapp/`

2. **Configure**:
   ```json
   "whatsapp": {
     "enabled": true,
     "allowFrom": ["15037348571"]
   }
   ```

3. **Phone number format**:
   - Use phone numbers in international format (country code + number)
   - No spaces, dashes, or plus signs needed
   - Example: `"15037348571"` for US number `+1 (503) 734-8571`

### Twilio (SMS/MMS)

1. **Get credentials**:
   - Sign up at https://console.twilio.com
   - Copy your **Account SID** and **Auth Token** from the dashboard

2. **Buy a phone number**:
   - Go to **Phone Numbers > Buy a Number**
   - Ensure SMS capability is checked
   - Note the number in E.164 format (e.g. `+15551234567`)

3. **Create a Conversation Service**:
   - Go to **Messaging > Conversations > Manage > Create Service**
   - Note the Conversation Service SID

4. **Configure webhooks**:
   - Go to **Conversations > Manage > [Your Service] > Webhooks**
   - Set **Post-Webhook URL** to your nanobot server's public URL (e.g. `https://your-server.example.com/twilio/webhook`)
   - Subscribe to events: **`onMessageAdded`**
   - Method: **POST**

5. **Add participants to conversations**:
   Conversations need participants before messages flow. Via Twilio API or Console:
   ```bash
   curl -X POST "https://conversations.twilio.com/v1/Conversations/{ConversationSid}/Participants" \
     -u "YOUR_ACCOUNT_SID:YOUR_AUTH_TOKEN" \
     --data-urlencode "MessagingBinding.Address=+19876543210" \
     --data-urlencode "MessagingBinding.ProxyAddress=+15551234567"
   ```

6. **Expose your webhook**:
   The webhook server must be reachable from the internet. Options:
   - **Cloudflare Tunnel** (recommended): `cloudflared tunnel run` — free, stable, no open ports
   - **ngrok**: `ngrok http 8080` — quick for development
   - **Reverse proxy**: nginx/caddy with TLS termination

7. **Configure**:
   ```json
   "twilio": {
     "enabled": true,
     "accountSid": "ACxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
     "authToken": "your-auth-token",
     "phoneNumber": "+15551234567",
     "webhookPort": 8080,
     "webhookPath": "/twilio/webhook",
     "webhookUrl": "https://your-server.example.com/twilio/webhook",
     "allowFrom": []
   }
   ```

   - `webhookUrl` must match exactly what Twilio POSTs to (used for signature validation)
   - `allowFrom` empty means all senders are allowed; add phone numbers to restrict

## Running

### Gateway Mode

Start the gateway to run all enabled channels and the agent:

```bash
./target/release/nanobot gateway
```

### CLI Mode

Interact with the agent directly from the terminal:

```bash
# Interactive session
./target/release/nanobot agent

# Single message
./target/release/nanobot agent -m "What's the weather?"
```

### Cron Jobs

Manage scheduled jobs from the CLI:

```bash
# List jobs
./target/release/nanobot cron list

# Add a recurring job (every 3600 seconds)
./target/release/nanobot cron add -n "Hourly check" -m "Check my inbox" -e 3600 --channel telegram --to 123456789

# Add a cron-expression job targeting all channels
./target/release/nanobot cron add -n "Morning briefing" -m "Give me a morning briefing" -c "0 9 * * *" --tz "America/New_York" --all-channels

# Remove a job
./target/release/nanobot cron remove --id abc12345

# Enable/disable
./target/release/nanobot cron enable --id abc12345
./target/release/nanobot cron enable --id abc12345 --disable

# Edit a job
./target/release/nanobot cron edit --id abc12345 -m "New message" --all-channels

# Manually trigger a job
./target/release/nanobot cron run --id abc12345 --force
```

Jobs support optional auto-stop limits via the LLM tool interface:
- **`expires_at`**: ISO 8601 datetime after which the job auto-disables (e.g. stop a recurring ping after 5 minutes)
- **`max_runs`**: Maximum number of executions before auto-disabling (e.g. "ping 7 times then stop")

### Authentication

```bash
# Authenticate with Google (Gmail, Calendar)
./target/release/nanobot auth google
```

### Voice Transcription

Voice messages from channels are automatically transcribed to text. Two backends are supported:

**Local (whisper-rs)** — On-device inference using whisper.cpp. Requires `ffmpeg` and a GGML model file:

```bash
# Install ffmpeg
sudo apt install ffmpeg

# Download the model (~574 MB)
mkdir -p ~/.nanobot/models
wget -O ~/.nanobot/models/ggml-large-v3-turbo-q5_0.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin
```

**Cloud (Whisper API)** — Uses Groq, OpenAI, or any OpenAI-compatible transcription endpoint. Requires an API key.

Routing is controlled by `preferLocal` (default `true`):
- `preferLocal: true` — tries local first, falls back to cloud if local fails
- `preferLocal: false` — tries cloud first, falls back to local if no API key

Either backend alone is sufficient. Set `localModelPath` for local, `apiKey` for cloud, or both for fallback.

```json
"voice": {
  "transcription": {
    "enabled": true,
    "localModelPath": "~/.nanobot/models/ggml-large-v3-turbo-q5_0.bin",
    "preferLocal": true,
    "threads": 4,
    "apiKey": "",
    "apiBase": "https://api.groq.com/openai/v1/audio/transcriptions",
    "model": "whisper-large-v3-turbo"
  }
}
```

### Logging

```bash
# Default: info level, with noisy dependencies suppressed
./target/release/nanobot gateway

# Debug logging
RUST_LOG=debug ./target/release/nanobot gateway

# Custom filtering
RUST_LOG=info,whatsapp_rust=warn,nanobot::channels=debug ./target/release/nanobot gateway
```

## Model Configuration

### API Key Models

For models that use API keys (most models):

```json
{
  "agents": {
    "defaults": {
      "model": "claude-sonnet-4-5-20250929"
    }
  },
  "providers": {
    "anthropic": {
      "apiKey": "sk-ant-api03-..."
    }
  }
}
```

Available API key models:
- `claude-sonnet-4-5-20250929` (Anthropic) - Recommended, best balance
- `claude-haiku-4-5-20251001` (Anthropic) - Fastest
- `claude-opus-4-5-20251101` (Anthropic) - Most capable
- `gpt-4`, `gpt-3.5-turbo` (OpenAI)
- `gemini-pro` (Google)

### OpenAI-Compatible Models

Any model whose name contains a supported provider keyword is automatically routed to that provider's OpenAI-compatible API. Just set the API key in the config — no other setup needed:

```json
{
  "agents": {
    "defaults": {
      "model": "deepseek-chat"
    }
  },
  "providers": {
    "deepseek": {
      "apiKey": "sk-..."
    }
  }
}
```

Supported providers and their default endpoints:

| Provider | Keyword | Default Base URL |
|----------|---------|------------------|
| OpenRouter | `openrouter` | `https://openrouter.ai/api/v1/chat/completions` |
| DeepSeek | `deepseek` | `https://api.deepseek.com/v1/chat/completions` |
| Groq | `groq` | `https://api.groq.com/openai/v1/chat/completions` |
| Moonshot | `moonshot` | `https://api.moonshot.cn/v1/chat/completions` |
| Zhipu | `zhipu` | `https://open.bigmodel.cn/api/paas/v4/chat/completions` |
| DashScope | `dashscope` | `https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions` |
| vLLM | `vllm` | `http://localhost:8000/v1/chat/completions` |
| Ollama | `ollama` | `http://localhost:11434/v1/chat/completions` |

Local providers (Ollama and vLLM) do not require an API key. Use the `provider/model` prefix format to route to them — the prefix is stripped before sending to the API (e.g. `ollama/qwen3-coder:30b` sends `qwen3-coder:30b` to the Ollama API).

To override the default endpoint, set `apiBase` on the provider:

```json
{
  "providers": {
    "vllm": {
      "apiKey": "token-abc123",
      "apiBase": "http://my-server:8080/v1/chat/completions"
    }
  }
}
```

### Local Model Fallback

You can configure a local model (e.g. Ollama) as a fallback. The cloud model remains the primary provider — the local model is only used if the cloud provider fails or returns malformed tool calls:

```json
{
  "agents": {
    "defaults": {
      "model": "claude-sonnet-4-5-20250929",
      "localModel": "ollama/qwen3-coder:30b"
    }
  },
  "providers": {
    "anthropic": {
      "apiKey": "sk-ant-api03-..."
    }
  }
}
```

When `localModel` is set, each LLM call tries the cloud model first. If the cloud provider returns an error (e.g. network failure, rate limit) or the response contains malformed tool calls (empty name, non-object arguments), the request is automatically retried against the local model.

### OAuth Models

Some Anthropic models require OAuth authentication (models starting with `anthropic/`):

- `anthropic/claude-opus-4-5`
- `anthropic/claude-opus-4-6`

For OAuth models, you need to:
1. Install [Claude CLI](https://github.com/anthropics/claude-cli) or [OpenClaw](https://github.com/anthropics/openclaw)
2. Or configure OAuth credentials in the config:
   ```json
   {
     "providers": {
       "anthropicOAuth": {
         "enabled": true,
         "autoDetect": true,
         "credentialsPath": "~/.anthropic/credentials.json"
       }
     }
   }
   ```

## Tools

The agent has access to 23 built-in tools:

### Core Tools (always available)

| Tool | Description |
|------|-------------|
| `read_file` | Read files from disk |
| `write_file` | Write files to disk (with automatic versioned backups) |
| `edit_file` | Edit files with find/replace diffs |
| `list_dir` | List directory contents |
| `exec` | Execute shell commands (allowlist/blocklist secured) |
| `tmux` | Manage persistent tmux shell sessions (create, send, read, list, kill) |
| `web_search` | Search the web (configurable: Brave API or DuckDuckGo) |
| `web_fetch` | Fetch and extract web page content |
| `http` | Make HTTP requests (GET, POST, PUT, PATCH, DELETE) |
| `message` | Send messages to chat channels |
| `spawn` | Spawn background subagents for parallel task execution |
| `subagent_control` | List running subagents, check capacity, or cancel by ID |
| `cron` | Schedule tasks: agent or echo mode, with optional `expires_at` and `max_runs` auto-stop |
| `memory_search` | Search long-term memory and daily notes (FTS5, optional hybrid vector+keyword) |
| `reddit` | Fetch posts from Reddit subreddits (hot, new, top) |

### Configurable Tools (require setup)

| Tool | Description | Config Required |
|------|-------------|-----------------|
| `google_mail` | Gmail: search, read, send, reply, label | `tools.google.*` + OAuth |
| `google_calendar` | Google Calendar: list, create, update, delete events | `tools.google.*` + OAuth |
| `github` | GitHub API: issues, PRs, repos | `tools.github.token` |
| `weather` | Weather forecasts via OpenWeatherMap | `tools.weather.apiKey` |
| `todoist` | Todoist task management: list, create, complete, update | `tools.todoist.token` |
| `media` | Radarr/Sonarr: search, add, monitor movies & TV | `tools.media.*` |
| `obsidian` | Obsidian vault: read, write, append, search, list notes | `tools.obsidian.*` |
| `browser` | Browser automation via Chrome DevTools Protocol: open, click, type, screenshot, eval JS | `tools.browser.enabled` |

### Subagent System

The agent can spawn background subagents to handle complex tasks in parallel:

- **Concurrency limiting**: Configurable max concurrent subagents (default 5) via semaphore
- **Context injection**: Subagents receive the parent conversation's compaction summary so they understand what was discussed
- **Silent mode**: Internal spawns (from cron/daemon) can skip user-facing announcements
- **Lifecycle management**: List running subagents, check capacity, cancel by ID
- **Tool isolation**: Subagents get filesystem, shell, and web tools but cannot message users or spawn more subagents
- **Parallel tool execution**: Subagent tool calls run in parallel (same pattern as the main agent loop)

## Workspace Structure

```
~/.nanobot/
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
├── errors.rs       # NanobotError typed error enum
└── utils/          # URL security, atomic writes, task tracking, voice transcription
```

## Architecture

- **Async-first**: Built on `tokio` for high-performance async I/O
- **Cargo feature flags**: Each channel is a compile-time feature (`channel-telegram`, `channel-discord`, `channel-slack`, `channel-whatsapp`, `channel-twilio`), allowing slim builds without unused dependencies
- **Message bus**: Decoupled channel-agent communication via inbound/outbound message bus
- **Connection resilience**: All channels (Telegram, Discord, Slack, WhatsApp, Twilio) use exponential backoff retry loops for automatic reconnection after disconnects
- **Channel edit/delete**: `BaseChannel` trait provides `send_and_get_id`, `edit_message`, and `delete_message` with default no-ops; implemented for Telegram, Discord, and Slack
- **Session management**: SQLite-backed sessions with automatic TTL cleanup
- **Memory**: SQLite FTS5 for semantic memory indexing with background indexer, automatic fact extraction, optional hybrid vector+keyword search via local ONNX embeddings (fastembed), and automatic memory hygiene (archive old notes, purge expired archives, clean orphaned entries)
- **Compaction**: Automatic conversation summarization when context exceeds token threshold
- **Tool execution**: Panic-isolated via `tokio::task::spawn`, parallel execution via `join_all`, LRU result caching for read-only tools, pre-execution JSON schema validation
- **Tool facts injection**: Each agent turn injects a reminder listing all available tools, preventing the LLM from falsely claiming tools are unavailable
- **Reflection turns**: After tool results are returned, a reflection prompt forces deliberative reasoning about next steps
- **Editable status messages**: Tool execution progress shown as a single message that edits in-place rather than flooding the chat. Tracks status per (channel, chat_id), accumulates tool status lines with emoji prefixes, adds a "Composing response..." indicator during LLM thinking, and deletes the status message when the final response arrives. Channels without edit support (WhatsApp) fall back to separate messages.
- **Subagents**: Semaphore-limited background task execution with conversation context injection and parallel tool calls
- **Cron**: File-backed job store with multi-channel target delivery, agent mode and echo mode, timezone auto-detection, auto-expiry (`expires_at`), run limits (`max_runs`), and automatic name deduplication
- **Heartbeat/Daemon**: Periodic background check-ins driven by a strategy file (`HEARTBEAT.md`)
- **Voice transcription**: Dual-backend transcription service (local whisper.cpp via `whisper-rs` + cloud Whisper API). Audio converted to 16kHz mono f32 PCM via ffmpeg subprocess; local inference runs on a blocking thread pool. Configurable routing (`preferLocal`) with automatic fallback between backends.
- **Skills**: Extensible via workspace SKILL.md files with YAML frontmatter, dependency checking, and auto-include
- **Hallucination detection**: Regex-based action claim detection, tool-name mention counting, and false no-tools-claim detection with automatic retry prevent the LLM from fabricating actions or denying tool access; first-iteration forced tool use prevents text-only hallucinations
- **Security**: Shell command allowlist + blocklist with pipe/chain operator parsing; SSRF protection blocking private IPs, loopback, and metadata endpoints; path traversal prevention; OAuth credential file permissions (0o600); config secret redaction in Debug impls

## Development

### Prerequisites

- Rust (nightly toolchain required for WhatsApp support)
- OpenSSL development libraries (`libssl-dev` on Debian/Ubuntu)
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
