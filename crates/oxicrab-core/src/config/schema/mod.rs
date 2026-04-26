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
mod router;
mod tools;

pub use agent::*;
pub use channels::*;
pub use providers::*;
pub use router::*;
pub use tools::*;

pub(crate) fn default_true() -> bool {
    true
}

fn default_metrics_bind() -> String {
    "127.0.0.1:9901".to_string()
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
    /// Exact IPs or CIDRs allowed to supply X-Forwarded-For.
    /// Required when trustProxy is enabled.
    #[serde(default, rename = "trustedProxies")]
    pub trusted_proxies: Vec<String>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            requests_per_second: default_rps(),
            burst: default_burst(),
            trust_proxy: false,
            trusted_proxies: vec![],
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

redact_debug!(
    GatewayConfig,
    enabled,
    host,
    port,
    redact(api_key),
    webhooks,
    a2a,
    rate_limit,
);

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
    /// External URL clients should use to reach this agent. When unset, the
    /// AgentCard returns the internal bind host:port, which may leak
    /// deployment topology when the gateway sits behind a reverse proxy.
    /// Set this to the public endpoint (e.g. `https://agents.example.com`)
    /// in production.
    #[serde(default, rename = "publicUrl")]
    pub public_url: String,
}

/// Configuration for a named webhook receiver endpoint.
///
/// Each webhook is available at `POST /api/webhook/{name}`.
#[derive(Clone, Serialize, Deserialize)]
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
    /// Structured dispatch for direct tool execution, bypassing LLM.
    #[serde(default)]
    pub dispatch: Option<WebhookDispatchConfig>,
    /// When true, the receiver requires an `X-Webhook-Timestamp` header
    /// **and** that header must be included in the HMAC signature input
    /// (`HMAC(timestamp + "." + body)`). This is the only configuration
    /// that gives hard replay protection: with the lenient body-only
    /// mode an attacker who captured a valid request can strip the
    /// timestamp on replay and the body-only signature still validates.
    /// Default `false` to support senders that don't sign the
    /// timestamp; flip to `true` for hosts where replay risk matters.
    #[serde(default, rename = "requireTimestamp")]
    pub require_timestamp: bool,
}

redact_debug!(
    WebhookConfig,
    enabled,
    redact(secret),
    template,
    targets,
    agent_turn,
    dispatch,
    require_timestamp,
);

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            secret: String::new(),
            template: default_webhook_template(),
            targets: vec![],
            agent_turn: false,
            dispatch: None,
            require_timestamp: false,
        }
    }
}

/// Structured dispatch configuration for webhook direct tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDispatchConfig {
    /// Tool name to dispatch to.
    pub tool: String,
    /// Template for tool params. Use `{{key}}` for JSON payload substitution.
    #[serde(rename = "paramsTemplate")]
    pub params_template: serde_json::Value,
    /// When true (the default), the dispatched action is routed through the
    /// operator approval workflow regardless of whether the tool/action is
    /// otherwise covered by `approval.actions`. Set to `false` only when the
    /// webhook source is fully trusted (e.g. signed by an internal service
    /// on a closed network).
    #[serde(default = "default_true", rename = "requireApproval")]
    pub require_approval: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsExporterConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_metrics_bind")]
    pub bind: String,
}

impl Default for MetricsExporterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_metrics_bind(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ObservabilityConfig {
    #[serde(default)]
    pub metrics: MetricsExporterConfig,
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
    pub router: RouterConfig,
    #[serde(default)]
    pub voice: VoiceConfig,
    #[serde(default, rename = "credentialHelper")]
    pub credential_helper: CredentialHelperConfig,
    #[serde(default)]
    pub observability: ObservabilityConfig,
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
        self.validate_router()?;
        self.validate_tools()?;
        self.validate_channels()?;
        self.validate_model_routing()?;
        self.validate_provider_temperatures()?;
        self.validate_provider_headers()?;
        self.validate_observability()?;
        self.validate_context_providers()?;
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
        if let Some(t) = d.temperature
            && (t.is_nan() || t.is_infinite() || !(0.0..=2.0).contains(&t))
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
        if d.approval.enabled && d.approval.timeout < 10 {
            return Err(OxicrabError::Config(
                "agents.defaults.approval.timeout must be >= 10 seconds when enabled".into(),
            ));
        }
        // Reflection: warn (don't fail) when both allow and block lists
        // are populated — `covers_tool` resolves allow-wins-over-block,
        // so the block list is silently dead. A startup warning is the
        // cheapest way to surface a misconfiguration that would
        // otherwise look like the operator's intent was honoured.
        let r = &d.reflection;
        if !r.allowed_tools.is_empty() && !r.blocked_tools.is_empty() {
            warn!(
                "agents.defaults.reflection: both allowedTools ({}) and blockedTools ({}) \
                 are set; allowedTools wins and blockedTools is ignored — clear one to remove this warning",
                r.allowed_tools.len(),
                r.blocked_tools.len()
            );
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
        if m.rrf_k == 0 {
            return Err(OxicrabError::Config(
                "agents.defaults.memory.rrfK must be at least 1 (0 would cause division by zero)"
                    .into(),
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

    fn validate_observability(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;
        let metrics = &self.observability.metrics;
        if metrics.enabled && metrics.bind.parse::<std::net::SocketAddr>().is_err() {
            return Err(OxicrabError::Config(format!(
                "observability.metrics.bind must be host:port, got '{}'",
                metrics.bind
            )));
        }
        if metrics.enabled
            && let Ok(addr) = metrics.bind.parse::<std::net::SocketAddr>()
            && !addr.ip().is_loopback()
        {
            warn!(
                "metrics exporter is binding to {} without authentication; scrape it only behind a trusted network boundary or reverse proxy",
                addr
            );
        }
        Ok(())
    }

    fn validate_gateway(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;
        use ipnet::IpNet;

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
        if self.gateway.enabled
            && !self.gateway.api_key.is_empty()
            && self.gateway.api_key.len() < 32
            && self.gateway.host != "127.0.0.1"
            && self.gateway.host != "localhost"
            && self.gateway.host != "::1"
        {
            return Err(OxicrabError::Config(format!(
                "gateway.apiKey must be at least 32 characters when listening on {} (got {})",
                self.gateway.host,
                self.gateway.api_key.len()
            )));
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
        if self.gateway.rate_limit.trust_proxy && self.gateway.rate_limit.trusted_proxies.is_empty()
        {
            return Err(OxicrabError::Config(
                "gateway.rateLimit.trustedProxies must contain at least one IP or CIDR when trustProxy is enabled".into(),
            ));
        }
        for proxy in &self.gateway.rate_limit.trusted_proxies {
            if proxy.parse::<IpNet>().is_err() {
                return Err(OxicrabError::Config(format!(
                    "gateway.rateLimit.trustedProxies entries must be valid IPs or CIDRs, got '{proxy}'"
                )));
            }
        }
        for (name, webhook) in &self.gateway.webhooks {
            if !webhook.enabled {
                continue;
            }
            let trimmed_secret = webhook.secret.trim();
            if trimmed_secret.is_empty() {
                return Err(OxicrabError::Config(format!(
                    "gateway.webhooks.{name}.secret is required when webhook is enabled"
                )));
            }
            if trimmed_secret.len() < 32 {
                return Err(OxicrabError::Config(format!(
                    "gateway.webhooks.{name}.secret must be at least 32 characters (got {})",
                    trimmed_secret.len()
                )));
            }
            if webhook.targets.is_empty() {
                return Err(OxicrabError::Config(format!(
                    "gateway.webhooks.{name}.targets must contain at least one target when webhook is enabled"
                )));
            }
            if let Some(dispatch) = &webhook.dispatch
                && dispatch.tool.trim().is_empty()
            {
                return Err(OxicrabError::Config(format!(
                    "gateway.webhooks.{name}.dispatch.tool must not be empty"
                )));
            }
            if webhook.agent_turn && webhook.dispatch.is_some() {
                return Err(OxicrabError::Config(format!(
                    "gateway.webhooks.{name} cannot set both agentTurn and dispatch"
                )));
            }
            for (idx, target) in webhook.targets.iter().enumerate() {
                if target.channel.trim().is_empty() {
                    return Err(OxicrabError::Config(format!(
                        "gateway.webhooks.{name}.targets[{idx}].channel must not be empty"
                    )));
                }
                if target.chat_id.trim().is_empty() {
                    return Err(OxicrabError::Config(format!(
                        "gateway.webhooks.{name}.targets[{idx}].chatId must not be empty"
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_router(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;
        let router = &self.router;

        if router.prefix.is_empty() {
            return Err(OxicrabError::Config(
                "router.prefix must not be empty".into(),
            ));
        }
        if router.semantic_top_k == 0 {
            return Err(OxicrabError::Config(
                "router.semanticTopK must be > 0".into(),
            ));
        }
        if router.semantic_prefilter_k == 0 {
            return Err(OxicrabError::Config(
                "router.semanticPrefilterK must be > 0".into(),
            ));
        }
        if router.semantic_top_k > router.semantic_prefilter_k {
            return Err(OxicrabError::Config(
                "router.semanticTopK must be <= router.semanticPrefilterK".into(),
            ));
        }
        if !router.semantic_threshold.is_finite()
            || !(0.0..=1.0).contains(&router.semantic_threshold)
        {
            return Err(OxicrabError::Config(
                "router.semanticThreshold must be a finite number between 0.0 and 1.0".into(),
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
        // Warn about AllowedCommands polarity: empty = unrestricted (opposite of
        // DenyByDefaultList where empty = deny-all). Operators expecting deny-all
        // semantics from `allowedCommands = []` would get unrestricted execution.
        if !self.tools.exec.allowed_commands.is_restricted() {
            warn!(
                "tools.exec.allowedCommands is empty/unset — all shell commands are allowed. \
                 set a command list to restrict execution, or set to the defaults to restore \
                 the built-in allowlist"
            );
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
            if self.tools.obsidian.api_url.is_none() {
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
        if self.tools.web_search.max_results == 0 {
            return Err(OxicrabError::Config(
                "tools.web_search.maxResults must be > 0".into(),
            ));
        }
        if self.tools.web_search.max_results > 100 {
            warn!("tools.web_search.maxResults is very large (> 100), this may be slow");
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

        if routing.default.is_empty() {
            return Err(OxicrabError::Config(
                "modelRouting.default must not be empty".to_string(),
            ));
        }

        for (i, fb) in routing.fallbacks.iter().enumerate() {
            if fb.is_empty() {
                return Err(OxicrabError::Config(format!(
                    "modelRouting.fallbacks[{i}] must not be empty"
                )));
            }
        }

        for (task_name, task_routing) in &routing.tasks {
            match task_routing {
                TaskRouting::Model(model_str) => {
                    if model_str.is_empty() {
                        return Err(OxicrabError::Config(format!(
                            "modelRouting.tasks.{task_name} model must not be empty"
                        )));
                    }
                }
                TaskRouting::Chat(chat_config) => {
                    if task_name != "chat" {
                        return Err(OxicrabError::Config(format!(
                            "modelRouting.tasks.{task_name} uses chat routing object but \
                             only the 'chat' key supports complexity escalation"
                        )));
                    }

                    // Thresholds must be finite, in [0,1], and ordered
                    for (name, val) in [
                        ("standard", chat_config.thresholds.standard),
                        ("heavy", chat_config.thresholds.heavy),
                    ] {
                        if !val.is_finite() || !(0.0..=1.0).contains(&val) {
                            return Err(OxicrabError::Config(format!(
                                "modelRouting.tasks.chat.thresholds.{name} must be finite \
                                 and in [0.0, 1.0], got {val}"
                            )));
                        }
                    }
                    if chat_config.thresholds.standard >= chat_config.thresholds.heavy {
                        return Err(OxicrabError::Config(
                            "modelRouting.tasks.chat.thresholds: standard must be less than heavy"
                                .to_string(),
                        ));
                    }

                    // Models must be non-empty
                    if chat_config.models.standard.is_empty() {
                        return Err(OxicrabError::Config(
                            "modelRouting.tasks.chat.models.standard must not be empty".to_string(),
                        ));
                    }
                    if chat_config.models.heavy.is_empty() {
                        return Err(OxicrabError::Config(
                            "modelRouting.tasks.chat.models.heavy must not be empty".to_string(),
                        ));
                    }

                    // Weights must be finite
                    let w = &chat_config.weights;
                    let weight_values = [
                        ("messageLength", w.message_length),
                        ("reasoningKeywords", w.reasoning_keywords),
                        ("technicalVocabulary", w.technical_vocabulary),
                        ("questionComplexity", w.question_complexity),
                        ("codePresence", w.code_presence),
                        ("instructionComplexity", w.instruction_complexity),
                        ("conversationalSimplicity", w.conversational_simplicity),
                    ];
                    for (name, val) in &weight_values {
                        if !val.is_finite() {
                            return Err(OxicrabError::Config(format!(
                                "modelRouting.tasks.chat.weights.{name} must be finite, got {val}"
                            )));
                        }
                    }

                    // Warn when the absolute sum of weights is outside the
                    // reasonable range — the scorer's sigmoid may saturate or
                    // become insensitive with extreme total magnitudes.
                    let abs_sum: f64 = weight_values.iter().map(|(_, v)| v.abs()).sum();
                    if !(0.3..=2.0).contains(&abs_sum) {
                        warn!(
                            "modelRouting.tasks.chat.weights: absolute sum is {:.2} \
                             (expected 0.3..2.0) — scoring may be overly sensitive or insensitive",
                            abs_sum
                        );
                    }
                }
            }
        }

        Ok(())
    }

    fn validate_provider_temperatures(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;

        for (name, cfg) in self.providers.all_providers() {
            if let Some(t) = cfg.temperature
                && (t.is_nan() || t.is_infinite() || !(0.0..=2.0).contains(&t))
            {
                return Err(OxicrabError::Config(format!(
                    "providers.{name}.temperature must be a finite number between 0.0 and 2.0"
                )));
            }
        }

        Ok(())
    }

    fn validate_provider_headers(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;

        let reserved_headers: &[&str] = &[
            "authorization",
            "content-type",
            "x-api-key",
            "anthropic-version",
            "x-session-affinity",
        ];
        for (provider_name, config) in self.providers.all_providers() {
            for key in config.headers.keys() {
                if reserved_headers.contains(&key.to_lowercase().as_str()) {
                    return Err(OxicrabError::Config(format!(
                        "providers.{provider_name}.headers: '{key}' is a reserved header name \
                         and cannot be overridden"
                    )));
                }
            }
        }

        Ok(())
    }

    fn validate_context_providers(&self) -> Result<(), crate::errors::OxicrabError> {
        use crate::errors::OxicrabError;

        fn has_control_chars(s: &str) -> bool {
            s.chars().any(|c| c.is_control())
        }

        for (i, cp) in self.agents.defaults.context_providers.iter().enumerate() {
            for (j, bin) in cp.requires_bins.iter().enumerate() {
                if bin.is_empty() {
                    return Err(OxicrabError::Config(format!(
                        "agents.defaults.contextProviders[{i}].requiresBins[{j}] must not be empty"
                    )));
                }
                if has_control_chars(bin) {
                    return Err(OxicrabError::Config(format!(
                        "agents.defaults.contextProviders[{i}].requiresBins[{j}] contains control characters"
                    )));
                }
                if bin.contains('/') || bin.contains('\\') {
                    return Err(OxicrabError::Config(format!(
                        "agents.defaults.contextProviders[{i}].requiresBins[{j}] must be a binary name, not a path"
                    )));
                }
            }
            for (j, env_var) in cp.requires_env.iter().enumerate() {
                if env_var.is_empty() {
                    return Err(OxicrabError::Config(format!(
                        "agents.defaults.contextProviders[{i}].requiresEnv[{j}] must not be empty"
                    )));
                }
                if has_control_chars(env_var) {
                    return Err(OxicrabError::Config(format!(
                        "agents.defaults.contextProviders[{i}].requiresEnv[{j}] contains control characters"
                    )));
                }
                if env_var.contains('=') {
                    return Err(OxicrabError::Config(format!(
                        "agents.defaults.contextProviders[{i}].requiresEnv[{j}] must not contain '='"
                    )));
                }
            }
            if cp.command.is_empty() {
                return Err(OxicrabError::Config(format!(
                    "agents.defaults.contextProviders[{i}].command must not be empty"
                )));
            }
            if has_control_chars(&cp.command) {
                return Err(OxicrabError::Config(format!(
                    "agents.defaults.contextProviders[{i}].command contains control characters"
                )));
            }
            if cp.command.contains('/') || cp.command.contains('\\') {
                return Err(OxicrabError::Config(format!(
                    "agents.defaults.contextProviders[{i}].command must be a binary name, not a path"
                )));
            }
            for (j, arg) in cp.args.iter().enumerate() {
                if has_control_chars(arg) {
                    return Err(OxicrabError::Config(format!(
                        "agents.defaults.contextProviders[{i}].args[{j}] contains control characters"
                    )));
                }
            }
        }

        Ok(())
    }

    pub fn get_api_key(&self, model: Option<&str>) -> Option<&str> {
        let model = model.unwrap_or(&self.agents.defaults.model_routing.default);
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
            ("minimax_api_key", &self.providers.minimax.api_key),
            ("telegram_token", &self.channels.telegram.token),
            ("discord_token", &self.channels.discord.token),
            ("slack_bot_token", &self.channels.slack.bot_token),
            ("slack_app_token", &self.channels.slack.app_token),
            ("twilio_auth_token", &self.channels.twilio.auth_token),
            ("github_token", &self.tools.github.token),
            ("weather_api_key", &self.tools.weather.api_key),
            ("todoist_token", &self.tools.todoist.token),
            ("obsidian_api_key", &self.tools.obsidian.api_key),
            ("web_search_api_key", &self.tools.web_search.api_key),
            ("vllm_api_key", &self.providers.vllm.base.api_key),
            ("ollama_api_key", &self.providers.ollama.base.api_key),
            ("google_client_secret", &self.tools.google.client_secret),
            ("radarr_api_key", &self.tools.media.radarr.api_key),
            ("sonarr_api_key", &self.tools.media.sonarr.api_key),
            ("transcription_api_key", &self.voice.transcription.api_key),
            ("twilio_account_sid", &self.channels.twilio.account_sid),
            ("gateway_api_key", &self.gateway.api_key),
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
        for (_, cfg) in self.providers.all_providers() {
            for value in cfg.headers.values() {
                if !value.is_empty() {
                    secrets.push(("provider_header", value.as_str()));
                }
            }
        }

        // Include MCP server environment variable values (may contain API keys)
        for server in self.tools.mcp.servers.values() {
            for value in server.env.values() {
                if !value.is_empty() {
                    secrets.push(("mcp_server_env", value.as_str()));
                }
            }
        }

        secrets
    }
}
