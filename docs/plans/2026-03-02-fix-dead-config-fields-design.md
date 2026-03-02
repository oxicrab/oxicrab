# Fix Dead Config Fields

**Date:** 2026-03-02

## Problem

Four categories of config fields are parsed from JSON but silently ignored at runtime:

1. `providers.{anthropic,openai,gemini}.apiBase` — silently ignored; URLs are hardcoded in constructors
2. `agents.defaults.daemon.executionProvider` — warns and ignores; redundant with prefix notation
3. `providers.{anthropic,openai,gemini}.headers` — scanned for secrets but never sent in HTTP requests
4. `providers.*.promptGuidedTools` — only works on ollama/vllm but appears on all providers

## Design

### Fix 1: Wire up `apiBase` for first-party providers

All three providers already store `base_url: String` and use `self.base_url` in requests. The constructors just hardcode the initial value.

**Changes:**
- `AnthropicProvider`: make `with_base_url` public (currently `#[cfg(test)]`), add headers param
- `GeminiProvider`: make `with_base_url` public, add headers param
- `OpenAIProvider`: already has `with_config_and_headers` — no change needed
- `ProviderFactory::create_for_provider()`: read `api_base` and `headers` from `ProviderConfig` for all three first-party branches, use new constructors when overrides are present

### Fix 2: Remove `executionProvider` from DaemonConfig

The `provider/model` prefix notation in `executionModel` (e.g. `"openrouter/gemini-3-flash"`) already handles the use case. The field is purely dead weight.

**Changes:**
- Remove `execution_provider` field from `DaemonConfig` struct and `Default` impl
- Remove the warning log in `setup_heartbeat()`
- Remove from `config.example.json`
- Remove from docs (`_pages/config.html`, `_pages/index.html`)
- Update `test_defaults()` if referenced
- Run `python3 docs/build.py` to regenerate

### Fix 3: Wire up custom headers for Anthropic and Gemini

OpenAI provider already injects `self.custom_headers` in its request builder. Extend the same pattern to Anthropic and Gemini.

**Changes:**
- `AnthropicProvider`: add `custom_headers: HashMap<String, String>` field, inject in request builders
- `GeminiProvider`: add `custom_headers: HashMap<String, String>` field, inject in request builders
- Constructors accept headers; `new()` defaults to empty map for backward compat

### Fix 4: Move `promptGuidedTools` to local-provider-only config

Replace the shared `ProviderConfig` field with a dedicated `LocalProviderConfig` struct.

**Changes:**
- Create `LocalProviderConfig` that wraps/extends `ProviderConfig` with `prompt_guided_tools: bool`
- Change `ProvidersConfig.ollama` and `ProvidersConfig.vllm` from `ProviderConfig` to `LocalProviderConfig`
- Update `should_use_prompt_guided_tools()` to access the new type
- Update `collect_secrets()` and `get_provider_config()` which iterate over provider configs
- Remove `prompt_guided_tools` from `ProviderConfig`

## Files Affected

- `src/config/schema/providers.rs` — ProviderConfig, LocalProviderConfig, ProvidersConfig
- `src/config/schema/agent.rs` — DaemonConfig
- `src/config/schema/mod.rs` — should_use_prompt_guided_tools, collect_secrets, create_provider
- `src/providers/strategy/mod.rs` — ProviderFactory::create_for_provider, get_provider_config
- `src/providers/anthropic/mod.rs` — constructor, request builders
- `src/providers/gemini/mod.rs` — constructor, request builders
- `src/cli/commands/mod.rs` — remove executionProvider warning
- `config.example.json` — remove executionProvider
- `docs/_pages/config.html` — remove executionProvider row
- `docs/_pages/index.html` — remove executionProvider from example
- `src/config/schema/tests.rs` — update credential_overlays if needed
- `tests/common/mod.rs` — update test helpers if needed

## Testing

- Existing unit tests continue to pass (constructors are backward-compatible via defaults)
- `test_config_example_is_up_to_date` must pass after config.example.json update
- Provider strategy tests may need updates for new constructor paths
