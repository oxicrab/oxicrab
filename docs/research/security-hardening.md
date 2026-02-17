# Security Hardening Research — Phase 1

## Overview

This document captures the security gaps identified in oxicrab and the hardening measures implemented in Phase 1.

## Credential Security

### Problem: Plaintext Secrets with No Env Var Override
- All API keys and tokens stored in `~/.oxicrab/config.json` in plaintext
- No way to inject secrets via environment variables (standard for containers/CI)
- Config file often has overly permissive file permissions

### Solution
- Added `OXICRAB_*` env var overrides for all API keys and channel tokens
- Config file permission check on startup (warns if world-readable on unix)
- Atomic config writes via tempfile+rename to prevent corruption
- `oxicrab doctor` now includes security audit section

## Channel Security

### Problem: Open-by-Default AllowLists
- Empty `allowFrom` arrays permitted all senders — security risk for public bots
- No mechanism for unknown senders to request access

### Solution
- Empty allowlist now denies all senders (default-deny)
- Explicit `"*"` wildcard required for open access
- DM pairing system allows controlled onboarding of new senders
- CLI commands: `oxicrab pairing list/approve/revoke`

### Problem: Timing-Attack-Vulnerable Signature Comparison
- Twilio webhook signature compared with `==` (vulnerable to timing attacks)

### Solution
- Added `subtle` crate for constant-time byte comparison

### Problem: Missing Discord DM Intent
- Discord gateway intents lacked `DIRECT_MESSAGES`, preventing DM support

### Solution
- Added `GatewayIntents::DIRECT_MESSAGES` to the intent set

## Output Security

### Problem: No Leak Detection
- LLM responses could inadvertently contain API keys or tokens
- No scanning of outbound messages for secret patterns

### Solution
- New `LeakDetector` module with regex patterns for common API key formats
- Integrated into `MessageBus::publish_outbound()` — scans and redacts before sending
- Patterns cover: Anthropic, OpenAI, Slack, GitHub, Groq, Telegram, Discord tokens

## Env Var Mapping

| Env Var | Config Field |
|---------|-------------|
| `OXICRAB_ANTHROPIC_API_KEY` | `providers.anthropic.api_key` |
| `OXICRAB_OPENAI_API_KEY` | `providers.openai.api_key` |
| `OXICRAB_OPENROUTER_API_KEY` | `providers.openrouter.api_key` |
| `OXICRAB_GEMINI_API_KEY` | `providers.gemini.api_key` |
| `OXICRAB_DEEPSEEK_API_KEY` | `providers.deepseek.api_key` |
| `OXICRAB_GROQ_API_KEY` | `providers.groq.api_key` |
| `OXICRAB_TELEGRAM_TOKEN` | `channels.telegram.token` |
| `OXICRAB_DISCORD_TOKEN` | `channels.discord.token` |
| `OXICRAB_SLACK_BOT_TOKEN` | `channels.slack.bot_token` |
| `OXICRAB_SLACK_APP_TOKEN` | `channels.slack.app_token` |
| `OXICRAB_TWILIO_ACCOUNT_SID` | `channels.twilio.account_sid` |
| `OXICRAB_TWILIO_AUTH_TOKEN` | `channels.twilio.auth_token` |
| `OXICRAB_GITHUB_TOKEN` | `tools.github.token` |
