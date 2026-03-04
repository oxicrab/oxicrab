use super::cli_types::CredentialCommands;
use crate::config::load_config;
use anyhow::Result;

pub(super) fn credentials_command(cmd: CredentialCommands) -> Result<()> {
    use crate::config::credentials::{
        CREDENTIAL_ENV_VARS, CREDENTIAL_NAMES, detect_source, get_credential_value,
    };

    match cmd {
        CredentialCommands::Set { name, value } => {
            if !CREDENTIAL_NAMES.contains(&name.as_str()) {
                anyhow::bail!(
                    "unknown credential: {name}\nRun `oxicrab credentials list` to see valid names"
                );
            }

            #[cfg(not(feature = "keyring-store"))]
            {
                let _ = value;
                anyhow::bail!("keyring support not compiled (enable 'keyring-store' feature)");
            }

            #[cfg(feature = "keyring-store")]
            {
                let secret = if let Some(v) = value {
                    v
                } else {
                    use std::io::BufRead;
                    eprint!("Enter value for {name}: ");
                    let stdin = std::io::stdin();
                    let mut line = String::new();
                    stdin.lock().read_line(&mut line)?;
                    line.trim().to_string()
                };

                if secret.is_empty() {
                    anyhow::bail!("value cannot be empty");
                }

                crate::config::credentials::keyring_set(&name, &secret)?;
                println!("Stored {name} in keyring");
            }
        }
        CredentialCommands::Get { name } => {
            if !CREDENTIAL_NAMES.contains(&name.as_str()) {
                anyhow::bail!(
                    "unknown credential: {name}\nRun `oxicrab credentials list` to see valid names"
                );
            }

            #[cfg(not(feature = "keyring-store"))]
            {
                println!("{name}: keyring support not compiled");
            }

            #[cfg(feature = "keyring-store")]
            {
                let status = if crate::config::credentials::keyring_has(&name) {
                    "[set]"
                } else {
                    "[empty]"
                };
                println!("{name}: {status}");
            }
        }
        CredentialCommands::Delete { name } => {
            if !CREDENTIAL_NAMES.contains(&name.as_str()) {
                anyhow::bail!(
                    "unknown credential: {name}\nRun `oxicrab credentials list` to see valid names"
                );
            }

            #[cfg(not(feature = "keyring-store"))]
            anyhow::bail!("keyring support not compiled (enable 'keyring-store' feature)");

            #[cfg(feature = "keyring-store")]
            {
                crate::config::credentials::keyring_delete(&name)?;
                println!("Deleted {name} from keyring");
            }
        }
        CredentialCommands::List => {
            let config = load_config(None)?;

            println!("{:<30} Source", "Credential");
            println!("{}", "\u{2500}".repeat(50));

            for &name in CREDENTIAL_NAMES {
                let source = detect_source(name, &config);
                println!("{name:<30} {source}");
            }

            println!(
                "\n{} credential slot(s), {} populated",
                CREDENTIAL_NAMES.len(),
                CREDENTIAL_NAMES
                    .iter()
                    .filter(|&&n| {
                        get_credential_value(&config, n).is_some_and(|v: &str| !v.is_empty())
                    })
                    .count()
            );

            // Show env var hint
            let env_count = CREDENTIAL_ENV_VARS
                .iter()
                .filter(|(_, env)| std::env::var(env).ok().is_some_and(|v| !v.is_empty()))
                .count();
            if env_count > 0 {
                println!("{env_count} credential(s) from environment variables");
            }
        }
        CredentialCommands::Import => {
            #[cfg(not(feature = "keyring-store"))]
            anyhow::bail!("keyring support not compiled (enable 'keyring-store' feature)");

            #[cfg(feature = "keyring-store")]
            {
                let config = load_config(None)?;
                let mut imported = 0u32;

                for &name in CREDENTIAL_NAMES {
                    if let Some(val) = get_credential_value(&config, name)
                        && !val.is_empty()
                    {
                        match crate::config::credentials::keyring_set(name, val) {
                            Ok(()) => {
                                println!("  Imported {name}");
                                imported += 1;
                            }
                            Err(e) => {
                                eprintln!("  Failed to import {name}: {e}");
                            }
                        }
                    }
                }

                if imported == 0 {
                    println!("No credentials to import (all slots empty in config).");
                } else {
                    println!(
                        "\nImported {imported} credential(s) into keyring.\n\
                         You can now remove them from config.json if desired."
                    );
                }
            }
        }
    }
    Ok(())
}
