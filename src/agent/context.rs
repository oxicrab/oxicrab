use crate::agent::memory::MemoryStore;
use crate::agent::skills::SkillsLoader;
use anyhow::{Context, Result};
use chrono::{Datelike, Local};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;

const BOOTSTRAP_FILES: &[&str] = &["AGENTS.md", "SOUL.md", "USER.md", "TOOLS.md", "IDENTITY.md"];

pub struct ContextBuilder {
    workspace: PathBuf,
    memory: MemoryStore,
    skills: SkillsLoader,
    bootstrap_cache: Option<String>,
    bootstrap_mtimes: HashMap<String, u64>,
}

impl ContextBuilder {
    pub fn new(workspace: impl AsRef<Path>) -> Result<Self> {
        let workspace = workspace.as_ref().to_path_buf();

        // Ensure workspace exists and is accessible
        std::fs::create_dir_all(&workspace).with_context(|| {
            format!(
                "Failed to create workspace directory: {}",
                workspace.display()
            )
        })?;

        let memory = MemoryStore::new(&workspace).with_context(|| {
            format!(
                "Failed to initialize memory store for workspace: {}",
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
        })
    }

    pub fn build_system_prompt(
        &mut self,
        _skill_names: Option<&[String]>,
        query: Option<&str>,
    ) -> Result<String> {
        let mut parts = Vec::new();

        // Core identity
        parts.push(self.get_identity()?);

        // Bootstrap files
        let bootstrap = self.load_bootstrap_files()?;
        if !bootstrap.is_empty() {
            parts.push(bootstrap);
        }

        // Memory context
        let memory = self.memory.get_memory_context(query)?;
        if !memory.is_empty() {
            parts.push(format!("# Memory\n\n{}", memory));
        }

        // Skills - progressive loading
        // 1. Always-loaded skills: include full content
        let always_skills = self.skills.get_always_skills();
        if !always_skills.is_empty() {
            let always_content = self.skills.load_skills_for_context(&always_skills);
            if !always_content.is_empty() {
                parts.push(format!("# Active Skills\n\n{}", always_content));
            }
        }

        // 2. Available skills: only show summary (agent uses read_file to load)
        let skills_summary = self.skills.build_skills_summary();
        if !skills_summary.is_empty() {
            parts.push(format!(
                "# Skills\n\nThe following skills extend your capabilities. To use a skill, read its SKILL.md file using the read_file tool.\nSkills with available=\"false\" need dependencies installed first - you can try installing them with apt/brew.\n\n{}",
                skills_summary
            ));
        }

        Ok(parts.join("\n\n---\n\n"))
    }

    fn get_identity(&self) -> Result<String> {
        let now = Local::now();
        let date_str = format!(
            "{}-{:02}-{:02} ({}) {}",
            now.year(),
            now.month(),
            now.day(),
            now.format("%A"),
            now.format("%H:%M %Z")
        );

        let workspace_path = self
            .workspace
            .canonicalize()
            .unwrap_or_else(|_| self.workspace.clone())
            .to_string_lossy()
            .to_string();

        let runtime = format!("Rust {}", env!("CARGO_PKG_VERSION"));

        // Try to load identity from IDENTITY.md
        let identity_file = self.workspace.join("IDENTITY.md");
        if identity_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&identity_file) {
                return Ok(self.build_identity_with_context(
                    &content,
                    &date_str,
                    &runtime,
                    &workspace_path,
                ));
            }
            warn!("Failed to load IDENTITY.md, using defaults");
        }

        // Fallback to defaults
        Ok(self.get_default_identity(&date_str, &runtime, &workspace_path))
    }

    fn build_identity_with_context(
        &self,
        identity_content: &str,
        now: &str,
        runtime: &str,
        workspace_path: &str,
    ) -> String {
        format!(
            "{}\n\n## Current Context\n\n**Date**: {}\n**Runtime**: {}\n**Workspace**: {}\n- Memory files: {}/memory/MEMORY.md\n- Daily notes: {}/memory/YYYY-MM-DD.md\n- Custom skills: {}/skills/{{skill-name}}/SKILL.md\n\n{}",
            identity_content, now, runtime, workspace_path, workspace_path, workspace_path, workspace_path,
            self.get_behavioural_notes(workspace_path)
        )
    }

    fn get_default_identity(&self, now: &str, runtime: &str, workspace_path: &str) -> String {
        format!(
            "# nanobot\n\nYou are nanobot, a helpful AI assistant. You have access to tools that allow you to:\n- Read, write, and edit files\n- Execute shell commands\n- Search the web and fetch web pages\n- Send messages to users on chat channels\n- Spawn subagents for complex background tasks\n\n## Current Date\n{}\n\n## Runtime\n{}\n\n## Workspace\nYour workspace is at: {}\n- Memory files: {}/memory/MEMORY.md\n- Daily notes: {}/memory/YYYY-MM-DD.md\n- Custom skills: {}/skills/{{skill-name}}/SKILL.md\n\n{}",
            now, runtime, workspace_path, workspace_path, workspace_path, workspace_path,
            self.get_behavioural_notes(workspace_path)
        )
    }

    fn get_behavioural_notes(&self, workspace_path: &str) -> String {
        format!(
            "IMPORTANT: When responding to direct questions or conversations, reply directly with your text response.\nOnly use the 'message' tool when you need to send a message to a specific chat channel (like WhatsApp).\nFor normal conversation, just respond with text - do not call the message tool.\n\nAlways be helpful, accurate, and concise. When using tools, explain what you're doing.\nWhen remembering something, write to {}/memory/MEMORY.md\n\nCRITICAL: Never invent, guess, or make up information. If you don't know something:\n- Say \"I don't know\" or \"I'm not sure\" clearly\n- Use tools (web_search, read_file) to find accurate information before answering\n- Never guess file paths, command syntax, API details, or factual claims\n- When uncertain, ask the user for clarification rather than assuming\n\n## Action Integrity\nNever claim you performed an action (created, updated, wrote, deleted, configured, set up, etc.) unless you actually called a tool to do it in this conversation turn. If you cannot perform the requested action, explain what you would need to do and offer to do it. Do not say \"I've updated the file\" or \"Done!\" without having used the appropriate tool.\n\nWhen asked to retry, re-run, or re-check something, you MUST actually call the tool again. Never repeat a previous result from conversation history — conditions may have changed.",
            workspace_path
        )
    }

    fn load_bootstrap_files(&mut self) -> Result<String> {
        let mut current_mtimes = HashMap::new();

        for filename in BOOTSTRAP_FILES {
            if *filename == "IDENTITY.md" {
                continue; // Handled separately
            }
            let file_path = self.workspace.join(filename);
            if file_path.exists() {
                if let Ok(metadata) = std::fs::metadata(&file_path) {
                    if let Ok(mtime) = metadata.modified() {
                        if let Ok(duration) = mtime.duration_since(std::time::UNIX_EPOCH) {
                            current_mtimes.insert(filename.to_string(), duration.as_secs());
                        }
                    }
                }
            }
        }

        // Return cached if unchanged
        if let Some(ref cache) = self.bootstrap_cache {
            if current_mtimes == self.bootstrap_mtimes {
                return Ok(cache.clone());
            }
        }

        // Rebuild from disk
        let mut parts = Vec::new();
        for filename in BOOTSTRAP_FILES {
            if *filename == "IDENTITY.md" {
                continue;
            }
            let file_path = self.workspace.join(filename);
            if file_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&file_path) {
                    parts.push(format!("## {}\n\n{}", filename, content));
                }
            }
        }

        let cache = parts.join("\n\n");
        self.bootstrap_cache = Some(cache.clone());
        self.bootstrap_mtimes = current_mtimes;
        Ok(cache)
    }

    pub fn build_messages(
        &mut self,
        history: &[HashMap<String, serde_json::Value>],
        current_message: &str,
        channel: Option<&str>,
        chat_id: Option<&str>,
    ) -> Result<Vec<crate::providers::base::Message>> {
        let mut messages = Vec::new();

        // System prompt
        let mut system_prompt = self.build_system_prompt(None, Some(current_message))?;
        if let (Some(ch), Some(cid)) = (channel, chat_id) {
            system_prompt.push_str(&format!(
                "\n\n## Current Session\nChannel: {}\nChat ID: {}",
                ch, cid
            ));
        }
        messages.push(crate::providers::base::Message::system(system_prompt));

        // History (skip messages with empty content — Anthropic rejects empty text blocks)
        for msg in history {
            if let (Some(role), Some(content)) = (
                msg.get("role").and_then(|v| v.as_str()),
                msg.get("content").and_then(|v| v.as_str()),
            ) {
                if !content.is_empty() {
                    messages.push(crate::providers::base::Message {
                        role: role.to_string(),
                        content: content.to_string(),
                        ..Default::default()
                    });
                }
            }
        }

        // Current message with local time prefix
        let time_prefix = format!("[{}] ", Local::now().format("%H:%M"));
        messages.push(crate::providers::base::Message::user(format!(
            "{}{}",
            time_prefix, current_message
        )));

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
        _reasoning_content: Option<&str>,
    ) {
        let msg = crate::providers::base::Message::assistant(content.unwrap_or(""), tool_calls);
        messages.push(msg);
    }
}
