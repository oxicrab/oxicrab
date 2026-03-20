pub mod providers;

use crate::agent::memory::MemoryStore;
use crate::agent::skills::SkillsLoader;
use anyhow::{Context, Result};
use chrono::{Datelike, Local};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::warn;

const BOOTSTRAP_FILES: &[&str] = &["USER.md", "TOOLS.md", "AGENTS.md"];
/// Maximum size for a single context file (500 KB)
const MAX_CONTEXT_FILE_SIZE: u64 = 500 * 1024;

pub struct ContextBuilder {
    workspace: PathBuf,
    memory: Arc<MemoryStore>,
    skills: SkillsLoader,
    bootstrap_cache: Option<String>,
    bootstrap_mtimes: HashMap<String, u64>,
    providers: Option<Arc<providers::ContextProviderRunner>>,
    cached_provider_context: Option<String>,
}

impl ContextBuilder {
    /// Create a new `ContextBuilder` with an externally-owned `MemoryStore`.
    /// This ensures the context builder shares the same (embedding-configured)
    /// memory store as the rest of the agent loop.
    pub fn with_memory(workspace: impl AsRef<Path>, memory: Arc<MemoryStore>) -> Result<Self> {
        let workspace = workspace.as_ref().to_path_buf();

        // Ensure workspace exists and is accessible
        std::fs::create_dir_all(&workspace).with_context(|| {
            format!(
                "Failed to create workspace directory: {}",
                workspace.display()
            )
        })?;

        // Try to find builtin skills directory (relative to executable or workspace)
        let builtin_skills = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.join("skills")))
            .filter(|p| p.exists())
            .or_else(|| {
                // Fallback: check workspace/skills or common locations
                let ws_skills = workspace.join("skills");
                if ws_skills.exists() {
                    Some(ws_skills)
                } else {
                    None
                }
            });

        let skills = SkillsLoader::new(&workspace, builtin_skills);

        Ok(Self {
            workspace,
            memory,
            skills,
            bootstrap_cache: None,
            bootstrap_mtimes: HashMap::new(),
            providers: None,
            cached_provider_context: None,
        })
    }

    /// Create a new `ContextBuilder` with its own `MemoryStore`.
    /// Convenience for tests and standalone usage where no shared store exists.
    pub fn new(workspace: impl AsRef<Path>) -> Result<Self> {
        let workspace = workspace.as_ref();

        std::fs::create_dir_all(workspace).with_context(|| {
            format!(
                "Failed to create workspace directory: {}",
                workspace.display()
            )
        })?;

        let memory = Arc::new(MemoryStore::new(workspace).with_context(|| {
            format!(
                "Failed to initialize memory store for workspace: {}",
                workspace.display()
            )
        })?);

        Self::with_memory(workspace, memory)
    }

    pub fn set_providers(&mut self, runner: Arc<providers::ContextProviderRunner>) {
        self.providers = Some(runner);
    }

    pub async fn refresh_provider_context(&mut self) {
        if let Some(ref runner) = self.providers {
            let ctx = runner.get_all_context().await;
            self.cached_provider_context = if ctx.is_empty() { None } else { Some(ctx) };
        }
    }

    pub fn build_system_prompt(
        &mut self,
        _skill_names: Option<&[String]>,
        query: Option<&str>,
    ) -> Result<String> {
        self.build_system_prompt_inner(query, false)
    }

    fn build_system_prompt_inner(&mut self, query: Option<&str>, is_group: bool) -> Result<String> {
        let mut parts = Vec::new();

        // Core identity
        parts.push(self.get_identity());

        // Bootstrap files
        let bootstrap = self.load_bootstrap_files();
        if !bootstrap.is_empty() {
            parts.push(bootstrap);
        }

        // Memory context (skip personal memory in group chats)
        let memory = if is_group {
            self.memory.get_memory_context_scoped(query, true)?
        } else {
            self.memory.get_memory_context(query)?
        };
        if !memory.is_empty() {
            parts.push(format!("# Memory\n\n{memory}"));
        }

        // Dynamic context from external providers
        if let Some(ref ctx) = self.cached_provider_context {
            parts.push(ctx.clone());
        }

        // Skills - progressive loading
        // 1. Always-loaded skills: include full content
        let always_skills = self.skills.get_always_skills();
        if !always_skills.is_empty() {
            let always_content = self.skills.load_skills_for_context(&always_skills);
            if !always_content.is_empty() {
                parts.push(format!("# Active Skills\n\n{always_content}"));
            }
        }

        // 2. Available skills: only show summary (agent uses read_file to load)
        let skills_summary = self.skills.build_skills_summary();
        if !skills_summary.is_empty() {
            parts.push(format!(
                "# Skills\n\nThe following skills extend your capabilities. To use a skill, read its SKILL.md file using the read_file tool.\nSkills with available=\"false\" need dependencies installed first - you can try installing them with apt/brew.\n\n{skills_summary}"
            ));
        }

        Ok(parts.join("\n\n---\n\n"))
    }

    fn get_identity(&self) -> String {
        let now = Local::now();
        let date_str = format!(
            "{}-{:02}-{:02} ({}) {}",
            now.year(),
            now.month(),
            now.day(),
            now.format("%A"),
            now.format("%H:%M %Z")
        );
        let tz_str = now.format("%Z").to_string();
        // Natural-language datetime for prominence (LLMs respond better to this)
        let datetime_natural = now.format("%A, %B %-d, %Y at %H:%M %Z").to_string();

        let workspace_path = self
            .workspace
            .canonicalize()
            .unwrap_or_else(|_| self.workspace.clone())
            .to_string_lossy()
            .to_string();

        let runtime = format!("Rust {}", env!("CARGO_PKG_VERSION"));

        // Try to load identity from AGENTS.md
        let identity_file = self.workspace.join("AGENTS.md");
        if identity_file.exists() {
            if let Ok(meta) = std::fs::metadata(&identity_file)
                && meta.len() > MAX_CONTEXT_FILE_SIZE
            {
                warn!(
                    "AGENTS.md is too large ({} bytes, max {}), using defaults",
                    meta.len(),
                    MAX_CONTEXT_FILE_SIZE
                );
            } else if let Ok(content) = std::fs::read_to_string(&identity_file) {
                return Self::build_identity_with_context(
                    &content,
                    &date_str,
                    &tz_str,
                    &runtime,
                    &workspace_path,
                    &datetime_natural,
                );
            }
            warn!("Failed to load AGENTS.md, using defaults");
        }

        // Fallback to defaults
        Self::get_default_identity(
            &date_str,
            &tz_str,
            &runtime,
            &workspace_path,
            &datetime_natural,
        )
    }

    fn build_identity_with_context(
        identity_content: &str,
        now: &str,
        tz: &str,
        runtime: &str,
        workspace_path: &str,
        datetime_natural: &str,
    ) -> String {
        format!(
            "The current date and time is {datetime_natural}.\n\n{identity_content}\n\n## Current Context\n\n**Date**: {now}\n**Timezone**: {tz}\n**Runtime**: {runtime}\n**Workspace**: {workspace_path}\n- Memory: SQLite database in {workspace_path}/memory/\n- Custom skills: {workspace_path}/skills/{{skill-name}}/SKILL.md",
        )
    }

    fn get_default_identity(
        now: &str,
        tz: &str,
        runtime: &str,
        workspace_path: &str,
        datetime_natural: &str,
    ) -> String {
        format!(
            "The current date and time is {datetime_natural}.\n\n# oxicrab\n\nYou are oxicrab, a helpful AI assistant.\n\n## Capabilities\n\n- Read, write, and edit files\n- Execute shell commands\n- Search the web and fetch web pages\n- Communicate with users across chat channels\n- Attach interactive buttons to messages (use the add_buttons tool — works on Slack and Discord)\n- Spawn subagents for complex background tasks\n\n## Tool Usage Rules\n\n- NEVER claim to have called a tool or report tool results unless you actually invoked the tool in this conversation.\n- NEVER fabricate or simulate tool output. If you need data, call the tool.\n- If asked to test or run tools, you MUST call each tool individually and report the real results.\n- If a tool is unavailable or fails, say so explicitly — do not invent results.\n- If a tool returns unexpected or limited output, report what it returned honestly. Do NOT fabricate explanations for why the tool behaved that way — say you are unsure of the cause.\n- If you need to use tools, call them directly — never send a preliminary message like \"Let me check\" without actually calling a tool in the same response.\n\n## Interactive Buttons\n\nUse add_buttons after tool results that have natural follow-up actions:\n- **Tasks** (todoist, google_tasks): Complete, Snooze, Edit, Delete buttons after listing or showing tasks\n- **Calendar** (google_calendar): RSVP, Edit, Delete buttons after showing events\n- **Email** (google_mail): Reply, Archive, Label buttons after reading messages\n- **GitHub**: Approve, Request Changes buttons after showing PRs; Close, Label after issues\n- **Cron**: Pause, Remove buttons after listing scheduled jobs\n- **General**: Confirmation buttons before destructive actions (delete, bulk operations)\n\nOnly attach buttons on Slack/Discord channels. Max 5 per message. Use clear, short labels.\n\nNote: Many tools automatically attach relevant buttons to their results (e.g. Complete for tasks, RSVP for calendar events, Approve for PRs). You don't need to call add_buttons for these — they appear automatically. Use add_buttons only for additional buttons beyond what tools provide.\n\n## Current Context\n\n**Date**: {now}\n**Timezone**: {tz}\n**Runtime**: {runtime}\n**Workspace**: {workspace_path}\n- Memory: SQLite database in {workspace_path}/memory/\n- Custom skills: {workspace_path}/skills/{{skill-name}}/SKILL.md",
        )
    }

    fn load_bootstrap_files(&mut self) -> String {
        let mut current_mtimes = HashMap::new();

        for filename in BOOTSTRAP_FILES {
            // AGENTS.md is loaded separately via get_identity() since it provides
            // the core identity/persona, not just supplemental context
            if *filename == "AGENTS.md" {
                continue;
            }
            let file_path = self.workspace.join(filename);
            if file_path.exists()
                && let Ok(metadata) = std::fs::metadata(&file_path)
                && let Ok(mtime) = metadata.modified()
                && let Ok(duration) = mtime.duration_since(std::time::UNIX_EPOCH)
            {
                current_mtimes.insert(filename.to_string(), duration.as_secs());
            }
        }

        // Return cached if unchanged
        if let Some(ref cache) = self.bootstrap_cache
            && current_mtimes == self.bootstrap_mtimes
        {
            return cache.clone();
        }

        // Rebuild from disk
        let mut parts = Vec::new();
        for filename in BOOTSTRAP_FILES {
            if *filename == "AGENTS.md" {
                continue; // Loaded via get_identity()
            }
            let file_path = self.workspace.join(filename);
            if file_path.exists() {
                // Check file size before reading to prevent OOM from huge files
                if let Ok(meta) = std::fs::metadata(&file_path)
                    && meta.len() > MAX_CONTEXT_FILE_SIZE
                {
                    warn!(
                        "{} is too large ({} bytes, max {}), skipping",
                        filename,
                        meta.len(),
                        MAX_CONTEXT_FILE_SIZE
                    );
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&file_path) {
                    parts.push(format!("## {filename}\n\n{content}"));
                }
            }
        }

        let cache = parts.join("\n\n");
        self.bootstrap_cache = Some(cache.clone());
        self.bootstrap_mtimes = current_mtimes;
        cache
    }

    /// Return a channel-specific formatting hint for the system prompt.
    fn channel_formatting_hint(channel: &str) -> Option<&'static str> {
        match channel {
            "discord" => Some(
                "Formatting: Markdown supported but NOT tables. Wrap URLs in <> to suppress embeds. Max 2000 chars per message.",
            ),
            "telegram" => Some(
                "Formatting: Bold, italic, code, and bullet lists work. Tables NOT supported. Max 4096 chars per message.",
            ),
            "slack" => Some(
                "Formatting: Use Slack mrkdwn — *bold*, _italic_, `code`. Standard markdown ** does NOT work. Prefer threaded replies.",
            ),
            "whatsapp" => Some(
                "Formatting: Keep messages concise. Headers/tables ignored. Bold (*text*) and italic (_text_) work.",
            ),
            "twilio" => Some("Formatting: Plain text only (SMS). Keep responses very concise."),
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_messages(
        &mut self,
        history: &[HashMap<String, serde_json::Value>],
        current_message: &str,
        channel: Option<&str>,
        chat_id: Option<&str>,
        sender_id: Option<&str>,
        images: Vec<crate::providers::base::ImageData>,
        is_group: bool,
        entity_context: Option<&str>,
    ) -> Result<Vec<crate::providers::base::Message>> {
        let mut messages = Vec::new();

        // System prompt
        let mut system_prompt = self.build_system_prompt_inner(Some(current_message), is_group)?;
        if let (Some(ch), Some(cid)) = (channel, chat_id) {
            let mut session_info = format!("\n\n## Current Session\nChannel: {ch}\nChat ID: {cid}");
            if let Some(sid) = sender_id {
                use std::fmt::Write as _;
                let _ = write!(session_info, "\nSender: {sid}");
            }
            if let Some(hint) = Self::channel_formatting_hint(ch) {
                use std::fmt::Write as _;
                let _ = write!(session_info, "\n{hint}");
            }
            system_prompt.push_str(&session_info);
        }
        // Tell the model that the history below IS its real conversation, so it
        // doesn't claim it "can't look up past messages" or needs a tool to do so.
        if !history.is_empty() {
            system_prompt.push_str(
                "\n\n## Conversation History\n\n\
                 The messages below are your actual conversation history with this user. \
                 You do NOT need any tool to recall what was said — it is right here. \
                 CRITICAL: When the user says \"that\", \"it\", \"this one\", \"the task\", \
                 \"close it\", \"complete that\", \"mark it done\", etc., resolve the reference \
                 from the messages below and ACT. Never respond with \"What would you like me to ...?\" \
                 or \"Which one?\" when the referent is clear from this history.",
            );
        }

        // Inject tracked entities so the LLM has a structured reference for resolution
        if let Some(ctx) = entity_context {
            use std::fmt::Write as _;
            let _ = write!(
                system_prompt,
                "\n\n## Recently Referenced Entities\n\n\
                 These entities were mentioned in this conversation. When the user refers to \
                 \"that\", \"it\", \"the task\", etc., match to an entity below and ACT on it. \
                 Do NOT ask which one — if one entity matches, use it.\n\n{ctx}"
            );
        }

        messages.push(crate::providers::base::Message::system(system_prompt));

        // History — reconstruct Message structs from session HashMap data.
        // Anthropic rejects empty text blocks, so skip messages with no content
        // UNLESS they are assistant messages with tool_calls (tool-only responses).
        for msg in history {
            let Some(role) = msg.get("role").and_then(|v| v.as_str()) else {
                continue;
            };
            // Only allow valid conversation roles — reject injected "system" messages
            if !matches!(role, "user" | "assistant" | "tool") {
                warn!("skipping history message with invalid role: {role}");
                continue;
            }
            // Extract content (string or first text element from array)
            let content = msg
                .get("content")
                .and_then(|v| {
                    v.as_str().map(String::from).or_else(|| {
                        // Handle content-block arrays (Anthropic format)
                        v.as_array().and_then(|arr| {
                            arr.iter()
                                .find_map(|b| b.get("text").and_then(|t| t.as_str()))
                                .map(String::from)
                        })
                    })
                })
                .unwrap_or_default();
            // Reconstruct tool_calls for assistant messages
            let tool_calls: Option<Vec<crate::providers::base::ToolCallRequest>> = msg
                .get("tool_calls")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .or_else(|| {
                    // Also check Anthropic-style content array for tool_use blocks
                    msg.get("content").and_then(|v| {
                        v.as_array().map(|arr| {
                            arr.iter()
                                .filter_map(|b| {
                                    if b.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                                        Some(crate::providers::base::ToolCallRequest {
                                            id: b
                                                .get("id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or_default()
                                                .to_string(),
                                            name: b
                                                .get("name")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or_default()
                                                .to_string(),
                                            arguments: b.get("input").cloned().unwrap_or_default(),
                                        })
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                })
                .filter(|tc| !tc.is_empty());
            let tool_call_id = msg
                .get("tool_call_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let reasoning = msg
                .get("reasoning_content")
                .and_then(|v| v.as_str())
                .map(String::from);
            let reasoning_sig = msg
                .get("reasoning_signature")
                .and_then(|v| v.as_str())
                .map(String::from);
            // Skip empty messages unless they have tool_calls (tool-only assistant turns)
            if content.is_empty() && tool_calls.is_none() && tool_call_id.is_none() {
                continue;
            }
            messages.push(crate::providers::base::Message {
                role: role.to_string(),
                content,
                tool_calls,
                tool_call_id,
                reasoning_content: reasoning,
                reasoning_signature: reasoning_sig,
                ..Default::default()
            });
        }

        let time_prefix = format!("[{}] ", Local::now().format("%H:%M"));
        let user_content = format!("{time_prefix}{current_message}");
        if images.is_empty() {
            messages.push(crate::providers::base::Message::user(user_content));
        } else {
            messages.push(crate::providers::base::Message::user_with_images(
                user_content,
                images,
            ));
        }

        Ok(messages)
    }

    pub fn add_tool_result(
        messages: &mut Vec<crate::providers::base::Message>,
        tool_call_id: &str,
        _tool_name: &str,
        result: &str,
        is_error: bool,
    ) {
        messages.push(crate::providers::base::Message::tool_result(
            tool_call_id,
            result,
            is_error,
        ));
    }

    pub fn add_assistant_message(
        messages: &mut Vec<crate::providers::base::Message>,
        content: Option<&str>,
        tool_calls: Option<Vec<crate::providers::base::ToolCallRequest>>,
        reasoning_content: Option<&str>,
        reasoning_signature: Option<&str>,
        redacted_thinking_blocks: Option<Vec<String>>,
    ) {
        let mut msg = crate::providers::base::Message::assistant_with_thinking(
            content.unwrap_or_default(),
            tool_calls,
            reasoning_content.map(String::from),
            reasoning_signature.map(String::from),
        );
        msg.redacted_thinking_blocks = redacted_thinking_blocks;
        messages.push(msg);
    }
}

#[cfg(test)]
mod tests;
