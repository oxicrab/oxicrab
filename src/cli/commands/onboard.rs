use anyhow::Result;

use crate::config::Config;

pub(super) fn onboard() -> Result<()> {
    println!("\u{1f916} Initializing oxicrab...");

    let config_path = crate::config::get_config_path()?;
    if config_path.exists() {
        println!(
            "\u{26a0}\u{fe0f}  Config already exists at {}",
            config_path.display()
        );
        println!("Overwrite? (y/N): ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            return Ok(());
        }
    }

    let config = Config::default();
    crate::config::save_config(&config, Some(config_path.as_path()))?;
    println!("\u{2713} Created config at {}", config_path.display());

    let workspace = config.workspace_path();
    crate::utils::ensure_dir(&workspace)?;
    println!("\u{2713} Created workspace at {}", workspace.display());

    create_workspace_templates(&workspace)?;

    println!("\n\u{1f916} oxicrab is ready!");
    println!("\nNext steps:");
    println!("  1. Add your API key to ~/.oxicrab/config.json");
    println!("     Get one at: https://openrouter.ai/keys");
    println!("  2. Chat: oxicrab agent -m \"Hello!\"");

    Ok(())
}

pub(super) fn create_workspace_templates(workspace: &std::path::Path) -> Result<()> {
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
        if !file_path.exists() {
            std::fs::write(&file_path, content)?;
            println!("  Created {filename}");
        }
    }

    // Create memory directory (SQLite DB lives here)
    let memory_dir = workspace.join("memory");
    crate::utils::ensure_dir(&memory_dir)?;

    Ok(())
}
