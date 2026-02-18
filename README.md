<p align="center">
  <img src="docs/oxicrab.png" alt="oxicrab" width="200">
</p>

<h1 align="center">Oxicrab</h1>

<p align="center">A high-performance Rust multi-channel AI assistant framework.</p>

**[Documentation](https://oxicrab.github.io/oxicrab/)** | [Channel Setup](https://oxicrab.github.io/oxicrab/channels.html) | [Tool Reference](https://oxicrab.github.io/oxicrab/tools.html) | [Deployment](https://oxicrab.github.io/oxicrab/deploy.html)

## Features

- **Multi-channel support**: Telegram, Discord (slash commands, embeds, button components), Slack, WhatsApp, Twilio (SMS/MMS) — each behind a Cargo feature flag for slim builds
- **LLM providers**: Anthropic (Claude), OpenAI (GPT), Google (Gemini), plus OpenAI-compatible providers (OpenRouter, DeepSeek, Groq, Ollama, Moonshot, Zhipu, DashScope, vLLM), with OAuth support and local model fallback
- **22+ built-in tools**: Filesystem, shell, web, HTTP, browser automation, Google Workspace, GitHub, scheduling, memory, media management, and more — plus MCP (Model Context Protocol) for external tool servers
- **Subagents**: Background task execution with concurrency limiting, context injection, and lifecycle management
- **Cron scheduling**: Recurring jobs, one-shot timers, cron expressions, echo mode (LLM-free delivery), multi-channel targeting, auto-expiry (`expires_at`) and run limits (`max_runs`)
- **Memory system**: SQLite FTS5-backed long-term memory with background indexing, automatic fact extraction, optional hybrid vector+keyword search (local ONNX embeddings via fastembed), and automatic memory hygiene (archive/purge old notes)
- **Session management**: Persistent sessions with automatic compaction and context summarization
- **Hallucination detection**: Action claim detection, false no-tools-claim retry, tool facts injection, and reflection turns
- **Editable status messages**: Tool progress shown as a single message that edits in-place (Telegram, Discord, Slack), with composing indicator and automatic cleanup
- **Connection resilience**: All channels auto-reconnect with exponential backoff
- **Voice transcription**: Local whisper.cpp inference (via whisper-rs) with cloud API fallback, automatic audio conversion via ffmpeg
- **CostGuard**: Daily budget cap (cents) and hourly rate limiting with embedded pricing data for 50+ models, automatic midnight UTC reset
- **Circuit breaker**: Three-state circuit breaker (Closed/Open/HalfOpen) wraps the LLM provider, tripping only on transient errors (429, 5xx, timeout), with configurable threshold and recovery
- **Doctor command**: Run `oxicrab doctor` to check config, workspace, provider connectivity, channels, voice, external tools, and MCP servers
- **Credential management**: OS keyring (macOS Keychain, GNOME Keyring, Windows Credential Manager), external credential helpers (1Password, Bitwarden, custom scripts), and 28 `OXICRAB_*` env var overrides. Resolution: env → helper → keyring → config.json
- **Security**: Default-deny sender allowlists with DM pairing system, outbound leak detection (scans for API key patterns and redacts before sending), shell command allowlist/blocklist, SSRF protection, path traversal prevention, secret redaction in logs, constant-time webhook signature validation
- **Async-first**: Built on Tokio for high-performance async I/O

## Installation

### Pre-built binaries

Download the latest release from [GitHub Releases](https://github.com/oxicrab/oxicrab/releases/latest):

| Platform | Archive |
|----------|---------|
| Linux x86_64 | `oxicrab-*-linux-x86_64.tar.gz` |
| Linux ARM64 (aarch64) | `oxicrab-*-linux-arm64.tar.gz` |
| macOS ARM64 (Apple Silicon) | `oxicrab-*-macos-arm64.tar.gz` |

```bash
# Example: download and install linux-x86_64
tar xzf oxicrab-*-linux-x86_64.tar.gz
sudo cp oxicrab-*/oxicrab /usr/local/bin/
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

Oxicrab supports multiple credential backends with the following resolution order (highest priority wins):

```
Environment variables (OXICRAB_*) → Credential helper → OS keyring → config.json
```

Each layer only fills fields that are still empty, so higher-priority sources always win.

#### OS Keyring (desktop)

Store credentials in your OS keychain (macOS Keychain, GNOME Keyring, Windows Credential Manager) via the `keyring-store` feature (enabled by default):

```bash
# Store a credential
oxicrab credentials set anthropic-api-key

# Check what's configured
oxicrab credentials list

# Import all secrets from config.json into keyring
oxicrab credentials import

# Remove a credential
oxicrab credentials delete anthropic-api-key
```

#### Credential Helper (1Password, Bitwarden, etc.)

Configure an external credential helper to fetch secrets from a password manager. The helper CLI must already be authenticated before oxicrab starts — oxicrab calls it non-interactively.

**1Password** — install the [1Password CLI](https://developer.1password.com/docs/cli/), then authenticate:
- **Desktop**: Run `op signin` once; 1Password CLI caches the session and can use biometric unlock
- **CI/containers**: Set `OP_SERVICE_ACCOUNT_TOKEN` env var with a [service account token](https://developer.1password.com/docs/service-accounts/)

Store each credential in a 1Password vault named `oxicrab` (item name = slot name, e.g. `anthropic-api-key`), then configure:

```json
{
  "credentialHelper": {
    "command": "op",
    "args": ["--account", "my.1password.com"],
    "format": "1password"
  }
}
```

**Bitwarden** — install the [Bitwarden CLI](https://bitwarden.com/help/cli/), then `bw login && bw unlock` (or set `BW_SESSION` env var). Store items at `oxicrab/{slot-name}`:

```json
{
  "credentialHelper": {
    "command": "bw",
    "format": "bitwarden"
  }
}
```

**Custom script** — any executable that prints the secret to stdout:

```json
{
  "credentialHelper": {
    "command": "/path/to/my-secret-fetcher",
    "args": ["--vault", "production"],
    "format": "line"
  }
}
```

Supported formats:

| Format | Invocation |
|--------|-----------|
| `1password` | `op read "op://oxicrab/{key}" {args}` |
| `bitwarden` | `bw get password "oxicrab/{key}" {args}` |
| `line` | `{command} {args} {key}` (raw value on stdout) |
| `json` (default) | `{command} {args}` with `{"action":"get","key":"{key}"}` on stdin |

#### Environment Variable Overrides

All API keys and channel tokens can be set via environment variables, which take precedence over all other backends. This is recommended for containerized deployments and CI.

| Variable | Config Field |
|----------|-------------|
| `OXICRAB_ANTHROPIC_API_KEY` | `providers.anthropic.apiKey` |
| `OXICRAB_OPENAI_API_KEY` | `providers.openai.apiKey` |
| `OXICRAB_OPENROUTER_API_KEY` | `providers.openrouter.apiKey` |
| `OXICRAB_GEMINI_API_KEY` | `providers.gemini.apiKey` |
| `OXICRAB_DEEPSEEK_API_KEY` | `providers.deepseek.apiKey` |
| `OXICRAB_GROQ_API_KEY` | `providers.groq.apiKey` |
| `OXICRAB_MOONSHOT_API_KEY` | `providers.moonshot.apiKey` |
| `OXICRAB_ZHIPU_API_KEY` | `providers.zhipu.apiKey` |
| `OXICRAB_DASHSCOPE_API_KEY` | `providers.dashscope.apiKey` |
| `OXICRAB_VLLM_API_KEY` | `providers.vllm.apiKey` |
| `OXICRAB_OLLAMA_API_KEY` | `providers.ollama.apiKey` |
| `OXICRAB_ANTHROPIC_OAUTH_ACCESS` | `providers.anthropicOAuth.accessToken` |
| `OXICRAB_ANTHROPIC_OAUTH_REFRESH` | `providers.anthropicOAuth.refreshToken` |
| `OXICRAB_TELEGRAM_TOKEN` | `channels.telegram.token` |
| `OXICRAB_DISCORD_TOKEN` | `channels.discord.token` |
| `OXICRAB_SLACK_BOT_TOKEN` | `channels.slack.botToken` |
| `OXICRAB_SLACK_APP_TOKEN` | `channels.slack.appToken` |
| `OXICRAB_TWILIO_ACCOUNT_SID` | `channels.twilio.accountSid` |
| `OXICRAB_TWILIO_AUTH_TOKEN` | `channels.twilio.authToken` |
| `OXICRAB_GITHUB_TOKEN` | `tools.github.token` |
| `OXICRAB_WEATHER_API_KEY` | `tools.weather.apiKey` |
| `OXICRAB_TODOIST_TOKEN` | `tools.todoist.token` |
| `OXICRAB_WEB_SEARCH_API_KEY` | `tools.web.search.apiKey` |
| `OXICRAB_GOOGLE_CLIENT_SECRET` | `tools.google.clientSecret` |
| `OXICRAB_OBSIDIAN_API_KEY` | `tools.obsidian.apiKey` |
| `OXICRAB_MEDIA_RADARR_API_KEY` | `tools.media.radarr.apiKey` |
| `OXICRAB_MEDIA_SONARR_API_KEY` | `tools.media.sonarr.apiKey` |
| `OXICRAB_TRANSCRIPTION_API_KEY` | `voice.transcription.apiKey` |

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
     "allowFrom": ["123456789012345678"],
     "commands": [
       {
         "name": "ask",
         "description": "Ask the AI assistant",
         "options": [{ "name": "question", "description": "Your question", "required": true }]
       }
     ]
   }
   ```

   The `commands` array defines Discord slash commands registered on startup. The default `/ask` command is registered automatically if omitted. Each command supports string options that are concatenated and sent to the agent. Button component interactions are also handled — clicking a button sends `[button:{custom_id}]` to the agent.

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
   | `files:write` | Upload outbound media (screenshots, images) to channels |
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
   - Run `./oxicrab gateway` with WhatsApp enabled in config
   - Scan the QR code displayed in the terminal with your phone (WhatsApp > Settings > Linked Devices > Link a Device)
   - Session is automatically stored in `~/.oxicrab/whatsapp/`

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
   - Set **Post-Webhook URL** to your oxicrab server's public URL (e.g. `https://your-server.example.com/twilio/webhook`)
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

> **Breaking change**: Empty `allowFrom` now defaults to deny-all. Use `["*"]` to allow all senders, or use `oxicrab pairing approve` to onboard specific users.

## Running

### Gateway Mode

Start the gateway to run all enabled channels and the agent:

```bash
./target/release/oxicrab gateway
```

### CLI Mode

Interact with the agent directly from the terminal:

```bash
# Interactive session
./target/release/oxicrab agent

# Single message
./target/release/oxicrab agent -m "What's the weather?"
```

### Cron Jobs

Manage scheduled jobs from the CLI:

```bash
# List jobs
./target/release/oxicrab cron list

# Add a recurring job (every 3600 seconds)
./target/release/oxicrab cron add -n "Hourly check" -m "Check my inbox" -e 3600 --channel telegram --to 123456789

# Add a cron-expression job targeting all channels
./target/release/oxicrab cron add -n "Morning briefing" -m "Give me a morning briefing" -c "0 9 * * *" --tz "America/New_York" --all-channels

# Remove a job
./target/release/oxicrab cron remove --id abc12345

# Enable/disable
./target/release/oxicrab cron enable --id abc12345
./target/release/oxicrab cron enable --id abc12345 --disable

# Edit a job
./target/release/oxicrab cron edit --id abc12345 -m "New message" --all-channels

# Manually trigger a job
./target/release/oxicrab cron run --id abc12345 --force
```

Jobs support optional auto-stop limits via the LLM tool interface:
- **`expires_at`**: ISO 8601 datetime after which the job auto-disables (e.g. stop a recurring ping after 5 minutes)
- **`max_runs`**: Maximum number of executions before auto-disabling (e.g. "ping 7 times then stop")

### Pairing

Manage sender access for channels:

```bash
# Show pending pairing requests
./target/release/oxicrab pairing list

# Approve a pairing request
./target/release/oxicrab pairing approve <code>

# Revoke access for a specific sender
./target/release/oxicrab pairing revoke <channel> <sender_id>
```

### Authentication

```bash
# Authenticate with Google (Gmail, Calendar)
./target/release/oxicrab auth google
```

### System Diagnostics

Run `oxicrab doctor` to check your entire setup:

```bash
./target/release/oxicrab doctor
```

Checks config file, workspace, provider API keys and connectivity (warmup latency), each channel's status and credentials, voice transcription backends, external tools (ffmpeg, git), and MCP servers. Includes a **Security** section checking config file permissions, directory permissions, empty allowlists, pairing store status, OS keyring availability, and credential helper status. Reports PASS/FAIL/SKIP for every check with a summary at the end.

### Voice Transcription

Voice messages from channels are automatically transcribed to text. Two backends are supported:

**Local (whisper-rs)** — On-device inference using whisper.cpp. Requires `ffmpeg` and a GGML model file:

```bash
# Install ffmpeg
sudo apt install ffmpeg

# Download the model (~574 MB)
mkdir -p ~/.oxicrab/models
wget -O ~/.oxicrab/models/ggml-large-v3-turbo-q5_0.bin \
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
    "localModelPath": "~/.oxicrab/models/ggml-large-v3-turbo-q5_0.bin",
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
./target/release/oxicrab gateway

# Debug logging
RUST_LOG=debug ./target/release/oxicrab gateway

# Custom filtering
RUST_LOG=info,whatsapp_rust=warn,oxicrab::channels=debug ./target/release/oxicrab gateway
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

The agent has access to 22 built-in tools, plus any tools provided by MCP servers:

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
| `web_fetch` | Fetch and extract web page content (binary/image URLs auto-saved to disk) |
| `http` | Make HTTP requests (GET, POST, PUT, PATCH, DELETE); binary responses auto-saved to disk |
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
| `github` | GitHub API: issues, PRs, file content, PR reviews, CI/CD workflows | `tools.github.token` |
| `weather` | Weather forecasts via OpenWeatherMap | `tools.weather.apiKey` |
| `todoist` | Todoist task management: list, create, complete, update | `tools.todoist.token` |
| `media` | Radarr/Sonarr: search, add, monitor movies & TV | `tools.media.*` |
| `obsidian` | Obsidian vault: read, write, append, search, list notes | `tools.obsidian.*` |
| `browser` | Browser automation via Chrome DevTools Protocol: open, click, type, screenshot (saved to disk), eval JS | `tools.browser.enabled` |

### MCP (Model Context Protocol)

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
