mod channels_cmd;
mod cli_types;
mod credentials_cmd;
mod cron_cmd;
mod gateway_setup;
mod onboard;
mod stats_cmd;
mod subcommands;

#[cfg(test)]
mod tests;

use cli_types::{Cli, Commands};

use onboard::create_workspace_templates;

use anyhow::Result;
use clap::{CommandFactory, Parser};

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Onboard => {
            onboard::onboard()?;
        }
        Commands::Gateway { model, echo } => {
            if echo {
                gateway_setup::gateway_echo().await?;
            } else {
                gateway_setup::gateway(model).await?;
            }
        }
        Commands::Agent { message, session } => {
            subcommands::agent(message, session).await?;
        }
        Commands::Cron { cmd } => {
            cron_cmd::cron_command(cmd).await?;
        }
        Commands::Auth { cmd } => {
            subcommands::auth_command(cmd).await?;
        }
        Commands::Channels { cmd } => {
            channels_cmd::channels_command(cmd).await?;
        }
        Commands::Status => {
            subcommands::status_command()?;
        }
        Commands::Doctor => {
            crate::cli::doctor::doctor_command().await?;
        }
        Commands::Pairing { cmd } => {
            subcommands::pairing_command(cmd)?;
        }
        Commands::Credentials { cmd } => {
            credentials_cmd::credentials_command(cmd)?;
        }
        Commands::Stats { ref cmd } => {
            stats_cmd::stats_command(cmd)?;
        }
        Commands::Completion { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "oxicrab",
                &mut std::io::stdout(),
            );
        }
    }

    Ok(())
}
