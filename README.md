# Nanobot Rust

A high-performance Rust implementation of the nanobot AI assistant framework with multi-channel support.

## Features

- **Multi-channel support**: Telegram, Discord, Slack, WhatsApp
- **LLM providers**: Anthropic (Claude), OpenAI (GPT), Google (Gemini), with OAuth support
- **Agent capabilities**: Tool calling, memory, context management, subagents
- **Async-first**: Built on Tokio for high-performance async I/O
- **Type-safe**: Leverages Rust's type system for compile-time guarantees

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
     "token": "MTQ2OTE4Mjc4NDI0MDIyNjMzNA.GsVdH8.VnCu4ns8V3hWUbQmYOarZzavg8xe807QuK917o",
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

4. **Subscribe to events** (optional, for app mentions):
   - Go to "Event Subscriptions"
   - Enable "Enable Events"
   - Subscribe to bot events: `app_mention`, `message.channels`, `message.groups`, `message.im`

5. **Get user IDs**:
   - Right-click on users in Slack and select "Copy member ID" (if available)
   - Or use the user's email/username in `allowFrom`

6. **Configure**:
   ```json
   "slack": {
     "enabled": true,
     "botToken": "xoxb-1234567890-1234567890123-abcdefghijklmnopqrstuvwx",
     "appToken": "xapp-1-A1234567890-1234567890123-abcdefghijklmnopqrstuvwxyz1234567890abcdefghijklmnopqrstuvwxyz",
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
# Debug build
./target/debug/nanobot gateway

# Release build
./target/release/nanobot gateway
```

### Command Options

```bash
# Use a specific model
./target/release/nanobot gateway --model claude-sonnet-4-5-20250929

# Enable debug logging
RUST_LOG=debug ./target/release/nanobot gateway

# Info level logging
RUST_LOG=info ./target/release/nanobot gateway
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

## Project Structure

```
src/
├── agent/          # Agent loop, context, memory, tools
├── auth/           # OAuth authentication (Google)
├── bus/            # Message bus for channel-agent communication
├── channels/       # Channel implementations (Telegram, Discord, Slack, WhatsApp)
├── cli/            # Command-line interface
├── config/         # Configuration schema and loader
├── cron/           # Cron job scheduling
├── heartbeat/      # Heartbeat/daemon service
├── providers/      # LLM provider implementations
├── session/        # Session management
└── utils/          # Utility functions
```

## Architecture

- **Async-first**: Built on `tokio` for high-performance async I/O
- **Message bus**: Decoupled channel-agent communication via message bus
- **Session management**: LRU cache for conversation history
- **Memory**: SQLite FTS5 for semantic memory indexing
- **LLM providers**: Trait-based abstraction for multiple providers
- **Tools**: Extensible tool system for agent capabilities

## Troubleshooting

### Slack Connection Issues

- Verify Socket Mode is enabled in your Slack app settings
- Ensure `appToken` starts with `xapp-` (Socket Mode token)
- Ensure `botToken` starts with `xoxb-` (Bot User OAuth Token)
- Verify bot has required scopes: `chat:write`, `channels:history`, `groups:history`, `im:history`, `mpim:history`, `users:read`

### WhatsApp Connection Issues

- Ensure `~/.nanobot/whatsapp/` directory exists and is writable
- Scan QR code when prompted on first run
- Verify phone numbers in `allowFrom` are in international format (no `+` prefix)

### Telegram Connection Issues

- Verify the bot token is correct
- Ensure `allowFrom` contains valid Telegram user IDs

### Discord Connection Issues

- Verify the bot token is correct
- Ensure "Message Content Intent" is enabled in Discord Developer Portal
- Verify bot has been invited to your server with correct permissions

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

### CI/CD

This project uses GitHub Actions for continuous integration. The CI pipeline:
- Builds on both stable and nightly Rust toolchains
- Runs `cargo fmt` and `cargo clippy` checks
- Runs test suite
- Creates release artifacts on main branch pushes

## License

MIT

## Contributing

Contributions welcome! Please ensure:
- Code compiles without errors
- All warnings are addressed (or justified)
- Tests pass
- Code follows Rust conventions
