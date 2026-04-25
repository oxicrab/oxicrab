use anyhow::{Context, Result};
use std::io::Write;
use tracing::debug;

use crate::config::Config;

/// Provider preset offered by the wizard. The first matching env var
/// gets recorded into the keyring (or env-suggestion line) so the
/// generated config has a runnable default.
struct ProviderPreset {
    label: &'static str,
    /// Default `provider/model` string written to `agents.defaults.modelRouting.default`.
    model_default: &'static str,
    /// Where the user can grab an API key.
    key_url: &'static str,
}

const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        label: "openrouter",
        model_default: "openrouter/anthropic/claude-sonnet-4.6",
        key_url: "https://openrouter.ai/keys",
    },
    ProviderPreset {
        label: "anthropic",
        model_default: "anthropic/claude-sonnet-4-5-20250929",
        key_url: "https://console.anthropic.com/settings/keys",
    },
    ProviderPreset {
        label: "openai",
        model_default: "openai/gpt-4o",
        key_url: "https://platform.openai.com/api-keys",
    },
    ProviderPreset {
        label: "ollama",
        model_default: "ollama/llama3.3:70b",
        key_url: "https://ollama.com/library  (run `ollama pull` first)",
    },
];

const CHANNEL_OPTIONS: &[&str] = &[
    "telegram",
    "discord",
    "slack",
    "whatsapp",
    "twilio",
    "(none — use CLI only for now)",
];

fn read_line(prompt: &str) -> Result<String> {
    eprint!("{prompt}");
    std::io::stderr().flush().ok();
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

fn read_choice(prompt: &str, options: &[&str], default: usize) -> Result<usize> {
    println!("\n{prompt}");
    for (i, opt) in options.iter().enumerate() {
        let marker = if i == default { "*" } else { " " };
        println!(" {marker} {}. {opt}", i + 1);
    }
    let raw = read_line(&format!("[1-{}, default {}]: ", options.len(), default + 1))?;
    if raw.is_empty() {
        return Ok(default);
    }
    match raw.parse::<usize>() {
        Ok(n) if n >= 1 && n <= options.len() => Ok(n - 1),
        _ => Ok(default),
    }
}

pub(super) fn onboard() -> Result<()> {
    println!("\u{1f916} oxicrab onboarding wizard");
    println!("This walks through provider, channels, and writes a config.toml.\n");

    let config_path = crate::config::get_config_path()?;
    if config_path.exists() {
        println!(
            "\u{26a0}\u{fe0f}  Config already exists at {}",
            config_path.display()
        );
        let confirm = read_line("Overwrite? (y/N): ")?;
        if !confirm.eq_ignore_ascii_case("y") {
            println!("Aborted. Run `oxicrab doctor` to inspect the existing config.");
            return Ok(());
        }
    }

    // ── Provider ──
    let provider_labels: Vec<&str> = PROVIDER_PRESETS.iter().map(|p| p.label).collect();
    let provider_idx = read_choice(
        "Which LLM provider should be the default?",
        &provider_labels,
        0,
    )?;
    let preset = &PROVIDER_PRESETS[provider_idx];

    // ── Channel ──
    let channel_idx = read_choice(
        "Which messaging channel will you use first? (you can add more later)",
        CHANNEL_OPTIONS,
        CHANNEL_OPTIONS.len() - 1,
    )?;
    let channel_choice = CHANNEL_OPTIONS[channel_idx];

    // ── Build config ──
    let mut config = Config::default();
    config.agents.defaults.model_routing.default = preset.model_default.to_string();

    // Hook the chosen channel on so it surfaces in `oxicrab doctor`. We
    // don't try to capture tokens interactively — keys belong in the
    // keyring or env, not echoed at the wizard prompt.
    match channel_choice {
        "telegram" => config.channels.telegram.enabled = true,
        "discord" => config.channels.discord.enabled = true,
        "slack" => config.channels.slack.enabled = true,
        "whatsapp" => config.channels.whatsapp.enabled = true,
        "twilio" => config.channels.twilio.enabled = true,
        _ => {}
    }

    crate::config::save_config(&config, Some(config_path.as_path()))?;
    println!("\u{2713} Wrote config to {}", config_path.display());

    let workspace = config.workspace_path();
    crate::utils::ensure_dir(&workspace)?;
    println!("\u{2713} Workspace at {}", workspace.display());
    create_workspace_templates(&workspace)?;

    // ── Next-step guidance tailored to choices ──
    println!("\n\u{1f916} oxicrab is ready. Next steps:");
    let mut step = 1;
    println!(
        "  {step}. Set your {} API key (env or keyring):",
        preset.label
    );
    let env_var = match preset.label {
        "openrouter" => "OXICRAB_OPENROUTER_API_KEY",
        "anthropic" => "OXICRAB_ANTHROPIC_API_KEY",
        "openai" => "OXICRAB_OPENAI_API_KEY",
        "ollama" => "(ollama runs locally — no API key needed)",
        _ => "OXICRAB_*_API_KEY",
    };
    if preset.label == "ollama" {
        println!("     Make sure `ollama serve` is running and the model is pulled.");
    } else {
        println!("     export {env_var}=...");
        println!("     OR: oxicrab credentials set {} <KEY>", preset.label);
        println!("     Get a key at: {}", preset.key_url);
    }
    step += 1;

    if channel_choice != "(none — use CLI only for now)" {
        println!(
            "  {step}. Configure {channel_choice} credentials in [channels.{channel_choice}] of \
             {}",
            config_path.display()
        );
        step += 1;
    }
    println!("  {step}. Run `oxicrab doctor` to validate everything.");
    println!("  {}. Chat: `oxicrab agent -m \"Hello!\"`", step + 1);

    Ok(())
}

pub(super) fn create_workspace_templates(workspace: &std::path::Path) -> Result<()> {
    debug!("creating workspace templates in: {}", workspace.display());

    let templates = vec![
        (
            "USER.md",
            r"# User

Information about the user goes here.

## Preferences

- Communication style: (casual/formal)
- Timezone: (your timezone)
- Language: (your preferred language)
",
        ),
        (
            "AGENTS.md",
            r#"# oxicrab

I am oxicrab, a personal AI assistant.

## Personality

- Friendly but professional
- Direct and concise, with detail when needed
- Accuracy over speed

## Capabilities

I have access to a variety of tools including file operations, web search, shell commands, messaging, and more. Some tools (Google services, GitHub, weather, etc.) require additional configuration.

## Behavioral Rules

- When responding to direct questions or conversations, reply directly with text. Your text response will be delivered to the user automatically.
- Always be helpful, accurate, and concise. When using tools, explain what you're doing.
- NEVER ask "which task?", "which one?", or "what would you like me to ...?" when the answer is \
obvious from conversation context. If you just listed one item, discussed a specific entity, or the \
user just asked you to create/do something, and they then say "that", "it", "the task", "close it", \
"complete that", "mark it done" — resolve the reference and act immediately. Asking for clarification \
when context is clear is a failure, not a safety feature.
- Only ask for clarification when there are genuinely multiple equally-likely referents AND the action \
is irreversible, or when required parameters are truly missing (not just implied by context).
- Examples of CORRECT behavior: User says "add a task for X" → you create it → user says "complete that" \
→ you complete the task you just created. User says "list my tasks" → one task returned → user says \
"delete it" → you delete that task.
- Never invent, guess, or make up information. If you don't know something:
  - Say "I don't know" or "I'm not sure" clearly
  - Use tools (web_search, read_file) to find accurate information before answering
  - Never guess file paths, command syntax, API details, or factual claims

### Action Integrity

Never claim you performed an action (created, updated, wrote, deleted, configured, set up, etc.) unless you actually called a tool to do it in this conversation turn. If you cannot perform the requested action, explain what you would need to do and offer to do it.

When asked to retry, re-run, or re-check something, you MUST actually call the tool again. Never repeat a previous result from conversation history.

Never volunteer apologies or commentary about past discrepancies. If a tool reveals that a previously discussed item doesn't exist or differs from what was discussed, silently fix it and report the current outcome. Do not say "it wasn't actually created" or "I apologize for the earlier error" — the user may have no awareness of any issue, and raising it unprompted causes confusion.

Before concluding that a previously discussed item doesn't exist, search thoroughly. If a filtered search fails or returns an error, retry with a broader filter or list without filters and scan the full results. Conversation history saying something was created is strong evidence it exists — a single failed search does not override that. Never create a duplicate item without first exhausting search options.

## Memory Management

I actively maintain my memory to be useful across sessions. Memory is stored in a SQLite database.

- **AGENTS.md**: My own identity. Update the "Learned Adaptations" section when I discover consistent user preferences
- **USER.md**: User preferences and habits. Update when I notice patterns

Be selective — only record genuinely useful facts, not transient conversation details.

## Learned Adaptations

*(This section is updated as I learn about user preferences)*
"#,
        ),
        (
            "TOOLS.md",
            r"# Tool Notes

Notes and configuration details for tools.

## Configured Tools

*(List tools you've configured and any important notes about them)*

## API Keys & Services

*(Record which services are set up — do NOT store actual keys here)*
",
        ),
    ];

    for (filename, content) in templates {
        let file_path = workspace.join(filename);
        if file_path.exists() {
            debug!("template already exists: {filename}");
        } else {
            std::fs::write(&file_path, content)
                .with_context(|| format!("failed to write template: {}", file_path.display()))?;
            println!("  Created {filename}");
        }
    }

    // Create memory directory (SQLite DB lives here)
    let memory_dir = workspace.join("memory");
    crate::utils::ensure_dir(&memory_dir)?;

    Ok(())
}
