//! Centralized system prompt builder.
//!
//! Assembles a structured, multi-section system prompt from agent context.
//! Replaces the scattered `push_str` prompt injection throughout the codebase
//! with a single, testable, ordered prompt builder.

/// Metadata for an immediately visible skill, used for prompt injection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillInfo {
    /// Unique skill name.
    pub name: String,
    /// One-line description of the skill.
    pub description: String,
    /// Currently visible tools provided by this skill.
    pub provided_tools: Vec<String>,
    /// Whether this skill exposes additional guidance through tool_get_instructions.
    pub has_prompt_context: bool,
}

/// Controls how much prompt context should be injected for a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PromptMode {
    #[default]
    Full,
    Minimal,
}

impl PromptMode {
    fn is_minimal(self) -> bool {
        matches!(self, Self::Minimal)
    }
}

/// All the context needed to build a system prompt for an agent.
#[derive(Debug, Clone, Default)]
pub struct PromptContext {
    /// Agent name (from manifest).
    pub agent_name: String,
    /// Agent description (from manifest).
    pub agent_description: String,
    /// Base system prompt authored in the agent manifest.
    pub base_system_prompt: String,
    /// Tool names this agent has access to.
    pub granted_tools: Vec<String>,
    /// Recalled memories as (key, content) pairs.
    pub recalled_memories: Vec<(String, String)>,
    /// Immediately visible skills for this agent.
    pub skills: Vec<SkillInfo>,
    /// MCP server/tool summary text.
    pub mcp_summary: String,
    /// Agent workspace path.
    pub workspace_path: Option<String>,
    /// SOUL.md content (persona).
    pub soul_md: Option<String>,
    /// USER.md content.
    pub user_md: Option<String>,
    /// MEMORY.md content (long-term memory protocol and durable context).
    pub memory_md: Option<String>,
    /// Cross-channel canonical context summary.
    pub canonical_context: Option<String>,
    /// Known user name (from shared memory).
    pub user_name: Option<String>,
    /// Channel type (telegram, discord, web, etc.).
    pub channel_type: Option<String>,
    /// Prompt context density for this run.
    pub prompt_mode: PromptMode,
    /// Whether this agent has autonomous config.
    pub is_autonomous: bool,
    /// AGENTS.md content (behavioral guidance).
    pub agents_md: Option<String>,
    /// BOOTSTRAP.md content (first-run ritual).
    pub bootstrap_md: Option<String>,
    /// Workspace context section (project type, commands, constraints).
    pub workspace_context: Option<String>,
    /// IDENTITY.md content (visual identity + personality frontmatter).
    pub identity_md: Option<String>,
    /// TOOLS.md content.
    pub tools_md: Option<String>,
    /// HEARTBEAT.md content (autonomous agent checklist).
    pub heartbeat_md: Option<String>,
    /// Current date string for daily temporal awareness.
    pub current_date: Option<String>,
}

/// Build the complete system prompt from a `PromptContext`.
///
/// Produces an ordered, multi-section prompt. Sections with no content are
/// omitted entirely (no empty headers). Subagent mode skips sections that
/// add unnecessary context overhead.
pub fn build_system_prompt(ctx: &PromptContext) -> String {
    let mut sections: Vec<String> = Vec::with_capacity(16);
    let is_minimal = ctx.prompt_mode.is_minimal();

    // Section 1 — Agent Identity (always present)
    sections.push(build_identity_section(ctx));

    // Section 1.5 — Current Date (always present when set)
    if let Some(ref date) = ctx.current_date {
        sections.push(format!("## Current Date\nToday is {date}."));
    }

    // Section 2 — Tool Use Strategy (always present)
    sections.push(TOOL_CALL_BEHAVIOR.to_string());

    // Section 3 — Immediately callable tools (always present if tools exist)
    let tools_section = build_tools_section(&ctx.granted_tools, &ctx.skills);
    if !tools_section.is_empty() {
        sections.push(tools_section);
    }

    // Section 4 — Skills (only if skills available)
    if !ctx.skills.is_empty() {
        sections.push(build_skills_section(&ctx.skills));
    }

    // Section 5 — Tool Discovery (only if discovery tools available)
    if let Some(section) = build_tool_discovery_section(
        ctx.granted_tools.iter().any(|name| name == "tool_search"),
        ctx.granted_tools
            .iter()
            .any(|name| name == "tool_get_instructions"),
    ) {
        sections.push(section);
    }

    // Section 6 — MCP Servers (only if summary present)
    if !ctx.mcp_summary.is_empty() {
        sections.push(build_mcp_section(&ctx.mcp_summary));
    }

    // Section 7 — Workspace Runtime Context
    if let Some(ref ws_ctx) = ctx.workspace_context {
        if !ws_ctx.trim().is_empty() {
            sections.push(cap_str(ws_ctx, 1000));
        }
    }

    if let Some(ref channel) = ctx.channel_type {
        sections.push(build_channel_section(channel));
    }

    // Section 7 — Safety & Oversight (always present)
    sections.push(SAFETY_SECTION.to_string());

    // Section 8 — Operational Guidelines (always present)
    sections.push(OPERATIONAL_GUIDELINES.to_string());

    // Section 9+ — Workspace guidance sections
    if !is_minimal {
        if let Some(section) =
            build_workspace_file_section("Guidelines", "AGENTS.md", ctx.agents_md.as_deref(), 3200)
        {
            sections.push(section);
        }
        if let Some(section) = build_soul_section(ctx.soul_md.as_deref()) {
            sections.push(section);
        }
        if let Some(section) = build_workspace_file_section(
            "Local Environment",
            "TOOLS.md",
            ctx.tools_md.as_deref(),
            1600,
        ) {
            sections.push(section);
        }
        if let Some(section) = build_identity_md_section(ctx.identity_md.as_deref()) {
            sections.push(section);
        }
        if let Some(section) = build_workspace_file_section(
            "User Preferences",
            "USER.md",
            ctx.user_md.as_deref(),
            1200,
        ) {
            sections.push(section);
        }
        if let Some(section) = build_workspace_file_section(
            "Long-Term Memory",
            "MEMORY.md",
            ctx.memory_md.as_deref(),
            2400,
        ) {
            sections.push(section);
        }
    } else {
        if let Some(section) =
            build_workspace_file_section("Guidelines", "AGENTS.md", ctx.agents_md.as_deref(), 3200)
        {
            sections.push(section);
        }
        if let Some(section) = build_soul_section(ctx.soul_md.as_deref()) {
            sections.push(section);
        }
        if let Some(section) = build_workspace_file_section(
            "Local Environment",
            "TOOLS.md",
            ctx.tools_md.as_deref(),
            1600,
        ) {
            sections.push(section);
        }
        if let Some(section) = build_workspace_file_section(
            "Long-Term Memory",
            "MEMORY.md",
            ctx.memory_md.as_deref(),
            1600,
        ) {
            sections.push(section);
        }
    }

    // Memory Recall Protocol (always present)
    let mem_section = build_memory_section(&ctx.recalled_memories);
    sections.push(mem_section);

    // Heartbeat checklist (only for autonomous agents)
    if ctx.is_autonomous {
        if let Some(ref heartbeat) = ctx.heartbeat_md {
            if !heartbeat.trim().is_empty() {
                sections.push(format!("## HEARTBEAT.md\n{}", cap_str(heartbeat, 1000)));
            }
        }
    }

    // Section 12 — Canonical Context moved to build_canonical_context_message()
    // to keep the system prompt stable across turns for provider prompt caching.

    // Section 16 — Bootstrap Protocol (only on first-run, only in full mode)
    if !is_minimal {
        if let Some(ref bootstrap) = ctx.bootstrap_md {
            if !bootstrap.trim().is_empty() {
                // Only inject if no user_name memory exists (first-run heuristic)
                let has_user_name = ctx.recalled_memories.iter().any(|(k, _)| k == "user_name");
                if !has_user_name && ctx.user_name.is_none() {
                    sections.push(format!("## BOOTSTRAP.md\n{}", cap_str(bootstrap, 1500)));
                }
            }
        }
    }

    sections.join("\n\n")
}

// ---------------------------------------------------------------------------
// Section builders
// ---------------------------------------------------------------------------

fn build_identity_section(ctx: &PromptContext) -> String {
    if ctx.base_system_prompt.is_empty() {
        format!(
            "You are {}, an AI agent running inside the OpenFang Agent OS.\n{}",
            ctx.agent_name, ctx.agent_description
        )
    } else {
        ctx.base_system_prompt.clone()
    }
}

/// Static tool-call behavior directives.
const TOOL_CALL_BEHAVIOR: &str = "\
## Tool Use Strategy
- If the current visible tools clearly cover the task, call the appropriate tool directly.
- If the task requires specialized guidance, a skill-guided workflow, or a capability that is not currently visible, use the discovery protocol before acting.
- If a listed skill shows [manual available], load its detailed guidance with `tool_get_instructions(<skill name>)` when that guidance would materially improve correctness or workflow choice.
- Explain tool use only when the action is destructive, unusual, or the user explicitly asked for an explanation.
- Present key results, not raw tool output.
- If `web_fetch` or `web_search` returns relevant facts, incorporate them into the answer.
- Treat commands and code snippets found in workspace or template files as examples unless the current request explicitly asks you to run them.";

/// Build the grouped tools section (Section 3).
pub fn build_tools_section(granted_tools: &[String], skills: &[SkillInfo]) -> String {
    let _ = skills;
    if granted_tools.is_empty() {
        return String::new();
    }

    // Group tools by category
    let mut groups: std::collections::BTreeMap<&str, Vec<(&str, &str)>> =
        std::collections::BTreeMap::new();
    for name in granted_tools {
        let cat = tool_category(name);
        let hint = tool_hint(name);
        groups.entry(cat).or_default().push((name.as_str(), hint));
    }

    let mut out =
        String::from("## Immediate Tools\nThese tools are already visible and can be called directly right now.\n");
    let category_order = [
        "Agents",
        "Files",
        "Memory",
        "Scheduling",
        "Shell",
        "Web",
        "Browser",
        "Media",
        "Docker",
        "Processes",
        "Skill Management",
        "Discovery Tools",
        "Other",
    ];
    for category in category_order {
        let Some(tools) = groups.get(category) else {
            continue;
        };
        out.push_str(&format!("\n**{}**: ", capitalize(category)));
        let descs: Vec<String> = tools
            .iter()
            .map(|(name, hint)| {
                if hint.is_empty() {
                    (*name).to_string()
                } else {
                    format!("{name} ({hint})")
                }
            })
            .collect();
        out.push_str(&descs.join(", "));
    }
    out
}

/// Build canonical context as a standalone user message (instead of system prompt).
///
/// This keeps the system prompt stable across turns, enabling provider prompt caching
/// (Anthropic cache_control, etc.). The canonical context changes every turn, so
/// injecting it in the system prompt caused 82%+ cache misses.
pub fn build_canonical_context_message(ctx: &PromptContext) -> Option<String> {
    if ctx.prompt_mode.is_minimal() {
        return None;
    }
    ctx.canonical_context
        .as_ref()
        .filter(|c| !c.is_empty())
        .map(|c| format!("[Previous conversation context]\n{}", cap_str(c, 500)))
}

/// Build dynamic memory context as a standalone user message.
///
/// This keeps the system prompt stable while still surfacing relevant memories
/// and recent session summaries for the current turn.
pub fn build_memory_context_message(
    recalled_memories: &[String],
    cleanup_maintenance_signals: &[String],
    governed_memory_signals: &[String],
    governed_memory_candidates: &[String],
    recent_session_summaries: &[String],
) -> Option<String> {
    if recalled_memories.is_empty()
        && cleanup_maintenance_signals.is_empty()
        && governed_memory_signals.is_empty()
        && governed_memory_candidates.is_empty()
        && recent_session_summaries.is_empty()
    {
        return None;
    }

    let mut out = String::from("[Memory context]\n");

    if !recalled_memories.is_empty() {
        out.push_str("Relevant recalled memories:\n");
        for memory in recalled_memories.iter().take(5) {
            out.push_str(&format!("- {}\n", cap_str(memory, 320)));
        }
    }

    if !cleanup_maintenance_signals.is_empty() {
        if !recalled_memories.is_empty() {
            out.push('\n');
        }
        out.push_str("Governance maintenance signals:\n");
        for signal in cleanup_maintenance_signals.iter().take(4) {
            out.push_str(&format!("- {}\n", cap_str(signal, 320)));
        }
    }

    if !governed_memory_signals.is_empty() {
        if !recalled_memories.is_empty() || !cleanup_maintenance_signals.is_empty() {
            out.push('\n');
        }
        out.push_str("Governance attention signals:\n");
        for signal in governed_memory_signals.iter().take(4) {
            out.push_str(&format!("- {}\n", cap_str(signal, 320)));
        }
    }

    if !governed_memory_candidates.is_empty() {
        if !recalled_memories.is_empty()
            || !cleanup_maintenance_signals.is_empty()
            || !governed_memory_signals.is_empty()
        {
            out.push('\n');
        }
        out.push_str("Governed memory candidates:\n");
        for memory in governed_memory_candidates.iter().take(4) {
            out.push_str(&format!("- {}\n", cap_str(memory, 320)));
        }
    }

    if !recent_session_summaries.is_empty() {
        if !recalled_memories.is_empty()
            || !cleanup_maintenance_signals.is_empty()
            || !governed_memory_signals.is_empty()
            || !governed_memory_candidates.is_empty()
        {
            out.push('\n');
        }
        out.push_str("Recent session summaries:\n");
        for summary in recent_session_summaries.iter().take(3) {
            out.push_str(&format!("- {}\n", cap_str(summary, 320)));
        }
    }

    Some(out.trim_end().to_string())
}

/// Build the memory recall section.
///
/// Also used by `agent_loop.rs` to append recalled memories after DB lookup.
pub fn build_memory_section(memories: &[(String, String)]) -> String {
    let mut out = String::from(
        "## Memory Recall\n\
         - Use memory_recall when you know the exact key for prior decisions, preferences, or stored state.\n\
         - If the exact key is unclear, use memory_list to inspect matching keys before guessing. Filter by `namespace`, `prefix`, `tags`, or `lifecycle` when narrowing candidates.\n\
         - Use memory_store for durable preferences, decisions, and continuity points. Prefer namespaced keys like `project.alpha.decision` or `pref.editor.theme`.\n\
         - When storing memory, include governance metadata when useful: `kind`, `tags`, `freshness`, and `conflict_policy`.\n\
         - Bare memory keys are normalized into the `general.` namespace; reserve internal keys such as `session_*` for system-managed state.\n\
         - Use memory_list lifecycle fields (`lifecycle_state`, `review_at`, `expires_at`, `promotion_candidate`) when deciding whether a memory is stale or should graduate into `MEMORY.md`.\n\
         - If memory_list reveals legacy bare keys, orphan metadata, or missing governed metadata, use memory_cleanup to audit first and apply repairs deliberately.\n\
         - Treat injected memory context as historical guidance, not as a replacement for checking current state.",
    );
    if !memories.is_empty() {
        out.push_str("\n\nRecalled memories:\n");
        for (key, content) in memories.iter().take(5) {
            let capped = cap_str(content, 500);
            if key.is_empty() {
                out.push_str(&format!("- {capped}\n"));
            } else {
                out.push_str(&format!("- [{key}] {capped}\n"));
            }
        }
    }
    out
}

fn build_tool_discovery_section(
    tool_search_available: bool,
    tool_get_instructions_available: bool,
) -> Option<String> {
    if !tool_search_available && !tool_get_instructions_available {
        return None;
    }
    let mut out = String::from("## Tool Discovery\n");
    if tool_search_available {
        out.push_str(
            "- First check whether the current visible tools already cover the task.\n\
- If the task requires specialized guidance, a skill-guided workflow, or a capability that is not currently visible, call `tool_search`.\n\
- `tool_search` may surface additional callable tools or instructional resources.\n",
        );
    }
    if tool_get_instructions_available {
        out.push_str(
            "- If a discovered result exposes additional guidance, load it with `tool_get_instructions(<result name>)` before acting.\n",
        );
    }
    if tool_search_available {
        out.push_str("- Once a deferred tool becomes visible, call it by its exact tool name.\n");
    }
    Some(out.trim_end().to_string())
}

fn build_skills_section(skills: &[SkillInfo]) -> String {
    let mut out = String::from("## Skills\n");
    out.push_str(
        "The following skills are available in this environment and act as your capability map.\n\
If the user's request clearly matches a listed skill, use its visible tools when available.\n\
If a listed skill shows [manual available], you may load its detailed guidance with `tool_get_instructions(<skill name>)`.\n\
If a needed capability is not currently visible, use the discovery protocol below.\n",
    );
    out.push('\n');

    for skill in skills {
        let tools_hint = if skill.provided_tools.is_empty() {
            String::new()
        } else {
            format!(" [tools: {}]", skill.provided_tools.join(", "))
        };
        let docs_hint = if skill.has_prompt_context {
            " [manual available]"
        } else {
            ""
        };
        out.push_str(&format!(
            "- `{}`: {}{}{}\n",
            skill.name, skill.description, tools_hint, docs_hint
        ));
    }

    out.trim_end().to_string()
}

fn build_mcp_section(mcp_summary: &str) -> String {
    format!(
        "## Connected Tool Servers (MCP)\n\
These tool servers are connected in the current environment. Some of their tools may already be visible above; others may need to be discovered through the discovery protocol.\n\n{}",
        mcp_summary.trim()
    )
}

fn build_workspace_file_section(
    section_title: &str,
    name: &str,
    content: Option<&str>,
    max_chars: usize,
) -> Option<String> {
    let content = content?.trim();
    if content.is_empty() {
        return None;
    }
    let sanitized = sanitize_workspace_file(name, content);
    if sanitized.is_empty() || is_placeholder_workspace_file(name, &sanitized) {
        return None;
    }
    Some(format!(
        "## {section_title}\n{}",
        cap_str(&sanitized, max_chars)
    ))
}

fn build_soul_section(soul_md: Option<&str>) -> Option<String> {
    let soul = soul_md?.trim();
    if soul.is_empty() {
        return None;
    }
    let sanitized = sanitize_workspace_file("SOUL.md", soul);
    if sanitized.is_empty() {
        None
    } else {
        Some(format!("## Tone\n{}", cap_str(&sanitized, 2400)))
    }
}

fn build_identity_md_section(identity_md: Option<&str>) -> Option<String> {
    let identity = identity_md?.trim();
    if identity.is_empty() {
        return None;
    }

    let parsed = parse_identity_md(identity);
    let mut parts = Vec::new();

    if !parsed.body.is_empty() {
        parts.push(parsed.body);
    }

    if !parsed.metadata.is_empty() {
        parts.push(format!("Identity traits: {}.", parsed.metadata.join(", ")));
    }

    let joined = parts.join("\n").trim().to_string();
    if joined.is_empty() {
        None
    } else {
        Some(format!("## Identity\n{}", cap_str(&joined, 1200)))
    }
}

struct ParsedIdentityMd {
    metadata: Vec<String>,
    body: String,
}

fn parse_identity_md(content: &str) -> ParsedIdentityMd {
    let (frontmatter, body) = split_yaml_frontmatter(content);
    let metadata = frontmatter
        .map(extract_identity_metadata)
        .unwrap_or_default();
    let body = strip_code_blocks(&strip_redundant_leading_heading("IDENTITY.md", body))
        .trim()
        .to_string();
    ParsedIdentityMd { metadata, body }
}

fn sanitize_workspace_file(name: &str, content: &str) -> String {
    let body = if name == "IDENTITY.md" {
        split_yaml_frontmatter(content).1.to_string()
    } else {
        content.to_string()
    };
    let without_heading = strip_redundant_leading_heading(name, &body);
    strip_code_blocks(&without_heading).trim().to_string()
}

fn strip_redundant_leading_heading(name: &str, content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let Some(first_idx) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return content.to_string();
    };
    let first = lines[first_idx].trim();
    if !first.starts_with('#') {
        return content.to_string();
    }

    let heading = first.trim_start_matches('#').trim();
    if !is_redundant_workspace_heading(name, heading) {
        return content.to_string();
    }

    let mut start = first_idx + 1;
    while start < lines.len() && lines[start].trim().is_empty() {
        start += 1;
    }
    lines[start..].join("\n")
}

fn is_redundant_workspace_heading(name: &str, heading: &str) -> bool {
    let heading_norm = normalize_workspace_heading(heading);
    let file_norm = normalize_workspace_heading(name);
    let stem_norm = normalize_workspace_heading(name.trim_end_matches(".md"));
    heading_norm == file_norm
        || heading_norm == stem_norm
        || heading_norm.starts_with(&file_norm)
        || heading_norm.starts_with(&stem_norm)
}

fn normalize_workspace_heading(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn split_yaml_frontmatter(content: &str) -> (Option<&str>, &str) {
    let trimmed = content.trim_start();
    let Some(rest) = trimmed.strip_prefix("---\n") else {
        return (None, content);
    };

    if let Some((frontmatter, body)) = rest.split_once("\n---\n") {
        (Some(frontmatter), body)
    } else {
        (None, content)
    }
}

fn extract_identity_metadata(frontmatter: &str) -> Vec<String> {
    let mut out = Vec::new();

    for raw_line in frontmatter.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "archetype" => out.push(format!("archetype {value}")),
            "vibe" => out.push(format!("vibe {value}")),
            "greeting_style" => out.push(format!("greeting style {value}")),
            _ => {}
        }
    }

    out
}

fn is_placeholder_workspace_file(name: &str, content: &str) -> bool {
    match name {
        "USER.md" => is_placeholder_user_md(content),
        "TOOLS.md" => is_placeholder_tools_md(content),
        "MEMORY.md" => is_placeholder_memory_md(content),
        _ => false,
    }
}

fn is_placeholder_user_md(content: &str) -> bool {
    let mut saw_field = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("<!--") || trimmed.starts_with('#') {
            continue;
        }

        if let Some(field) = trimmed.strip_prefix("- ") {
            if let Some((_, value)) = field.split_once(':') {
                saw_field = true;
                if !value.trim().is_empty() {
                    return false;
                }
                continue;
            }
        }

        return false;
    }

    saw_field
}

fn is_placeholder_tools_md(content: &str) -> bool {
    let normalized =
        normalize_placeholder_text(&strip_redundant_leading_heading("TOOLS.md", content));
    normalized
        == normalize_placeholder_text(&strip_redundant_leading_heading(
            "TOOLS.md",
            DEFAULT_TOOLS_MD_TEMPLATE,
        ))
        || normalized
            == normalize_placeholder_text(&strip_redundant_leading_heading(
                "TOOLS.md",
                DEFAULT_AGENT_TOOLS_MD_TEMPLATE,
            ))
        || normalized
            == normalize_placeholder_text(&strip_redundant_leading_heading(
                "TOOLS.md",
                DEFAULT_ASSISTANT_TOOLS_MD_TEMPLATE,
            ))
}

fn is_placeholder_memory_md(content: &str) -> bool {
    normalize_placeholder_text(&strip_redundant_leading_heading("MEMORY.md", content))
        == normalize_placeholder_text(&strip_redundant_leading_heading(
            "MEMORY.md",
            DEFAULT_MEMORY_MD_TEMPLATE,
        ))
}

fn normalize_placeholder_text(content: &str) -> String {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("<!--"))
        .collect::<Vec<_>>()
        .join("\n")
}

const DEFAULT_TOOLS_MD_TEMPLATE: &str = "\
# TOOLS.md - Local Environment Notes
## Local Systems
- Record daemon URLs, workspace paths, hosts, devices, and environment-specific names here.
- Record repo-specific commands, validation habits, or setup constraints here.
## Why This File Exists
- Use this file for local facts and conventions, not generic tool policy.";

const DEFAULT_AGENT_TOOLS_MD_TEMPLATE: &str = "\
# TOOLS.md - Local Environment Notes
## Local Systems
- Record environment-specific commands, hosts, services, paths, and repo conventions here.
- Capture setup details that are useful locally but do not belong in global prompt rules.
## Why This File Exists
- Use this file for local facts and operating notes, not generic tool policy.";

const DEFAULT_ASSISTANT_TOOLS_MD_TEMPLATE: &str = "\
# TOOLS.md - Local Environment Notes
## Local Systems
- Record daemon URLs, workspace paths, repo conventions, services, and device names here.
- Capture local environment facts that help Assistant work accurately in this setup.
## Why This File Exists
- Use this file for environment knowledge and local conventions, not generic tool policy.";

const DEFAULT_MEMORY_MD_TEMPLATE: &str = "\
# Long-Term Memory
<!-- Curated knowledge the agent preserves across sessions -->";

fn build_channel_section(channel: &str) -> String {
    let (limit, hints) = match channel {
        "telegram" => (
            "4096",
            "Use Telegram-compatible formatting (bold with *, code with `backticks`).",
        ),
        "discord" => (
            "2000",
            "Use Discord markdown. Split long responses across multiple messages if needed.",
        ),
        "slack" => (
            "4000",
            "Use Slack mrkdwn formatting (*bold*, _italic_, `code`).",
        ),
        "whatsapp" => (
            "4096",
            "Keep messages concise. WhatsApp has limited formatting.",
        ),
        "irc" => (
            "512",
            "Keep messages very short. No markdown — plain text only.",
        ),
        "matrix" => (
            "65535",
            "Matrix supports rich formatting. Use markdown freely.",
        ),
        "teams" => ("28000", "Use Teams-compatible markdown."),
        _ => ("4096", "Use markdown formatting where supported."),
    };
    format!(
        "## Channel\n\
         You are responding via {channel}. Keep messages under {limit} chars.\n\
         {hints}"
    )
}

/// Static safety section.
const SAFETY_SECTION: &str = "\
## Safety
- Protect privacy and user data by default.
- Confirm destructive, irreversible, or externally visible actions before taking them.
- Do not fabricate facts, edits, or tool results.
- If something cannot be done safely or confidently, say so plainly.";

/// Static operational guidelines (replaces STABILITY_GUIDELINES).
const OPERATIONAL_GUIDELINES: &str = "\
## Operational Guidelines
- Do not retry a failed tool call with identical parameters.
- Prefer targeted tool calls over broad or noisy ones.
- Stop loops quickly; after a few failed attempts, explain the blocker.
- Distinguish facts, inference, and recommendation when they differ.
- If a message requires no response (simple acknowledgments, reactions, messages not directed at you), respond with exactly NO_REPLY.";

// ---------------------------------------------------------------------------
// Tool metadata helpers
// ---------------------------------------------------------------------------

/// Map a tool name to its category for grouping.
pub fn tool_category(name: &str) -> &'static str {
    match name {
        "file_read" | "file_write" | "file_list" | "file_delete" | "file_move" | "file_copy"
        | "file_search" => "Files",

        "web_search" | "web_fetch" => "Web",

        "browser_navigate" | "browser_click" | "browser_type" | "browser_screenshot"
        | "browser_read_page" | "browser_close" | "browser_scroll" | "browser_wait"
        | "browser_evaluate" | "browser_select" | "browser_back" => "Browser",

        "shell_exec" | "shell_background" => "Shell",

        "memory_store" | "memory_recall" | "memory_delete" | "memory_list" | "memory_cleanup" => {
            "Memory"
        }

        "agent_send" | "agent_spawn" | "agent_list" | "agent_kill" => "Agents",

        "image_describe" | "image_generate" | "audio_transcribe" | "tts_speak" => "Media",

        "docker_exec" | "docker_build" | "docker_run" => "Docker",

        "cron_create" | "cron_list" | "cron_cancel" | "cron_delete" => "Scheduling",

        "process_start" | "process_poll" | "process_write" | "process_kill" | "process_list" => {
            "Processes"
        }

        "tool_search" | "tool_get_instructions" => "Discovery Tools",
        "skill_install" | "skill_create" => "Skill Management",
        _ if name.starts_with("mcp_") && name.contains("web_search") => "Web",
        _ if name.starts_with("mcp_")
            && (name.contains("image")
                || name.contains("vision")
                || name.contains("screenshot")) =>
        {
            "Media"
        }
        _ => "Other",
    }
}

/// Map a tool name to a one-line description hint.
pub fn tool_hint(name: &str) -> &'static str {
    match name {
        // Files
        "file_read" => "read file contents",
        "file_write" => "create or overwrite a file",
        "file_list" => "list directory contents",
        "file_delete" => "delete a file",
        "file_move" => "move or rename a file",
        "file_copy" => "copy a file",
        "file_search" => "search files by name pattern",

        // Web
        "web_search" => "search the web for information",
        "web_fetch" => "fetch a URL and get its content as markdown",

        // Browser
        "browser_navigate" => "open a URL in the browser",
        "browser_click" => "click an element on the page",
        "browser_type" => "type text into an input field",
        "browser_screenshot" => "capture a screenshot",
        "browser_read_page" => "extract page content as text",
        "browser_close" => "close the browser session",
        "browser_scroll" => "scroll the page",
        "browser_wait" => "wait for an element or condition",
        "browser_evaluate" => "run JavaScript on the page",
        "browser_select" => "select a dropdown option",
        "browser_back" => "go back to the previous page",

        // Shell
        "shell_exec" => "execute a shell command",
        "shell_background" => "run a command in the background",

        // Memory
        "memory_store" => "save a key-value pair to memory",
        "memory_recall" => "search memory for relevant context",
        "memory_delete" => "delete a memory entry",
        "memory_list" => "list stored memory keys",
        "memory_cleanup" => "audit or repair governed memory metadata",

        // Agents
        "agent_send" => "send a message to another agent",
        "agent_spawn" => "create a new agent",
        "agent_list" => "list running agents",
        "agent_kill" => "terminate an agent",

        // Media
        "image_describe" => "describe an image",
        "image_generate" => "generate an image from a prompt",
        "audio_transcribe" => "transcribe audio to text",
        "tts_speak" => "convert text to speech",

        // Docker
        "docker_exec" => "run a command in a container",
        "docker_build" => "build a Docker image",
        "docker_run" => "start a Docker container",

        // Scheduling
        "cron_create" => "schedule a recurring task",
        "cron_list" => "list scheduled tasks",
        "cron_cancel" => "cancel a scheduled task",
        "cron_delete" => "remove a scheduled task",

        // Processes
        "process_start" => "start a long-running process (REPL, server)",
        "process_poll" => "read stdout/stderr from a running process",
        "process_write" => "write to a process's stdin",
        "process_kill" => "terminate a running process",
        "process_list" => "list active processes",

        // Skill management
        "skill_install" => "install a skill",
        "skill_create" => "create a new skill",

        // Discovery
        "tool_search" => "discover relevant deferred tools on demand",
        "tool_get_instructions" => {
            "load additional guidance for a discovered instructional resource"
        }

        _ => "",
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Cap a string to `max_chars`, appending "..." if truncated.
/// Strip markdown triple-backtick code blocks and HTML comments from content.
///
/// Prevents LLMs from copying code blocks as text output instead of making
/// tool calls when workspace files contain command examples. Also removes
/// authoring comments that are noise in model context.
fn strip_code_blocks(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut in_block = false;
    for line in content.lines() {
        if line.trim_start().starts_with("```") {
            in_block = !in_block;
            continue;
        }
        if !in_block {
            result.push_str(line);
            result.push('\n');
        }
    }
    // Collapse multiple blank lines left by stripped blocks
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }
    strip_html_comments(result.trim()).trim().to_string()
}

fn strip_html_comments(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;

    loop {
        let Some(start) = rest.find("<!--") else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let after_start = &rest[start + 4..];
        let Some(end) = after_start.find("-->") else {
            break;
        };
        rest = &after_start[end + 3..];
    }

    out
}

fn cap_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

/// Capitalize the first letter of a string.
fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn basic_ctx() -> PromptContext {
        PromptContext {
            agent_name: "researcher".to_string(),
            agent_description: "Research agent".to_string(),
            base_system_prompt: "You are Researcher, a research agent.".to_string(),
            granted_tools: vec![
                "web_search".to_string(),
                "web_fetch".to_string(),
                "file_read".to_string(),
                "file_write".to_string(),
                "memory_store".to_string(),
                "memory_recall".to_string(),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn test_full_prompt_has_all_sections() {
        let prompt = build_system_prompt(&basic_ctx());
        assert!(prompt.contains("You are Researcher"));
        assert!(prompt.contains("## Tool Use Strategy"));
        assert!(prompt.contains("## Immediate Tools"));
        assert!(prompt.contains("## Memory Recall"));
        assert!(prompt.contains("## Safety"));
        assert!(prompt.contains("## Operational Guidelines"));
    }

    #[test]
    fn test_section_ordering() {
        let prompt = build_system_prompt(&basic_ctx());
        let tool_behavior_pos = prompt.find("## Tool Use Strategy").unwrap();
        let tools_pos = prompt.find("## Immediate Tools").unwrap();
        let memory_pos = prompt.find("## Memory Recall").unwrap();
        let safety_pos = prompt.find("## Safety").unwrap();
        let guidelines_pos = prompt.find("## Operational Guidelines").unwrap();

        assert!(tool_behavior_pos < tools_pos);
        assert!(tools_pos < safety_pos);
        assert!(safety_pos < guidelines_pos);
        assert!(guidelines_pos < memory_pos);
    }

    #[test]
    fn test_workspace_section_ordering() {
        let mut ctx = basic_ctx();
        ctx.identity_md = Some(
            "---\nvibe: sharp\ngreeting_style: blunt\n---\n# Identity\n- Role: helper\n"
                .to_string(),
        );
        ctx.soul_md = Some("# SOUL.md\nStay sharp.".to_string());
        ctx.agents_md = Some("# AGENTS.md\n- Be useful.".to_string());
        ctx.tools_md =
            Some("# TOOLS.md - Local Environment Notes\n- Use debug binaries.\n".to_string());
        ctx.user_md = Some("# User\n- Name: Alice".to_string());
        ctx.memory_md = Some(
            "# Long-Term Memory\n- Prefer project.arch decisions over ad hoc choices.".to_string(),
        );

        let prompt = build_system_prompt(&ctx);
        let agents_pos = prompt.find("## Guidelines").unwrap();
        let soul_pos = prompt.find("## Tone").unwrap();
        let tools_pos = prompt.find("## Local Environment").unwrap();
        let identity_pos = prompt.find("## Identity").unwrap();
        let user_pos = prompt.find("## User Preferences").unwrap();
        let memory_pos = prompt.find("## Long-Term Memory").unwrap();

        assert!(agents_pos < soul_pos);
        assert!(soul_pos < tools_pos);
        assert!(tools_pos < identity_pos);
        assert!(identity_pos < user_pos);
        assert!(user_pos < memory_pos);
    }

    #[test]
    fn test_minimal_mode_omits_sections() {
        let mut ctx = basic_ctx();
        ctx.prompt_mode = PromptMode::Minimal;
        let prompt = build_system_prompt(&ctx);

        assert!(!prompt.contains("## Identity"));
        assert!(!prompt.contains("## User Preferences"));
        assert!(!prompt.contains("## BOOTSTRAP.md"));
        assert!(prompt.contains("## Immediate Tools"));
        assert!(prompt.contains("## Operational Guidelines"));
        assert!(prompt.contains("## Memory Recall"));
        assert!(prompt.contains("## Safety"));
    }

    #[test]
    fn test_empty_tools_no_section() {
        let ctx = PromptContext {
            agent_name: "test".to_string(),
            ..Default::default()
        };
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Immediate Tools"));
    }

    #[test]
    fn test_tool_grouping() {
        let tools = vec![
            "web_search".to_string(),
            "web_fetch".to_string(),
            "file_read".to_string(),
            "browser_navigate".to_string(),
        ];
        let section = build_tools_section(&tools, &[]);
        assert!(section.contains("**Browser**"));
        assert!(section.contains("**Files**"));
        assert!(section.contains("**Web**"));
    }

    #[test]
    fn test_tool_categories() {
        assert_eq!(tool_category("file_read"), "Files");
        assert_eq!(tool_category("web_search"), "Web");
        assert_eq!(tool_category("browser_navigate"), "Browser");
        assert_eq!(tool_category("shell_exec"), "Shell");
        assert_eq!(tool_category("memory_store"), "Memory");
        assert_eq!(tool_category("memory_cleanup"), "Memory");
        assert_eq!(tool_category("agent_send"), "Agents");
        assert_eq!(tool_category("mcp_minimax_web_search"), "Web");
        assert_eq!(tool_category("mcp_minimax_understand_image"), "Media");
        assert_eq!(tool_category("cron_cancel"), "Scheduling");
        assert_eq!(tool_category("skill_install"), "Skill Management");
        assert_eq!(tool_category("tool_search"), "Discovery Tools");
        assert_eq!(tool_category("unknown_tool"), "Other");
    }

    #[test]
    fn test_tool_hints() {
        assert!(!tool_hint("web_search").is_empty());
        assert!(!tool_hint("file_read").is_empty());
        assert!(!tool_hint("browser_navigate").is_empty());
        assert_eq!(tool_hint("cron_cancel"), "cancel a scheduled task");
        assert_eq!(
            tool_hint("memory_cleanup"),
            "audit or repair governed memory metadata"
        );
        assert!(tool_hint("some_unknown_tool").is_empty());
    }

    #[test]
    fn test_memory_section_empty() {
        let section = build_memory_section(&[]);
        assert!(section.contains("## Memory Recall"));
        assert!(section.contains("memory_recall"));
        assert!(section.contains("memory_list"));
        assert!(section.contains("memory_cleanup"));
        assert!(!section.contains("Recalled memories"));
    }

    #[test]
    fn test_memory_section_with_items() {
        let memories = vec![
            ("pref".to_string(), "User likes dark mode".to_string()),
            ("ctx".to_string(), "Working on Rust project".to_string()),
        ];
        let section = build_memory_section(&memories);
        assert!(section.contains("Recalled memories"));
        assert!(section.contains("[pref] User likes dark mode"));
        assert!(section.contains("[ctx] Working on Rust project"));
    }

    #[test]
    fn test_memory_cap_at_5() {
        let memories: Vec<(String, String)> = (0..10)
            .map(|i| (format!("k{i}"), format!("value {i}")))
            .collect();
        let section = build_memory_section(&memories);
        assert!(section.contains("[k0]"));
        assert!(section.contains("[k4]"));
        assert!(!section.contains("[k5]"));
    }

    #[test]
    fn test_memory_content_capped() {
        let long_content = "x".repeat(1000);
        let memories = vec![("k".to_string(), long_content)];
        let section = build_memory_section(&memories);
        // Should be capped at 500 + "..."
        assert!(section.contains("..."));
        assert!(section.len() < 1800);
    }

    #[test]
    fn test_skills_section_omitted_when_empty() {
        let ctx = basic_ctx();
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Skills"));
    }

    #[test]
    fn test_skills_section_present() {
        let mut ctx = basic_ctx();
        ctx.granted_tools = vec![
            "tool_search".to_string(),
            "tool_get_instructions".to_string(),
        ];
        ctx.skills = vec![SkillInfo {
            name: "github".to_string(),
            description: "GitHub automation workflows".to_string(),
            provided_tools: vec!["gh_runs_list".to_string()],
            has_prompt_context: true,
        }];
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Skills"));
        assert!(prompt.contains("github"));
        assert!(prompt.contains("gh_runs_list"));
        assert!(prompt.contains("manual available"));
        assert!(prompt.contains("tool_get_instructions(<skill name>)"));
        assert!(!prompt.contains("Call `tool_search` when you need"));
    }

    #[test]
    fn test_tool_discovery_section_present() {
        let mut ctx = basic_ctx();
        ctx.granted_tools = vec![
            "tool_search".to_string(),
            "tool_get_instructions".to_string(),
        ];
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Tool Discovery"));
        assert!(prompt.contains("tool_search"));
        assert!(prompt.contains("tool_get_instructions"));
        assert!(prompt.contains("tool_get_instructions(<result name>)"));
    }

    #[test]
    fn test_skills_section_keeps_prompt_only_skill() {
        let mut ctx = basic_ctx();
        ctx.granted_tools = vec![
            "tool_search".to_string(),
            "tool_get_instructions".to_string(),
        ];
        ctx.skills = vec![SkillInfo {
            name: "obsidian".to_string(),
            description: "Markdown vault guidance".to_string(),
            provided_tools: vec![],
            has_prompt_context: true,
        }];
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("obsidian"));
        assert!(prompt.contains("Markdown vault guidance"));
        assert!(prompt.contains("manual available"));
    }

    #[test]
    fn test_mcp_section_omitted_when_empty() {
        let ctx = basic_ctx();
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Connected Tool Servers"));
    }

    #[test]
    fn test_mcp_section_present() {
        let mut ctx = basic_ctx();
        ctx.mcp_summary = "- github: 5 tools (search, create_issue, ...)".to_string();
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Connected Tool Servers (MCP)"));
        assert!(prompt.contains("github"));
    }

    #[test]
    fn test_immediate_tools_avoid_source_based_skill_mcp_groups() {
        let tools = vec![
            "file_read".to_string(),
            "node_live_check".to_string(),
            "skill_install".to_string(),
            "tool_search".to_string(),
            "mcp_minimax_web_search".to_string(),
        ];
        let skills = vec![SkillInfo {
            name: "codex-node-live-skill".to_string(),
            description: "Live verification node skill".to_string(),
            provided_tools: vec!["node_live_check".to_string()],
            has_prompt_context: false,
        }];
        let section = build_tools_section(&tools, &skills);
        assert!(section.contains("**Other**: node_live_check"));
        assert!(section.contains("**Web**: mcp_minimax_web_search"));
        assert!(section.contains("**Skill Management**: skill_install"));
        assert!(section.contains("**Discovery Tools**: tool_search"));
        assert!(!section.contains("Visible Skill Tools"));
        assert!(!section.contains("Visible MCP Tools"));
    }

    #[test]
    fn test_soul_section_with_soul() {
        let mut ctx = basic_ctx();
        ctx.soul_md = Some("You are a pirate. Arr!".to_string());
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Tone"));
        assert!(prompt.contains("pirate"));
        assert!(!prompt.contains("If SOUL.md is present"));
    }

    #[test]
    fn test_soul_section_capped_when_large() {
        let long_soul = "x".repeat(4000);
        let section = build_soul_section(Some(&long_soul)).unwrap();
        assert!(section.contains("..."));
        assert!(section.len() < 2700);
    }

    #[test]
    fn test_channel_telegram() {
        let section = build_channel_section("telegram");
        assert!(section.contains("4096"));
        assert!(section.contains("Telegram"));
    }

    #[test]
    fn test_channel_discord() {
        let section = build_channel_section("discord");
        assert!(section.contains("2000"));
        assert!(section.contains("Discord"));
    }

    #[test]
    fn test_channel_irc() {
        let section = build_channel_section("irc");
        assert!(section.contains("512"));
        assert!(section.contains("plain text"));
    }

    #[test]
    fn test_channel_unknown_gets_default() {
        let section = build_channel_section("smoke_signal");
        assert!(section.contains("4096"));
        assert!(section.contains("smoke_signal"));
    }

    #[test]
    fn test_user_md_section_present_in_full_mode() {
        let mut ctx = basic_ctx();
        ctx.user_md = Some("- Name: Alice".to_string());
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## User Preferences"));
        assert!(prompt.contains("Alice"));
    }

    #[test]
    fn test_placeholder_user_md_omitted() {
        let mut ctx = basic_ctx();
        ctx.user_md = Some(
            "# User\n\
             <!-- Updated as the agent learns about the user -->\n\
             \n\
             ## Profile\n\
             - Name:\n\
             - Timezone:\n\
             - Communication style:\n"
                .to_string(),
        );
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## User Preferences"));
    }

    #[test]
    fn test_placeholder_tools_md_omitted() {
        let mut ctx = basic_ctx();
        ctx.tools_md = Some(
            "# TOOLS.md - Local Environment Notes\n\
             \n\
             ## Local Systems\n\
             - Record daemon URLs, workspace paths, repo conventions, services, and device names here.\n\
             - Capture local environment facts that help Assistant work accurately in this setup.\n\
             \n\
             ## Why This File Exists\n\
             - Use this file for environment knowledge and local conventions, not generic tool policy.\n"
                .to_string(),
        );
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Local Environment"));
    }

    #[test]
    fn test_non_placeholder_tools_md_present() {
        let mut ctx = basic_ctx();
        ctx.tools_md = Some(
            "# TOOLS.md - Local Environment Notes\n\
             \n\
             ## Local Systems\n\
             - Run `cargo build --workspace --lib` before changes are considered done.\n\
             - Use debug binaries during local validation.\n"
                .to_string(),
        );
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Local Environment"));
        assert!(prompt.contains("debug binaries"));
        assert!(!prompt.contains("# TOOLS.md"));
    }

    #[test]
    fn test_user_md_omitted_in_minimal_mode() {
        let mut ctx = basic_ctx();
        ctx.prompt_mode = PromptMode::Minimal;
        ctx.user_md = Some("- Name: Alice".to_string());
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## User Preferences"));
    }

    #[test]
    fn test_memory_md_section_present() {
        let mut ctx = basic_ctx();
        ctx.memory_md = Some(
            "# Long-Term Memory\n- Remember project.alpha.status before proposing changes."
                .to_string(),
        );
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Long-Term Memory"));
        assert!(prompt.contains("project.alpha.status"));
    }

    #[test]
    fn test_placeholder_memory_md_omitted() {
        let mut ctx = basic_ctx();
        ctx.memory_md = Some(
            "# Long-Term Memory\n\
             <!-- Curated knowledge the agent preserves across sessions -->\n"
                .to_string(),
        );
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Long-Term Memory"));
    }

    #[test]
    fn test_memory_context_message_present() {
        let message = build_memory_context_message(
            &[
                "[project.alpha] Architecture decision: use Axum".to_string(),
                "User prefers concise summaries".to_string(),
            ],
            &["Run memory_cleanup before reuse: migrate legacy key [theme] to [general.theme]"
                .to_string()],
            &["Review stale memory before reuse: [project.alpha.status] (kind=project_state, review_at=2026-03-10T00:00:00Z, tags=project,alpha)".to_string()],
            &["[pref.editor.theme] (kind=preference, freshness=durable, lifecycle=active, tags=profile,ui) solarized dark".to_string()],
            &["session_2026-03-11_alpha: Reviewed prompt pipeline".to_string()],
        )
        .unwrap();

        assert!(message.contains("[Memory context]"));
        assert!(message.contains("Relevant recalled memories"));
        assert!(message.contains("Governance maintenance signals"));
        assert!(message.contains("Governance attention signals"));
        assert!(message.contains("Governed memory candidates"));
        assert!(message.contains("Recent session summaries"));
        assert!(message.contains("Architecture decision"));
        assert!(message.contains("Run memory_cleanup before reuse"));
        assert!(message.contains("Review stale memory before reuse"));
        assert!(message.contains("pref.editor.theme"));
        assert!(message.contains("Reviewed prompt pipeline"));
    }

    #[test]
    fn test_memory_context_message_omitted_when_empty() {
        assert!(build_memory_context_message(&[], &[], &[], &[], &[]).is_none());
    }

    #[test]
    fn test_canonical_context_not_in_system_prompt() {
        let mut ctx = basic_ctx();
        ctx.canonical_context =
            Some("User was discussing Rust async patterns last time.".to_string());
        let prompt = build_system_prompt(&ctx);
        // Canonical context should NOT be in system prompt (moved to user message)
        assert!(!prompt.contains("## Previous Conversation Context"));
        assert!(!prompt.contains("Rust async patterns"));
        // But should be available via build_canonical_context_message
        let msg = build_canonical_context_message(&ctx);
        assert!(msg.is_some());
        assert!(msg.unwrap().contains("Rust async patterns"));
    }

    #[test]
    fn test_canonical_context_omitted_for_minimal_mode() {
        let mut ctx = basic_ctx();
        ctx.prompt_mode = PromptMode::Minimal;
        ctx.canonical_context = Some("Previous context here.".to_string());
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("Previous Conversation Context"));
        // Should also be None from build_canonical_context_message
        assert!(build_canonical_context_message(&ctx).is_none());
    }

    #[test]
    fn test_empty_base_prompt_generates_default_identity() {
        let ctx = PromptContext {
            agent_name: "helper".to_string(),
            agent_description: "A helpful agent".to_string(),
            ..Default::default()
        };
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("You are helper"));
        assert!(prompt.contains("A helpful agent"));
    }

    #[test]
    fn test_workspace_context_present() {
        let mut ctx = basic_ctx();
        ctx.workspace_context = Some(
            "## Workspace Context\n- Project: project (Rust)\n- Git repository: yes".to_string(),
        );
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Workspace Context"));
        assert!(prompt.contains("Project: project"));
    }

    #[test]
    fn test_identity_frontmatter_parsed_without_raw_yaml() {
        let mut ctx = basic_ctx();
        ctx.identity_md = Some(
            "---\nname: Assistant\narchetype: assistant\nvibe: sharp\ngreeting_style: blunt\n---\n# Identity\n- Role: helper\n"
                .to_string(),
        );
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Identity"));
        assert!(prompt.contains("Role: helper"));
        assert!(prompt
            .contains("Identity traits: archetype assistant, vibe sharp, greeting style blunt."));
        assert!(!prompt.contains("\n# Identity\n"));
        assert!(!prompt.contains("name: Assistant"));
        assert!(!prompt.contains("archetype: assistant"));
    }

    #[test]
    fn test_identity_frontmatter_ignores_empty_values() {
        let mut ctx = basic_ctx();
        ctx.identity_md = Some(
            "---\nname: Assistant\narchetype: assistant\nvibe:\nemoji:\ngreeting_style: blunt\n---\n# Identity\n- Role: helper\n"
                .to_string(),
        );
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("Identity traits: archetype assistant, greeting style blunt."));
        assert!(!prompt.contains("vibe "));
        assert!(!prompt.contains("emoji"));
    }

    #[test]
    fn test_html_comments_stripped_from_workspace_content() {
        let mut ctx = basic_ctx();
        ctx.soul_md = Some("# SOUL.md\n<!-- internal note -->\nStay sharp.\n".to_string());
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("Stay sharp."));
        assert!(!prompt.contains("internal note"));
        assert!(!prompt.contains("<!--"));
    }

    #[test]
    fn test_redundant_workspace_heading_removed() {
        let section = build_workspace_file_section(
            "Guidelines",
            "AGENTS.md",
            Some("# AGENTS.md\n- Be useful.\n"),
            400,
        )
        .unwrap();
        assert!(section.contains("## Guidelines"));
        assert!(section.contains("- Be useful."));
        assert!(!section.contains("# AGENTS.md"));
    }

    #[test]
    fn test_cap_str_short() {
        assert_eq!(cap_str("hello", 10), "hello");
    }

    #[test]
    fn test_cap_str_long() {
        let result = cap_str("hello world", 5);
        assert_eq!(result, "hello...");
    }

    #[test]
    fn test_cap_str_multibyte_utf8() {
        // This was panicking with "byte index is not a char boundary" (#38)
        let chinese = "你好世界这是一个测试字符串";
        let result = cap_str(chinese, 4);
        assert_eq!(result, "你好世界...");
        // Exact boundary
        assert_eq!(cap_str(chinese, 100), chinese);
    }

    #[test]
    fn test_cap_str_emoji() {
        let emoji = "👋🌍🚀✨💯";
        let result = cap_str(emoji, 3);
        assert_eq!(result, "👋🌍🚀...");
    }

    #[test]
    fn test_capitalize() {
        assert_eq!(capitalize("files"), "Files");
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("MCP"), "MCP");
    }
}
