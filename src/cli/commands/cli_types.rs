use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "oxicrab")]
#[command(about = "Personal AI Assistant")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub(super) command: Commands,
}

#[derive(Subcommand)]
pub(super) enum Commands {
    /// Initialize oxicrab configuration and workspace
    Onboard,
    /// Run the gateway (channels + agent)
    Gateway {
        #[arg(long)]
        model: Option<String>,
        /// Echo mode: test channel connectivity without an LLM
        #[arg(long)]
        echo: bool,
    },
    /// Interact with the agent directly
    Agent {
        #[arg(short, long)]
        message: Option<String>,
        #[arg(short, long, default_value = "cli:default")]
        session: String,
    },
    /// Manage cron jobs
    Cron {
        #[command(subcommand)]
        cmd: CronCommands,
    },
    /// Manage authentication for external services
    Auth {
        #[command(subcommand)]
        cmd: AuthCommands,
    },
    /// Manage channels
    Channels {
        #[command(subcommand)]
        cmd: ChannelCommands,
    },
    /// Show oxicrab status
    Status,
    /// Run system diagnostics
    Doctor,
    /// Manage sender pairing (authorize new users to message the bot)
    Pairing {
        #[command(subcommand)]
        cmd: PairingCommands,
    },
    /// Manage credentials (keyring, env vars, credential helpers)
    Credentials {
        #[command(subcommand)]
        cmd: CredentialCommands,
    },
    /// Show memory and cost statistics
    Stats {
        #[command(subcommand)]
        cmd: StatsCommands,
    },
    /// Generate shell completion scripts
    Completion {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
pub(super) enum CronCommands {
    /// List scheduled jobs
    List {
        #[arg(long, short = 'a')]
        all: bool,
    },
    /// Add a new job
    Add {
        #[arg(long, short = 'n')]
        name: String,
        #[arg(long, short = 'm')]
        message: String,
        #[arg(long, short = 'e')]
        every: Option<u64>,
        #[arg(long, short = 'c')]
        cron: Option<String>,
        #[arg(long)]
        tz: Option<String>,
        #[arg(long)]
        at: Option<String>,
        #[arg(long)]
        agent_echo: bool,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        all_channels: bool,
    },
    /// Remove a job
    Remove {
        #[arg(long)]
        id: String,
    },
    /// Enable or disable a job
    Enable {
        #[arg(long)]
        id: String,
        #[arg(long)]
        disable: bool,
    },
    /// Edit an existing job
    Edit {
        #[arg(long)]
        id: String,
        #[arg(long, short = 'n')]
        name: Option<String>,
        #[arg(long, short = 'm')]
        message: Option<String>,
        #[arg(long, short = 'e')]
        every: Option<u64>,
        #[arg(long, short = 'c')]
        cron: Option<String>,
        #[arg(long)]
        tz: Option<String>,
        #[arg(long)]
        at: Option<String>,
        #[arg(long)]
        agent_echo: Option<bool>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        all_channels: bool,
    },
    /// Manually run a job
    Run {
        #[arg(long)]
        id: String,
        #[arg(long, short = 'f')]
        force: bool,
    },
}

#[derive(Subcommand)]
pub(super) enum AuthCommands {
    /// Authenticate with Google (Gmail, Calendar)
    Google {
        #[arg(long, short = 'p', default_value = "8099")]
        port: u16,
        #[arg(long)]
        headless: bool,
    },
}

#[derive(Subcommand)]
pub(super) enum PairingCommands {
    /// List pending pairing requests and paired sender counts
    List,
    /// Approve a pending request by its 8-character code (e.g. ABC12345)
    Approve {
        /// The pairing code shown by `oxicrab pairing list`
        code: String,
    },
    /// Revoke a previously approved sender's access
    Revoke {
        /// Channel name: telegram, discord, slack, whatsapp, or twilio
        channel: String,
        /// The sender ID to remove (same format as allowFrom entries)
        sender_id: String,
    },
}

#[derive(Subcommand)]
pub(super) enum ChannelCommands {
    /// Show channel status
    Status,
    /// Link `WhatsApp` device via QR code
    Login,
}

#[derive(Subcommand)]
pub(super) enum StatsCommands {
    /// Show LLM cost summary
    Costs {
        /// Number of days to look back (default: 7)
        #[arg(long, short = 'd', default_value = "7")]
        days: u32,
    },
    /// Show memory search statistics
    Search,
    /// Show cost for today
    Today,
    /// Show intent classification and hallucination detection metrics
    Intent {
        /// Number of days to look back (default: 7)
        #[arg(long, short = 'd', default_value = "7")]
        days: u32,
    },
    /// Show complexity routing statistics and cost correlation
    Complexity {
        /// Number of days to look back (default: 7)
        #[arg(long, short = 'd', default_value = "7")]
        days: u32,
    },
}

#[derive(Subcommand)]
pub(super) enum CredentialCommands {
    /// Store a credential in the OS keyring
    Set {
        /// Credential slot name (e.g. "anthropic-api-key")
        name: String,
        /// Value to store (reads from stdin if omitted)
        value: Option<String>,
    },
    /// Check if a credential exists (shows \[set\] or \[empty\])
    Get {
        /// Credential slot name
        name: String,
    },
    /// Remove a credential from the OS keyring
    Delete {
        /// Credential slot name
        name: String,
    },
    /// List all credential slots and their sources
    List,
    /// Import non-empty credentials from config.json into the OS keyring
    Import,
}
