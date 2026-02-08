# Nanobot Rust

A high-performance Rust implementation of the nanobot AI assistant framework with multi-channel support.

## Features

- **Multi-channel support**: Telegram, Discord, Slack, WhatsApp
- **LLM providers**: Anthropic (Claude), OpenAI (GPT), Google (Gemini), with OAuth support
- **Agent capabilities**: Tool calling, memory, context management, subagents
- **Cron scheduling**: Recurring jobs, one-shot timers, cron expressions with multi-channel delivery
- **Integrations**: Google (Gmail, Calendar), GitHub, Todoist, Weather, Web search
- **Streaming**: SSE-based streaming for Anthropic responses
- **Session management**: Persistent sessions with automatic compaction and fact extraction
- **Async-first**: Built on Tokio for high-performance async I/O

## Building

```bash
# Debug build
cargo build

# Release build
cargo build --release
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
      "maxToolIterations": 20
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
        "apiKey": "your-brave-search-api-key"
      }
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
   - Add scopes: `chat:write`, `channels:history`, `groups:history`, `im:history`, `mpim:history`, `users:read`
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

### Authentication

```bash
# Authenticate with Google (Gmail, Calendar)
./target/release/nanobot auth google
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

The agent has access to the following built-in tools:

| Tool | Description | Config Required |
|------|-------------|-----------------|
| `read_file` | Read files from disk | - |
| `write_file` | Write files to disk | - |
| `edit_file` | Edit files with find/replace | - |
| `list_dir` | List directory contents | - |
| `exec` | Execute shell commands | - |
| `web_search` | Search the web (Brave) | `tools.web.search.apiKey` |
| `web_fetch` | Fetch and extract web page content | - |
| `http` | Make HTTP requests | - |
| `message` | Send messages to chat channels | - |
| `cron` | Schedule reminders and recurring tasks | - |
| `spawn` | Spawn background subagents | - |
| `tmux` | Manage tmux sessions | - |
| `google_mail` | Read/send Gmail | `tools.google.*` + OAuth |
| `google_calendar` | Manage Google Calendar events | `tools.google.*` + OAuth |
| `github` | GitHub API (issues, PRs, repos) | `tools.github.token` |
| `weather` | Get weather forecasts | `tools.weather.apiKey` |
| `todoist` | Manage Todoist tasks | `tools.todoist.token` |

## Workspace Structure

```
~/.nanobot/
├── config.json              # Main configuration
├── workspace/
│   ├── IDENTITY.md          # Bot identity and adaptations
│   ├── SOUL.md              # Personality and behavioural directives
│   ├── USER.md              # User preferences
│   ├── AGENTS.md            # Agent behaviour guide
│   ├── TOOLS.md             # Tool usage guide
│   ├── memory/
│   │   ├── MEMORY.md        # Long-term memory
│   │   └── YYYY-MM-DD.md    # Daily notes (auto-extracted facts)
│   ├── sessions/            # Conversation sessions
│   └── skills/              # Custom skills (SKILL.md per skill)
├── cron/
│   └── jobs.json            # Scheduled jobs
└── whatsapp/
    └── whatsapp.db          # WhatsApp session storage
```

## Project Structure

```
src/
├── agent/          # Agent loop, context, memory, tools, subagents
├── auth/           # OAuth authentication (Google)
├── bus/            # Message bus for channel-agent communication
├── channels/       # Channel implementations (Telegram, Discord, Slack, WhatsApp)
├── cli/            # Command-line interface
├── config/         # Configuration schema and loader
├── cron/           # Cron job scheduling service
├── heartbeat/      # Heartbeat/daemon service
├── providers/      # LLM provider implementations (Anthropic, OpenAI, Gemini)
├── session/        # Session management with LRU cache
└── utils/          # Utility functions (atomic writes, task tracking)
```

## Architecture

- **Async-first**: Built on `tokio` for high-performance async I/O
- **Message bus**: Decoupled channel-agent communication via inbound/outbound message bus
- **Session management**: File-backed sessions with automatic TTL cleanup
- **Memory**: SQLite FTS5 for semantic memory indexing with background indexer
- **Compaction**: Automatic conversation summarisation when context exceeds threshold
- **Streaming**: SSE-based streaming for Anthropic provider responses
- **Tool execution**: Panic-isolated via `tokio::task::spawn` with LRU result caching
- **Cron**: File-backed job store with multi-channel target delivery
- **Action integrity**: Hallucination detection prevents the LLM from claiming actions it didn't perform

## Development

### Prerequisites

- Rust (nightly toolchain required for WhatsApp support)
- OpenSSL development libraries (`libssl-dev` on Debian/Ubuntu)
- SQLite (bundled via `rusqlite`)

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
