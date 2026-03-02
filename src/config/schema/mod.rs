use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::warn;

/// Generates a `Debug` impl that redacts secret fields.
///
/// Field specifiers:
/// - `field_name`            — printed normally via `&self.field_name`
/// - `redact(field_name)`    — `String` field: shows `[empty]` or `[REDACTED]`
/// - `redact_option(field_name)` — `Option<String>` field: shows `None` or `Some("[REDACTED]")`
macro_rules! redact_debug {
    // Internal: emit a single .field() call
    (@field $builder:ident, $self:ident, redact($field:ident)) => {
        $builder.field(
            stringify!($field),
            &if $self.$field.is_empty() {
                "[empty]"
            } else {
                "[REDACTED]"
            },
        );
    };
    (@field $builder:ident, $self:ident, redact_option($field:ident)) => {
        $builder.field(
            stringify!($field),
            &$self.$field.as_ref().map(|_| "[REDACTED]"),
        );
    };
    (@field $builder:ident, $self:ident, $field:ident) => {
        $builder.field(stringify!($field), &$self.$field);
    };

    // Internal: recursive TT muncher
    (@fields $builder:ident, $self:ident,) => {};
    (@fields $builder:ident, $self:ident, redact($field:ident), $($rest:tt)*) => {
        redact_debug!(@field $builder, $self, redact($field));
        redact_debug!(@fields $builder, $self, $($rest)*);
    };
    (@fields $builder:ident, $self:ident, redact_option($field:ident), $($rest:tt)*) => {
        redact_debug!(@field $builder, $self, redact_option($field));
        redact_debug!(@fields $builder, $self, $($rest)*);
    };
    (@fields $builder:ident, $self:ident, $field:ident, $($rest:tt)*) => {
        redact_debug!(@field $builder, $self, $field);
        redact_debug!(@fields $builder, $self, $($rest)*);
    };

    // Entry point
    ($struct_name:ident, $($fields:tt)*) => {
        impl std::fmt::Debug for $struct_name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let mut builder = f.debug_struct(stringify!($struct_name));
                redact_debug!(@fields builder, self, $($fields)*);
                builder.finish()
            }
        }
    };
}

// Submodules — declared after the macro so they can use `redact_debug!`
mod agent;
mod channels;
mod providers;
mod tools;

pub use agent::*;
pub use channels::*;
pub use providers::*;
pub use tools::*;

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Gateway
// ---------------------------------------------------------------------------

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    18790
}

fn default_rps() -> u32 {
    10
}

fn default_burst() -> u32 {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_rps", rename = "requestsPerSecond")]
    pub requests_per_second: u32,
    #[serde(default = "default_burst")]
    pub burst: u32,
    /// Trust X-Forwarded-For header for client IP extraction.
    /// Only enable when running behind a reverse proxy (nginx, Cloudflare, etc.).
    #[serde(default, rename = "trustProxy")]
    pub trust_proxy: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            requests_per_second: default_rps(),
            burst: default_burst(),
            trust_proxy: false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// API key for authenticating `/api/chat` and A2A endpoints.
    /// When set, requests must include `Authorization: Bearer <key>` or
    /// `X-API-Key: <key>`. Webhooks use their own HMAC auth and are exempt.
    /// Health and `.well-known/agent.json` are always public.
    #[serde(default, rename = "apiKey")]
    pub api_key: String,
    #[serde(default)]
    pub webhooks: HashMap<String, WebhookConfig>,
    #[serde(default)]
    pub a2a: A2aConfig,
    #[serde(default, rename = "rateLimit")]
    pub rate_limit: RateLimitConfig,
}

impl std::fmt::Debug for GatewayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayConfig")
            .field("enabled", &self.enabled)
            .field("host", &self.host)
            .field("port", &self.port)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "[empty]"
                } else {
                    "[REDACTED]"
                },
            )
            .field("webhooks", &self.webhooks)
            .field("a2a", &self.a2a)
            .field("rate_limit", &self.rate_limit)
            .finish()
    }
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            host: default_host(),
            port: default_port(),
            api_key: String::new(),
            webhooks: HashMap::new(),
            a2a: A2aConfig::default(),
            rate_limit: RateLimitConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct A2aConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "agentName")]
    pub agent_name: String,
    #[serde(default, rename = "agentDescription")]
    pub agent_description: String,
}

/// Configuration for a named webhook receiver endpoint.
///
/// Each webhook is available at `POST /api/webhook/{name}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// Whether this webhook is active. Disabled webhooks return 404.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// HMAC-SHA256 secret for signature validation.
    pub secret: String,
    /// Template for the message sent to the agent. Use `{{key}}` for JSON payload fields,
    /// `{{body}}` for the raw body.
    #[serde(default = "default_webhook_template")]
    pub template: String,
    /// Target channels to deliver the agent response to.
    #[serde(default)]
    pub targets: Vec<WebhookTarget>,
    /// If true, the webhook payload is routed through the agent loop.
    /// If false (default), the templated message is delivered directly to targets.
    #[serde(default, rename = "agentTurn")]
    pub agent_turn: bool,
}

fn default_webhook_template() -> String {
    "{{body}}".to_string()
}

/// Target channel for webhook delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookTarget {
    pub channel: String,
    #[serde(rename = "chatId")]
    pub chat_id: String,
}

// ---------------------------------------------------------------------------
// Credential helper
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CredentialHelperConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Format adapter: "json" (default), "1password", "bitwarden", "line"
    #[serde(default)]
    pub format: String,
}

// ---------------------------------------------------------------------------
// Top-level Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub agents: AgentsConfig,
    #[serde(default)]
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub voice: VoiceConfig,
    #[serde(default, rename = "credentialHelper")]
    pub credential_helper: CredentialHelperConfig,
}

impl Config {
    pub fn workspace_path(&self) -> PathBuf {
        crate::utils::get_workspace_path(&self.agents.defaults.workspace)
    }

    /// Validate configuration values
    pub fn validate(&self) -> Result<(), crate::errors::OxicrabError> {
        self.validate_agent_defaults()?;
        self.validate_compaction()?;
        self.validate_memory()?;
        self.validate_cognitive()?;
        self.validate_gateway()?;
        self.validate_tools()?;
        self.validate_channels()?;
        self.validate_model_routing()?;
        Ok(())
    }

    fn validate_agent_defaults(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;
        let d = &self.agents.defaults;

        if d.max_tokens == 0 {
            return Err(OxicrabError::Config(
                "agents.defaults.maxTokens must be > 0".into(),
            ));
        }
        if d.max_tokens > 1_000_000 {
            return Err(OxicrabError::Config(
                "agents.defaults.maxTokens is unreasonably large (> 1,000,000)".into(),
            ));
        }
        if d.temperature.is_nan()
            || d.temperature.is_infinite()
            || d.temperature < 0.0
            || d.temperature > 2.0
        {
            return Err(OxicrabError::Config(
                "agents.defaults.temperature must be a finite number between 0.0 and 2.0".into(),
            ));
        }
        if d.max_tool_iterations == 0 {
            return Err(OxicrabError::Config(
                "agents.defaults.maxToolIterations must be > 0".into(),
            ));
        }
        if d.max_tool_iterations > 1000 {
            return Err(OxicrabError::Config(
                "agents.defaults.maxToolIterations is unreasonably large (> 1000)".into(),
            ));
        }
        for (model_name, cost) in &d.cost_guard.model_costs {
            if cost.input_per_million.is_nan()
                || cost.input_per_million.is_infinite()
                || cost.input_per_million < 0.0
            {
                return Err(OxicrabError::Config(format!(
                    "agents.defaults.costGuard.modelCosts.{model_name}.inputPerMillion must be a non-negative finite number"
                )));
            }
            if cost.output_per_million.is_nan()
                || cost.output_per_million.is_infinite()
                || cost.output_per_million < 0.0
            {
                return Err(OxicrabError::Config(format!(
                    "agents.defaults.costGuard.modelCosts.{model_name}.outputPerMillion must be a non-negative finite number"
                )));
            }
        }
        if d.daemon.enabled {
            if d.daemon.interval == 0 {
                return Err(OxicrabError::Config(
                    "agents.defaults.daemon.interval must be > 0 when enabled".into(),
                ));
            }
            if d.daemon.interval < 60 {
                warn!("Daemon interval is very short (< 60s), this may cause high resource usage");
            }
        }
        Ok(())
    }

    fn validate_compaction(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;
        let c = &self.agents.defaults.compaction;

        if c.enabled {
            if c.threshold_tokens == 0 {
                return Err(OxicrabError::Config(
                    "agents.defaults.compaction.thresholdTokens must be > 0 when enabled".into(),
                ));
            }
            if c.keep_recent == 0 {
                return Err(OxicrabError::Config(
                    "agents.defaults.compaction.keepRecent must be > 0 when enabled".into(),
                ));
            }
        }
        if c.checkpoint.enabled && c.checkpoint.interval_iterations == 0 {
            return Err(OxicrabError::Config(
                "agents.defaults.compaction.checkpoint.intervalIterations must be > 0 when enabled"
                    .into(),
            ));
        }
        Ok(())
    }

    fn validate_memory(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;
        let m = &self.agents.defaults.memory;

        if m.hybrid_weight.is_nan()
            || m.hybrid_weight.is_infinite()
            || !(0.0..=1.0).contains(&m.hybrid_weight)
        {
            return Err(OxicrabError::Config(
                "agents.defaults.memory.hybridWeight must be a finite number between 0.0 and 1.0"
                    .into(),
            ));
        }
        if m.archive_after_days > 0
            && m.purge_after_days > 0
            && m.purge_after_days <= m.archive_after_days
        {
            return Err(OxicrabError::Config(
                "agents.defaults.memory.purgeAfterDays must be > archiveAfterDays".into(),
            ));
        }
        Ok(())
    }

    fn validate_cognitive(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;
        let c = &self.agents.defaults.cognitive;

        if c.enabled
            && (c.gentle_threshold >= c.firm_threshold || c.firm_threshold >= c.urgent_threshold)
        {
            return Err(OxicrabError::Config(
                "agents.defaults.cognitive thresholds must be ordered: gentle < firm < urgent"
                    .into(),
            ));
        }
        Ok(())
    }

    fn validate_gateway(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;

        if self.gateway.port == 0 {
            return Err(OxicrabError::Config("gateway.port must be > 0".into()));
        }
        if self.gateway.port < 1024 {
            warn!(
                "gateway.port {} is a privileged port (< 1024), may require elevated permissions",
                self.gateway.port
            );
        }
        // Warn when listening on non-loopback without auth
        if self.gateway.enabled
            && self.gateway.api_key.is_empty()
            && self.gateway.host != "127.0.0.1"
            && self.gateway.host != "localhost"
            && self.gateway.host != "::1"
        {
            warn!(
                "gateway listening on {} without authentication — any network client can \
                 send commands to the agent. set gateway.apiKey to secure the endpoint",
                self.gateway.host
            );
        }
        if self.gateway.rate_limit.enabled && self.gateway.rate_limit.requests_per_second == 0 {
            return Err(OxicrabError::Config(
                "gateway.rateLimit.requestsPerSecond must be > 0 when enabled".into(),
            ));
        }
        if self.gateway.rate_limit.enabled && self.gateway.rate_limit.burst == 0 {
            return Err(OxicrabError::Config(
                "gateway.rateLimit.burst must be > 0 when enabled".into(),
            ));
        }
        Ok(())
    }

    fn validate_tools(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;

        if self.tools.exec.timeout == 0 {
            return Err(OxicrabError::Config(
                "tools.exec.timeout must be > 0".into(),
            ));
        }
        if self.tools.exec.timeout > 3600 {
            warn!("tools.exec.timeout is very long (> 3600s), this may cause timeouts");
        }
        if self.tools.browser.timeout == 0 {
            return Err(OxicrabError::Config(
                "tools.browser.timeout must be > 0".into(),
            ));
        }
        if self.tools.exec.sandbox.additional_read_paths.len() > 100 {
            return Err(OxicrabError::Config(
                "tools.exec.sandbox.additionalReadPaths has too many entries (max 100)".into(),
            ));
        }
        if self.tools.exec.sandbox.additional_write_paths.len() > 100 {
            return Err(OxicrabError::Config(
                "tools.exec.sandbox.additionalWritePaths has too many entries (max 100)".into(),
            ));
        }
        if self.tools.obsidian.enabled {
            if self.tools.obsidian.api_url.is_empty() {
                return Err(OxicrabError::Config(
                    "tools.obsidian.apiUrl is required when obsidian is enabled".into(),
                ));
            }
            if self.tools.obsidian.api_key.is_empty() {
                return Err(OxicrabError::Config(
                    "tools.obsidian.apiKey is required when obsidian is enabled".into(),
                ));
            }
            if self.tools.obsidian.vault_name.is_empty() {
                return Err(OxicrabError::Config(
                    "tools.obsidian.vaultName is required when obsidian is enabled".into(),
                ));
            }
            if self.tools.obsidian.timeout == 0 {
                return Err(OxicrabError::Config(
                    "tools.obsidian.timeout must be > 0 when obsidian is enabled".into(),
                ));
            }
        }
        if self.tools.web.search.max_results == 0 {
            return Err(OxicrabError::Config(
                "tools.web.search.maxResults must be > 0".into(),
            ));
        }
        if self.tools.web.search.max_results > 100 {
            warn!("tools.web.search.maxResults is very large (> 100), this may be slow");
        }
        Ok(())
    }

    fn validate_channels(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;

        if self.channels.telegram.enabled && self.channels.telegram.token.is_empty() {
            return Err(OxicrabError::Config(
                "channels.telegram.token is required when telegram is enabled".into(),
            ));
        }
        if self.channels.discord.enabled && self.channels.discord.token.is_empty() {
            return Err(OxicrabError::Config(
                "channels.discord.token is required when discord is enabled".into(),
            ));
        }
        if self.channels.slack.enabled {
            if self.channels.slack.bot_token.is_empty() {
                return Err(OxicrabError::Config(
                    "channels.slack.botToken is required when slack is enabled".into(),
                ));
            }
            if self.channels.slack.app_token.is_empty() {
                return Err(OxicrabError::Config(
                    "channels.slack.appToken is required when slack is enabled".into(),
                ));
            }
        }

        let tw = &self.channels.twilio;
        if tw.enabled {
            if tw.account_sid.is_empty() {
                return Err(OxicrabError::Config(
                    "channels.twilio.accountSid is required when twilio is enabled".into(),
                ));
            }
            if tw.auth_token.is_empty() {
                return Err(OxicrabError::Config(
                    "channels.twilio.authToken is required when twilio is enabled".into(),
                ));
            }
            if tw.webhook_url.is_empty() {
                return Err(OxicrabError::Config(
                    "channels.twilio.webhookUrl is required when twilio is enabled".into(),
                ));
            }
            if tw.webhook_port == 0 {
                return Err(OxicrabError::Config(
                    "channels.twilio.webhookPort must be > 0 when twilio is enabled".into(),
                ));
            }
            if tw.webhook_path.is_empty() || !tw.webhook_path.starts_with('/') {
                return Err(OxicrabError::Config(
                    "channels.twilio.webhookPath must start with '/' when twilio is enabled".into(),
                ));
            }
        }
        Ok(())
    }

    fn validate_model_routing(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;
        let routing = &self.agents.defaults.model_routing;

        for (rule_name, tier_name) in &routing.rules {
            if !routing.tiers.contains_key(tier_name) {
                return Err(OxicrabError::Config(format!(
                    "modelRouting.rules.{} references unknown tier '{}' — \
                     add it to modelRouting.tiers",
                    rule_name, tier_name
                )));
            }
        }
        for (name, model_str) in &routing.tiers {
            if model_str.is_empty() {
                return Err(OxicrabError::Config(format!(
                    "modelRouting.tiers.{} must not be empty",
                    name
                )));
            }
        }
        for (i, fb) in routing.fallbacks.iter().enumerate() {
            if fb.is_empty() {
                return Err(OxicrabError::Config(format!(
                    "modelRouting.fallbacks[{}] must not be empty",
                    i
                )));
            }
        }
        Ok(())
    }

    pub fn get_api_key(&self, model: Option<&str>) -> Option<&str> {
        let model = model.unwrap_or(&self.agents.defaults.model);
        self.providers.get_api_key(model)
    }

    /// Collect all non-empty secret values for leak detection.
    ///
    /// Returns `(name, value)` pairs covering provider API keys, channel tokens,
    /// and tool credentials. The leak detector uses these to scan outbound
    /// messages for encoded variants (raw, base64, hex).
    pub fn collect_secrets(&self) -> Vec<(&str, &str)> {
        let mut secrets = Vec::new();
        let candidates: &[(&str, &str)] = &[
            ("anthropic_api_key", &self.providers.anthropic.api_key),
            ("openai_api_key", &self.providers.openai.api_key),
            ("openrouter_api_key", &self.providers.openrouter.api_key),
            ("deepseek_api_key", &self.providers.deepseek.api_key),
            ("groq_api_key", &self.providers.groq.api_key),
            ("gemini_api_key", &self.providers.gemini.api_key),
            ("moonshot_api_key", &self.providers.moonshot.api_key),
            ("zhipu_api_key", &self.providers.zhipu.api_key),
            ("dashscope_api_key", &self.providers.dashscope.api_key),
            (
                "anthropic_oauth_access",
                &self.providers.anthropic_oauth.access_token,
            ),
            (
                "anthropic_oauth_refresh",
                &self.providers.anthropic_oauth.refresh_token,
            ),
            ("telegram_token", &self.channels.telegram.token),
            ("discord_token", &self.channels.discord.token),
            ("slack_bot_token", &self.channels.slack.bot_token),
            ("slack_app_token", &self.channels.slack.app_token),
            ("twilio_auth_token", &self.channels.twilio.auth_token),
            ("github_token", &self.tools.github.token),
            ("weather_api_key", &self.tools.weather.api_key),
            ("todoist_token", &self.tools.todoist.token),
            ("obsidian_api_key", &self.tools.obsidian.api_key),
            ("web_search_api_key", &self.tools.web.search.api_key),
            ("vllm_api_key", &self.providers.vllm.base.api_key),
            ("ollama_api_key", &self.providers.ollama.base.api_key),
            ("google_client_secret", &self.tools.google.client_secret),
            ("radarr_api_key", &self.tools.media.radarr.api_key),
            ("sonarr_api_key", &self.tools.media.sonarr.api_key),
            ("transcription_api_key", &self.voice.transcription.api_key),
            ("twilio_account_sid", &self.channels.twilio.account_sid),
        ];
        for &(name, value) in candidates {
            if !value.is_empty() {
                secrets.push((name, value));
            }
        }

        // Include webhook HMAC secrets
        for wh in self.gateway.webhooks.values() {
            if !wh.secret.is_empty() {
                secrets.push(("webhook_secret", wh.secret.as_str()));
            }
        }

        // Include custom header values from all providers (may contain auth tokens)
        let provider_configs = [
            &self.providers.anthropic,
            &self.providers.openai,
            &self.providers.openrouter,
            &self.providers.deepseek,
            &self.providers.groq,
            &self.providers.zhipu,
            &self.providers.dashscope,
            &self.providers.vllm.base,
            &self.providers.gemini,
            &self.providers.moonshot,
            &self.providers.ollama.base,
        ];
        for cfg in provider_configs {
            for value in cfg.headers.values() {
                if !value.is_empty() {
                    secrets.push(("provider_header", value.as_str()));
                }
            }
        }

        secrets
    }

    /// Create providers for all configured model routing tiers.
    pub fn create_routed_providers(
        &self,
    ) -> anyhow::Result<Option<crate::config::routing::ResolvedRouting>> {
        use crate::providers::strategy::ProviderFactory;

        let routing = &self.agents.defaults.model_routing;
        if routing.tiers.is_empty() {
            return Ok(None);
        }
        let factory = ProviderFactory::new(self);
        let mut tiers = std::collections::HashMap::new();
        for (name, model_str) in &routing.tiers {
            let mut provider = factory.create_provider(model_str)?;
            if self.should_use_prompt_guided_tools(model_str) {
                provider =
                    crate::providers::prompt_guided::PromptGuidedToolsProvider::wrap(provider);
            }
            tiers.insert(name.clone(), (provider, model_str.clone()));
        }
        Ok(Some(crate::config::routing::ResolvedRouting::new(
            tiers,
            routing.rules.clone(),
        )))
    }

    /// Create an LLM provider instance based on configuration.
    ///
    /// Uses a 2-tier resolution strategy: prefix notation → model-name
    /// inference.
    pub fn create_provider(
        &self,
        model: Option<&str>,
    ) -> anyhow::Result<std::sync::Arc<dyn crate::providers::base::LLMProvider>> {
        use crate::providers::strategy::ProviderFactory;

        let model = model.unwrap_or(&self.agents.defaults.model);
        let factory = ProviderFactory::new(self);

        // Build fallback chain from modelRouting.fallbacks (takes precedence over localModel)
        let routing = &self.agents.defaults.model_routing;
        if !routing.fallbacks.is_empty() {
            if self.agents.defaults.local_model.is_some() {
                warn!(
                    "both modelRouting.fallbacks and localModel are set — \
                     fallbacks takes precedence; localModel will be ignored"
                );
            }
            let mut primary = factory.create_provider(model)?;
            if self.should_use_prompt_guided_tools(model) {
                primary = crate::providers::prompt_guided::PromptGuidedToolsProvider::wrap(primary);
            }
            let primary_model = model.to_string();
            let mut chain = vec![(primary, primary_model)];
            for fb_model in &routing.fallbacks {
                let mut fb_provider = factory.create_provider(fb_model)?;
                if self.should_use_prompt_guided_tools(fb_model) {
                    fb_provider = crate::providers::prompt_guided::PromptGuidedToolsProvider::wrap(
                        fb_provider,
                    );
                }
                chain.push((fb_provider, fb_model.clone()));
            }
            return Ok(std::sync::Arc::new(
                crate::providers::fallback::FallbackProvider::new(chain),
            ));
        }

        if let Some(ref local_model) = self.agents.defaults.local_model
            && !local_model.is_empty()
        {
            let cloud = factory.create_provider(model)?;
            let mut local = factory.create_provider(local_model)?;
            if self.should_use_prompt_guided_tools(local_model) {
                local = crate::providers::prompt_guided::PromptGuidedToolsProvider::wrap(local);
            }
            return Ok(std::sync::Arc::new(
                crate::providers::fallback::FallbackProvider::pair(
                    cloud,
                    local,
                    model.to_string(),
                    local_model.clone(),
                ),
            ));
        }

        let provider = factory.create_provider(model)?;
        if self.should_use_prompt_guided_tools(model) {
            return Ok(crate::providers::prompt_guided::PromptGuidedToolsProvider::wrap(provider));
        }

        Ok(provider)
    }

    /// Check if a model should use prompt-guided tool calling based on its
    /// resolved provider config.
    fn should_use_prompt_guided_tools(&self, model: &str) -> bool {
        use crate::providers::strategy::{infer_provider_from_model, parse_model_ref};

        // Check prefix notation FIRST — it's a per-model override that takes
        // priority over the global provider setting.
        let model_ref = parse_model_ref(model);
        if let Some(prefix_provider) = model_ref.provider {
            let normalized = normalize_provider(prefix_provider);
            return match normalized.as_ref() {
                "ollama" => self.providers.ollama.prompt_guided_tools,
                "vllm" => self.providers.vllm.prompt_guided_tools,
                _ => false,
            };
        }

        // Check model-name inference
        if let Some(inferred) = infer_provider_from_model(model_ref.model) {
            return match inferred {
                "ollama" => self.providers.ollama.prompt_guided_tools,
                "vllm" => self.providers.vllm.prompt_guided_tools,
                _ => false,
            };
        }

        false
    }
}

#[cfg(test)]
mod tests;
