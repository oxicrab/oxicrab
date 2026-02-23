# Profile Feature — Implementation Design

**Date**: 2026-02-22
**Status**: Proposed
**Branch**: `test/improve-coverage`

---

## Overview

Add a psychographic profiling system to Oxicrab that consumes a pre-existing JSON profile (generated externally, e.g. via the Claude Project onboarding prompt in `workspace/profile-onboarding-prompt.md`) and uses it to personalize the agent's system prompt and behavior.

The onboarding conversation is **not** part of this implementation. A user generates their profile JSON externally, places it at a configured path, and Oxicrab consumes it.

No existing online tool performs this specific workflow (conversational profiling → assistant-configuration JSON). Existing personality tools are either survey-based (16Personalities, Big Five quizzes) or API-based (Crystal, Humantic AI) and none output assistant directive markdown. The Claude Project prompt in `workspace/profile-onboarding-prompt.md` is the recommended generation method.

---

## Architecture

### Data Flow

```
profile.json (user-provided)
    │
    ├── Startup: ProfileLoader reads + validates
    │       │
    │       ├── Renders USER.md (Tier 1: always in prompt)
    │       └── Renders PROFILE-DIRECTIVES.md (Tier 2: confidence-gated)
    │
    ├── ContextBuilder loads both as bootstrap files
    │       └── Injected into system prompt
    │
    └── Evolution: daemon cron re-analyzes conversation history
            └── Updates profile.json with shifted scores
```

### Files Modified

| File | Change |
|------|--------|
| `src/config/schema/agent.rs` | Add `ProfileConfig` struct + field on `AgentDefaults` |
| `src/config/schema/mod.rs` | Validate profile config in `Config::validate()` |
| `src/agent/profile/mod.rs` | **New module**: `ProfileLoader`, `PsychographicProfile`, rendering |
| `src/agent/context/mod.rs` | Add `PROFILE-DIRECTIVES.md` to bootstrap pipeline |
| `src/agent/loop/mod.rs` | Add `profile_config` to `AgentLoopConfig` + `from_config()` |
| `src/agent/mod.rs` | Declare `profile` module |
| `src/lib.rs` | Re-export if needed |
| `CLAUDE.md` | Document new config fields |
| `docs/_pages/config.html` | Document profile config |

### Files Not Modified

- `src/cli/commands/mod.rs` — `setup_agent()` uses `AgentLoopConfig::from_config()` which reads from config automatically; no manual wiring needed.
- `tests/common/mod.rs` — `test_defaults()` already sets `None` for optional configs; profile config would follow same pattern.

---

## Component Design

### 1. Config (`src/config/schema/agent.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileConfig {
    #[serde(default)]
    pub enabled: bool,

    /// Path to the psychographic profile JSON file.
    /// Supports `~/` expansion. Default: `{workspace}/profile.json`
    #[serde(default, rename = "profilePath")]
    pub profile_path: Option<String>,

    /// Minimum confidence score (0.0-1.0) required to inject Tier 2 directives.
    /// Below this threshold, only USER.md (Tier 1) is generated.
    #[serde(default = "default_profile_confidence_threshold", rename = "confidenceThreshold")]
    pub confidence_threshold: f32,

    /// Enable automatic profile evolution via daemon.
    /// When true, the daemon periodically re-analyzes recent conversations
    /// and updates personality scores (max +/-10 per cycle).
    #[serde(default, rename = "evolutionEnabled")]
    pub evolution_enabled: bool,
}

fn default_profile_confidence_threshold() -> f32 {
    0.6
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            profile_path: None,
            confidence_threshold: default_profile_confidence_threshold(),
            evolution_enabled: false,
        }
    }
}
```

Add to `AgentDefaults`:

```rust
#[serde(default)]
pub profile: ProfileConfig,
```

YAML config example:

```yaml
agents:
  defaults:
    profile:
      enabled: true
      profilePath: "~/.oxicrab/workspace/profile.json"
      confidenceThreshold: 0.6
      evolutionEnabled: false
```

### 2. Profile Types (`src/agent/profile/mod.rs`)

New module with ~300 lines. Key types:

```rust
/// The 9-dimension psychographic profile (matches onboarding prompt output schema).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PsychographicProfile {
    pub version: u32,
    pub preferred_name: String,
    pub personality: PersonalityTraits,
    pub communication: CommunicationStyle,
    pub cohort: UserCohort,
    pub behavior: BehavioralPatterns,
    pub friendship: FriendshipProfile,
    pub assistance: AssistancePreferences,
    pub context: ContextualInfo,
    pub relationship_values: RelationshipValues,
    pub interaction_preferences: InteractionPreferences,
    pub analysis_metadata: AnalysisMetadata,
    pub confidence: f32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalityTraits {
    pub empathy: u8,
    pub problem_solving: u8,
    pub emotional_intelligence: u8,
    pub adaptability: u8,
    pub communication: u8,
}

// ... remaining sub-structs matching the JSON schema in profile-onboarding-prompt.md
```

### 3. Profile Loader (`src/agent/profile/mod.rs`)

```rust
pub struct ProfileLoader;

impl ProfileLoader {
    /// Load and validate a profile from disk.
    /// Returns None if the file doesn't exist (not an error — profile is optional).
    pub fn load(path: &Path) -> Result<Option<PsychographicProfile>> {
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read profile: {}", path.display()))?;
        let profile: PsychographicProfile = serde_json::from_str(&content)
            .with_context(|| "failed to parse profile JSON")?;
        Self::validate(&profile)?;
        Ok(Some(profile))
    }

    /// Validate profile invariants.
    fn validate(profile: &PsychographicProfile) -> Result<()> {
        if profile.version != 2 {
            bail!("unsupported profile version: {}", profile.version);
        }
        if !(0.0..=1.0).contains(&profile.confidence) {
            bail!("confidence must be 0.0-1.0, got {}", profile.confidence);
        }
        // Personality scores 0-100
        for (name, score) in [
            ("empathy", profile.personality.empathy),
            ("problem_solving", profile.personality.problem_solving),
            ("emotional_intelligence", profile.personality.emotional_intelligence),
            ("adaptability", profile.personality.adaptability),
            ("communication", profile.personality.communication),
        ] {
            if score > 100 {
                bail!("personality.{} must be 0-100, got {}", name, score);
            }
        }
        Ok(())
    }

    /// Render Tier 1 content: USER.md
    /// Always generated when a valid profile exists.
    /// Contains: preferred name, communication preferences, key context.
    pub fn render_user_md(profile: &PsychographicProfile) -> String {
        let mut out = String::new();
        writeln!(out, "# About {}", profile.preferred_name).ok();
        writeln!(out).ok();

        // Communication style
        writeln!(out, "## Communication Preferences").ok();
        writeln!(out, "- Detail level: {}", profile.communication.detail_level).ok();
        writeln!(out, "- Tone: {}", profile.communication.tone).ok();
        writeln!(out, "- Pace: {}", profile.communication.pace).ok();
        if profile.communication.formality != "unknown" {
            writeln!(out, "- Formality: {}", profile.communication.formality).ok();
        }
        writeln!(out).ok();

        // Context
        if let Some(ref profession) = profile.context.profession {
            writeln!(out, "## Context").ok();
            writeln!(out, "- Profession: {}", profession).ok();
            if let Some(ref life_stage) = profile.context.life_stage {
                writeln!(out, "- Life stage: {}", life_stage).ok();
            }
            if !profile.context.interests.is_empty() {
                writeln!(out, "- Interests: {}", profile.context.interests.join(", ")).ok();
            }
            writeln!(out).ok();
        }

        // Goals
        if !profile.assistance.goals.is_empty() {
            writeln!(out, "## Current Goals").ok();
            for goal in &profile.assistance.goals {
                writeln!(out, "- {}", goal).ok();
            }
            writeln!(out).ok();
        }

        out
    }

    /// Render Tier 2 content: PROFILE-DIRECTIVES.md
    /// Only generated when confidence >= threshold.
    /// Contains: behavioral directives derived from personality analysis.
    pub fn render_directives_md(profile: &PsychographicProfile) -> String {
        let mut out = String::new();
        writeln!(out, "# Assistant Directives").ok();
        writeln!(out).ok();
        writeln!(out, "Adapt your behavior based on this user's profile:").ok();
        writeln!(out).ok();

        // Communication directives
        match profile.communication.detail_level.as_str() {
            "concise" => writeln!(out, "- Be brief and get to the point quickly").ok(),
            "detailed" => writeln!(out, "- Provide thorough explanations with context").ok(),
            _ => None,
        };
        match profile.communication.pace.as_str() {
            "fast" => writeln!(out, "- Keep responses quick and decisive").ok(),
            "measured" => writeln!(out, "- Take time to be thorough and considered").ok(),
            _ => None,
        };

        // Personality-derived directives
        if profile.personality.empathy > 70 {
            writeln!(out, "- User is highly empathetic; acknowledge emotional dimensions").ok();
        }
        if profile.personality.problem_solving > 70 {
            writeln!(out, "- User is a strong problem solver; present options, not just answers").ok();
        }

        // Assistance preferences
        match profile.assistance.proactivity.as_str() {
            "high" => writeln!(out, "- Be proactive: suggest next steps and anticipate needs").ok(),
            "low" => writeln!(out, "- Wait to be asked before offering help or suggestions").ok(),
            _ => None,
        };
        match profile.assistance.interaction_style.as_str() {
            "direct" => writeln!(out, "- Be direct: skip preamble, give answers first").ok(),
            "conversational" => writeln!(out, "- Be conversational: engage naturally, not robotically").ok(),
            _ => None,
        };

        // Pain points and strengths
        if !profile.behavior.pain_points.is_empty() {
            writeln!(out).ok();
            writeln!(out, "## Areas to Help With").ok();
            for point in &profile.behavior.pain_points {
                writeln!(out, "- {}", point).ok();
            }
        }
        if !profile.behavior.suggested_support.is_empty() {
            writeln!(out).ok();
            writeln!(out, "## Suggested Support").ok();
            for support in &profile.behavior.suggested_support {
                writeln!(out, "- {}", support).ok();
            }
        }

        out
    }
}
```

### 4. Integration with ContextBuilder (`src/agent/context/mod.rs`)

Two changes:

**a. Add `PROFILE-DIRECTIVES.md` to the bootstrap pipeline.**

The simplest approach: add it to `BOOTSTRAP_FILES`:

```rust
const BOOTSTRAP_FILES: &[&str] = &["USER.md", "TOOLS.md", "AGENTS.md", "PROFILE-DIRECTIVES.md"];
```

This works because `load_bootstrap_files()` already skips missing files gracefully. When the profile feature is disabled or confidence is below threshold, `PROFILE-DIRECTIVES.md` simply won't exist in the workspace and will be skipped.

USER.md is already a bootstrap file, so a profile-generated USER.md is automatically included — no code changes needed for Tier 1.

**b. Render files on startup.**

`ContextBuilder::new()` or a new `ContextBuilder::with_profile()` method would call `ProfileLoader` to render USER.md and PROFILE-DIRECTIVES.md into the workspace directory at startup. This happens once, not per-request.

```rust
impl ContextBuilder {
    /// Render profile-derived files into the workspace.
    /// Called once at startup when profile feature is enabled.
    pub fn apply_profile(&self, profile: &PsychographicProfile, confidence_threshold: f32) -> Result<()> {
        let user_md = ProfileLoader::render_user_md(profile);
        std::fs::write(self.workspace.join("USER.md"), user_md)
            .context("failed to write USER.md")?;
        info!("wrote profile-derived USER.md");

        if profile.confidence >= confidence_threshold {
            let directives = ProfileLoader::render_directives_md(profile);
            std::fs::write(self.workspace.join("PROFILE-DIRECTIVES.md"), directives)
                .context("failed to write PROFILE-DIRECTIVES.md")?;
            info!("wrote PROFILE-DIRECTIVES.md (confidence {:.2} >= threshold {:.2})",
                  profile.confidence, confidence_threshold);
        } else {
            // Remove stale directives if confidence dropped below threshold
            let directives_path = self.workspace.join("PROFILE-DIRECTIVES.md");
            if directives_path.exists() {
                std::fs::remove_file(&directives_path).ok();
                info!("removed PROFILE-DIRECTIVES.md (confidence {:.2} < threshold {:.2})",
                      profile.confidence, confidence_threshold);
            }
        }

        Ok(())
    }
}
```

**Important**: If the user already has a hand-written USER.md, the profile system should NOT overwrite it. The loader should check for an existing USER.md and either merge or skip. Recommended: skip with a warning if USER.md exists and wasn't generated by the profile system (check for a sentinel comment like `<!-- profile-generated -->`).

### 5. AgentLoop Integration (`src/agent/loop/mod.rs`)

Add to `AgentLoopConfig`:

```rust
pub profile_config: crate::config::ProfileConfig,
```

Add to `from_config()`:

```rust
profile_config: config.agents.defaults.profile.clone(),
```

Add to `test_defaults()`:

```rust
profile_config: crate::config::ProfileConfig::default(),
```

In `AgentLoop::new()`, after creating the `ContextBuilder`, apply the profile:

```rust
if config.profile_config.enabled {
    let profile_path = config.profile_config.profile_path
        .as_deref()
        .map(expand_tilde)
        .unwrap_or_else(|| config.workspace.join("profile.json"));
    match ProfileLoader::load(&profile_path) {
        Ok(Some(profile)) => {
            info!("loaded psychographic profile for {}", profile.preferred_name);
            context_builder.apply_profile(&profile, config.profile_config.confidence_threshold)?;
        }
        Ok(None) => debug!("no profile found at {}", profile_path.display()),
        Err(e) => warn!("failed to load profile: {}", e),
    }
}
```

### 6. Evolution (Optional, Behind `evolutionEnabled` Gate)

Profile evolution re-analyzes recent conversation history and updates personality scores gradually. This uses the existing daemon/heartbeat infrastructure.

**Mechanism:**

1. A new daemon strategy file `PROFILE-EVOLUTION.md` (or a hook in the existing heartbeat) runs weekly.
2. It reads recent session transcripts from the session store.
3. It sends them to the LLM with a re-analysis prompt (a subset of the onboarding analysis framework).
4. It compares new scores to existing scores and applies capped deltas (max +/-10 per personality dimension per cycle).
5. It updates `profile.json` and re-renders USER.md + PROFILE-DIRECTIVES.md.

**Confidence gating:** Only update fields where the new analysis has confidence > 0.6. This prevents low-confidence re-analysis from degrading a high-confidence initial profile.

**Implementation approach:** A new function in `src/agent/profile/mod.rs`:

```rust
impl PsychographicProfile {
    /// Apply evolution deltas from a new analysis, capping changes.
    pub fn evolve(&mut self, new: &PsychographicProfile, max_delta: u8) {
        self.personality.empathy = shift(self.personality.empathy, new.personality.empathy, max_delta);
        self.personality.problem_solving = shift(self.personality.problem_solving, new.personality.problem_solving, max_delta);
        // ... other dimensions
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }
}

fn shift(current: u8, target: u8, max_delta: u8) -> u8 {
    if target > current {
        current.saturating_add(max_delta.min(target - current))
    } else {
        current.saturating_sub(max_delta.min(current - target))
    }
}
```

Evolution is a **v2 feature** and can be deferred. The core value is in consuming the profile and injecting it into the system prompt.

---

## Three-Tier System

| Tier | File | When Injected | Content |
|------|------|---------------|---------|
| 1 | `USER.md` | Always (profile exists) | Name, communication prefs, context, goals |
| 2 | `PROFILE-DIRECTIVES.md` | confidence >= threshold | Behavioral directives, pain points, support suggestions |
| 3 | `profile.json` | Never injected | Raw data for tools, evolution, debugging |

---

## Estimated Effort

| Component | Lines | Complexity |
|-----------|-------|------------|
| `ProfileConfig` struct + defaults | ~40 | Low |
| `PsychographicProfile` types | ~150 | Low (serde structs) |
| `ProfileLoader` (load, validate, render) | ~200 | Medium |
| `ContextBuilder` integration | ~30 | Low |
| `AgentLoopConfig` wiring | ~10 | Low |
| Evolution (`shift()` + daemon hook) | ~80 | Medium (v2) |
| Tests | ~150 | Medium |
| **Total (v1, no evolution)** | **~430** | |
| **Total (v1 + v2 evolution)** | **~510** | |

---

## Edge Cases

1. **Existing USER.md**: Check for `<!-- profile-generated -->` sentinel. If absent, warn and don't overwrite.
2. **Invalid JSON**: `ProfileLoader::load()` returns `Err`, startup continues without profile (warn log).
3. **Missing file**: `ProfileLoader::load()` returns `Ok(None)`, no error.
4. **Version mismatch**: Only version 2 supported. Future versions bump this and add migration.
5. **Confidence regression**: If confidence drops below threshold on evolution, PROFILE-DIRECTIVES.md is removed.
6. **Tilde expansion**: Reuse `expand_tilde()` from `src/utils/transcription.rs` (move to a shared util).
7. **File size**: Profile JSON is typically <5KB. No size limit needed.
8. **Group chats**: USER.md is already excluded from group chats via existing `is_group` logic. PROFILE-DIRECTIVES.md should follow the same pattern (personal, not shared).

---

## Testing Strategy

1. **Unit tests** (inline, `src/agent/profile/mod.rs`):
   - `test_load_valid_profile` — round-trip serialize/deserialize
   - `test_load_missing_file` — returns `Ok(None)`
   - `test_load_invalid_json` — returns `Err`
   - `test_validate_version_mismatch` — rejects version != 2
   - `test_validate_confidence_out_of_range` — rejects > 1.0
   - `test_validate_personality_score_out_of_range` — rejects > 100
   - `test_render_user_md` — contains expected sections
   - `test_render_directives_md` — contains expected directives
   - `test_render_directives_skips_unknowns` — "unknown" fields omitted
   - `test_shift_capped` — evolution delta capping
   - `test_evolve_profile` — multi-dimension evolution

2. **Integration** (if needed):
   - Profile-derived USER.md appears in system prompt
   - Confidence gating prevents PROFILE-DIRECTIVES.md injection

---

## Config Documentation

Add to `docs/_pages/config.html`:

```
Profile
  agents.defaults.profile.enabled          false    Enable psychographic profile support
  agents.defaults.profile.profilePath      null     Path to profile JSON (default: {workspace}/profile.json)
  agents.defaults.profile.confidenceThreshold  0.6  Min confidence for Tier 2 directives
  agents.defaults.profile.evolutionEnabled false    Enable automatic profile evolution via daemon
```

---

## Dependencies

No new crate dependencies. Uses existing: `serde`, `serde_json`, `anyhow`, `tracing`, `chrono`.

---

## Generating the Profile

The profile JSON is generated externally using the Claude Project system prompt in `workspace/profile-onboarding-prompt.md`. Steps:

1. Create a new Claude Project at claude.ai
2. Paste the content of `workspace/profile-onboarding-prompt.md` into the Project Instructions
3. Start a conversation — Claude will run the onboarding interview
4. After all topics are covered (or you say "done"), Claude outputs the profile JSON
5. Save the JSON to `~/.oxicrab/workspace/profile.json` (or your configured `profilePath`)
6. Enable in config: `agents.defaults.profile.enabled: true`

No existing online tool performs this specific conversational-profiling-to-assistant-JSON workflow. Survey-based tools (16Personalities, Big Five) measure traits but don't output assistant configuration. API-based services (Crystal, Humantic AI) analyze LinkedIn/email but don't produce directive markdown. The Claude Project approach is purpose-built for this use case.
