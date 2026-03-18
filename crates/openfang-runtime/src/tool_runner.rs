//! Built-in tool execution.
//!
//! Provides filesystem, web, shell, and inter-agent tools. Agent tools
//! (agent_send, agent_spawn, etc.) require a KernelHandle to be passed in.

use crate::kernel_handle::KernelHandle;
use crate::mcp;
use crate::web_search::{parse_ddg_results, WebToolsContext};
use openfang_skills::registry::SkillRegistry;
use openfang_types::memory::{
    build_memory_autoconverge_plan, build_memory_record_metadata,
    canonicalize_memory_namespace, canonicalize_memory_tag_filters, canonicalize_user_memory_key,
    collect_memory_metadata, is_internal_memory_key, is_memory_metadata_key,
    memory_key_matches_prefix, memory_key_namespace, memory_lifecycle_snapshot,
    memory_lookup_candidates, memory_metadata_key, memory_tags_match, plan_memory_cleanup,
    MemoryCleanupAction, MemoryConflictPolicy, MemoryFreshness, MemoryLifecycleState,
    MEMORY_CLEANUP_SOURCE,
};
use openfang_types::taint::{TaintLabel, TaintSink, TaintedValue};
use openfang_types::tool::{ToolDefinition, ToolResult};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use openfang_types::tool_compat::normalize_tool_name;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, warn};

/// Maximum inter-agent call depth to prevent infinite recursion (A->B->C->...).
const MAX_AGENT_CALL_DEPTH: u32 = 5;

macro_rules! tool_definition {
    ($($tt:tt)*) => {
        ToolDefinition {
            defer_loading: false,
            $($tt)*
        }
    };
}

/// Check if a shell command should be blocked by taint tracking.
///
/// Layer 1: Shell metacharacter injection (backticks, `$(`, `${`, etc.)
/// Layer 2: Heuristic patterns for injected external data (piped curl, base64, eval)
///
/// This implements the TaintSink::shell_exec() policy from SOTA 2.
fn check_taint_shell_exec(command: &str) -> Option<String> {
    // Layer 1: Block shell metacharacters that enable command injection.
    // Uses the same validator as subprocess_sandbox and docker_sandbox.
    if let Some(reason) = crate::subprocess_sandbox::contains_shell_metacharacters(command) {
        return Some(format!("Shell metacharacter injection blocked: {reason}"));
    }

    // Layer 2: Heuristic patterns for injected external URLs / base64 payloads
    let suspicious_patterns = ["curl ", "wget ", "| sh", "| bash", "base64 -d", "eval "];
    for pattern in &suspicious_patterns {
        if command.contains(pattern) {
            let mut labels = HashSet::new();
            labels.insert(TaintLabel::ExternalNetwork);
            let tainted = TaintedValue::new(command, labels, "llm_tool_call");
            if let Err(violation) = tainted.check_sink(&TaintSink::shell_exec()) {
                warn!(command = crate::str_utils::safe_truncate_str(command, 80), %violation, "Shell taint check failed");
                return Some(violation.to_string());
            }
        }
    }
    None
}

/// Check if a URL should be blocked by taint tracking before network fetch.
///
/// Blocks URLs that appear to contain API keys, tokens, or other secrets
/// in query parameters (potential data exfiltration). Implements TaintSink::net_fetch().
fn check_taint_net_fetch(url: &str) -> Option<String> {
    let exfil_patterns = [
        "api_key=",
        "apikey=",
        "token=",
        "secret=",
        "password=",
        "Authorization:",
    ];
    for pattern in &exfil_patterns {
        if url.to_lowercase().contains(&pattern.to_lowercase()) {
            let mut labels = HashSet::new();
            labels.insert(TaintLabel::Secret);
            let tainted = TaintedValue::new(url, labels, "llm_tool_call");
            if let Err(violation) = tainted.check_sink(&TaintSink::net_fetch()) {
                warn!(url = crate::str_utils::safe_truncate_str(url, 80), %violation, "Net fetch taint check failed");
                return Some(violation.to_string());
            }
        }
    }
    None
}

tokio::task_local! {
    /// Tracks the current inter-agent call depth within a task.
    static AGENT_CALL_DEPTH: std::cell::Cell<u32>;
    /// Canvas max HTML size in bytes (set from kernel config at loop start).
    pub static CANVAS_MAX_BYTES: usize;
}

/// Get the current inter-agent call depth from the task-local context.
/// Returns 0 if called outside an agent task.
pub fn current_agent_depth() -> u32 {
    AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0)
}

/// Stateful tool runtime for a single agent-loop execution.
///
/// This currently keeps the authorized tool surface and exposes the
/// per-request visible tool list. Dynamic tool expansion will build on this
/// object instead of teaching `agent_loop` about hidden/deferred tool state.
pub struct ToolRunner<'a> {
    visible_tools: Vec<ToolDefinition>,
    visible_tool_names: Vec<String>,
    hidden_tools: HashMap<String, HiddenToolEntry>,
    kernel: Option<&'a Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&'a str>,
    skill_registry: Option<&'a SkillRegistry>,
    mcp_connections: Option<&'a tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    web_ctx: Option<&'a WebToolsContext>,
    browser_ctx: Option<&'a crate::browser::BrowserManager>,
    allowed_env_vars: Option<&'a [String]>,
    workspace_root: Option<&'a Path>,
    media_engine: Option<&'a crate::media_understanding::MediaEngine>,
    exec_policy: Option<&'a openfang_types::config::ExecPolicy>,
    tts_engine: Option<&'a crate::tts::TtsEngine>,
    docker_config: Option<&'a openfang_types::config::DockerSandboxConfig>,
    process_manager: Option<&'a crate::process_manager::ProcessManager>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HiddenToolSource {
    Builtin,
    Skill,
    Mcp,
}

#[derive(Debug, Clone)]
struct HiddenToolEntry {
    tool: ToolDefinition,
    source: HiddenToolSource,
    provider_name: Option<String>,
    search_text: String,
}

#[derive(Debug, Serialize)]
struct ToolSearchResultItem {
    kind: String,
    name: String,
    description: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    has_instructions: Option<bool>,
}

#[derive(Debug)]
struct RankedToolSearchItem {
    score: f32,
    expandable_tool_name: Option<String>,
    item: ToolSearchResultItem,
}

impl<'a> ToolRunner<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        authorized_tools: Vec<ToolDefinition>,
        kernel: Option<&'a Arc<dyn KernelHandle>>,
        caller_agent_id: Option<&'a str>,
        skill_registry: Option<&'a SkillRegistry>,
        mcp_connections: Option<&'a tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
        web_ctx: Option<&'a WebToolsContext>,
        browser_ctx: Option<&'a crate::browser::BrowserManager>,
        allowed_env_vars: Option<&'a [String]>,
        workspace_root: Option<&'a Path>,
        media_engine: Option<&'a crate::media_understanding::MediaEngine>,
        exec_policy: Option<&'a openfang_types::config::ExecPolicy>,
        tts_engine: Option<&'a crate::tts::TtsEngine>,
        docker_config: Option<&'a openfang_types::config::DockerSandboxConfig>,
        process_manager: Option<&'a crate::process_manager::ProcessManager>,
    ) -> Self {
        let mut visible_tools = Vec::new();
        let mut visible_tool_names = Vec::new();
        let mut hidden_tools = HashMap::new();

        for tool in authorized_tools {
            if tool.defer_loading {
                let hidden = HiddenToolEntry::from_tool(&tool, skill_registry);
                hidden_tools.insert(tool.name.clone(), hidden);
                continue;
            }

            visible_tool_names.push(tool.name.clone());
            visible_tools.push(tool);
        }

        Self {
            visible_tools,
            visible_tool_names,
            hidden_tools,
            kernel,
            caller_agent_id,
            skill_registry,
            mcp_connections,
            web_ctx,
            browser_ctx,
            allowed_env_vars,
            workspace_root,
            media_engine,
            exec_policy,
            tts_engine,
            docker_config,
            process_manager,
        }
    }

    pub fn visible_tools(&self) -> &[ToolDefinition] {
        &self.visible_tools
    }

    pub fn visible_tool_names(&self) -> &[String] {
        &self.visible_tool_names
    }

    pub async fn execute_tool_call(
        &mut self,
        tool_use_id: &str,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> ToolResult {
        if tool_name == "tool_search" {
            return self.execute_tool_search(tool_use_id, input);
        }

        execute_tool(
            tool_use_id,
            tool_name,
            input,
            self.kernel,
            Some(&self.visible_tool_names),
            self.caller_agent_id,
            self.skill_registry,
            self.mcp_connections,
            self.web_ctx,
            self.browser_ctx,
            self.allowed_env_vars,
            self.workspace_root,
            self.media_engine,
            self.exec_policy,
            self.tts_engine,
            self.docker_config,
            self.process_manager,
        )
        .await
    }

    fn execute_tool_search(&mut self, tool_use_id: &str, input: &serde_json::Value) -> ToolResult {
        let query = input["query"].as_str().unwrap_or("").trim();
        let top_k = input["top_k"].as_u64().unwrap_or(3) as usize;
        let results = self.search_discovery_items(query, top_k);

        let matched_names: Vec<String> = results
            .iter()
            .filter_map(|result| result.expandable_tool_name.clone())
            .collect();
        self.expand_hidden_tools(&matched_names);

        let visible_results: Vec<&ToolSearchResultItem> =
            results.iter().map(|result| &result.item).collect();

        match serde_json::to_string_pretty(&serde_json::json!({ "results": visible_results })) {
            Ok(content) => ToolResult {
                tool_use_id: tool_use_id.to_string(),
                content,
                is_error: false,
            },
            Err(err) => ToolResult {
                tool_use_id: tool_use_id.to_string(),
                content: format!("Failed to serialize tool search results: {err}"),
                is_error: true,
            },
        }
    }

    fn search_discovery_items(&self, query: &str, top_k: usize) -> Vec<RankedToolSearchItem> {
        if query.trim().is_empty() || top_k == 0 {
            return Vec::new();
        }

        let mut results = self.search_hidden_tools(query);
        results.extend(self.search_skill_manuals(query));
        results.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.item.kind.cmp(&b.item.kind))
                .then_with(|| a.item.name.cmp(&b.item.name))
        });
        results.truncate(top_k);
        results
    }

    fn expand_hidden_tools(&mut self, tool_names: &[String]) {
        for tool_name in tool_names {
            let Some(entry) = self.hidden_tools.remove(tool_name) else {
                continue;
            };
            if self
                .visible_tool_names
                .iter()
                .any(|name| name == &entry.tool.name)
            {
                continue;
            }
            self.visible_tool_names.push(entry.tool.name.clone());
            self.visible_tools.push(entry.tool);
        }
    }

    fn search_hidden_tools(&self, query: &str) -> Vec<RankedToolSearchItem> {
        let normalized_query = query.trim().to_lowercase();
        if normalized_query.is_empty() {
            return Vec::new();
        }

        let query_terms: Vec<&str> = normalized_query.split_whitespace().collect();
        let mut matches: Vec<(i32, &HiddenToolEntry)> = self
            .hidden_tools
            .values()
            .filter_map(|entry| {
                let score = hidden_tool_match_score(entry, &normalized_query, &query_terms);
                (score > 0).then_some((score, entry))
            })
            .collect();

        matches.sort_by(|(score_a, entry_a), (score_b, entry_b)| {
            score_b
                .cmp(score_a)
                .then_with(|| entry_a.tool.name.cmp(&entry_b.tool.name))
        });

        matches
            .into_iter()
            .map(|(score, entry)| RankedToolSearchItem {
                score: score as f32,
                expandable_tool_name: Some(entry.tool.name.clone()),
                item: ToolSearchResultItem {
                    kind: "tool".to_string(),
                    name: entry.tool.name.clone(),
                    description: entry.tool.description.clone(),
                    source: entry.source.as_str().to_string(),
                    provider: entry.provider_name.clone(),
                    has_instructions: None,
                },
            })
            .collect()
    }

    fn search_skill_manuals(&self, query: &str) -> Vec<RankedToolSearchItem> {
        let skill_results = if let Some(kh) = self.kernel {
            kh.search_visible_skills(query, 8, self.caller_agent_id)
        } else if let Some(registry) = self.skill_registry {
            Ok(registry.search(query, 8, None))
        } else {
            Err("Skill registry not available.".to_string())
        };

        match skill_results {
            Ok(results) => results
                .into_iter()
                .filter(|result| result.has_prompt_context)
                .map(|result| RankedToolSearchItem {
                    score: result.score,
                    expandable_tool_name: None,
                    item: ToolSearchResultItem {
                        kind: "skill_manual".to_string(),
                        name: result.name,
                        description: result.description,
                        source: "skill".to_string(),
                        provider: None,
                        has_instructions: Some(true),
                    },
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

impl HiddenToolEntry {
    fn from_tool(tool: &ToolDefinition, skill_registry: Option<&SkillRegistry>) -> Self {
        if let Some(registry) = skill_registry {
            if let Some(skill) = registry.find_tool_provider(&tool.name) {
                let mut parts = vec![
                    tool.name.clone(),
                    tool.description.clone(),
                    "skill".to_string(),
                    skill.manifest.skill.name.clone(),
                    skill.manifest.skill.description.clone(),
                ];
                parts.extend(skill.manifest.skill.tags.iter().cloned());
                return Self {
                    tool: tool.clone(),
                    source: HiddenToolSource::Skill,
                    provider_name: Some(skill.manifest.skill.name.clone()),
                    search_text: parts.join(" ").to_lowercase(),
                };
            }
        }

        if mcp::is_mcp_tool(&tool.name) {
            let provider_name = mcp::extract_mcp_server(&tool.name).map(str::to_string);
            let mut parts = vec![
                tool.name.clone(),
                tool.description.clone(),
                "mcp".to_string(),
            ];
            if let Some(ref provider) = provider_name {
                parts.push(provider.clone());
            }
            return Self {
                tool: tool.clone(),
                source: HiddenToolSource::Mcp,
                provider_name,
                search_text: parts.join(" ").to_lowercase(),
            };
        }

        Self {
            tool: tool.clone(),
            source: HiddenToolSource::Builtin,
            provider_name: None,
            search_text: format!("{} {} builtin", tool.name, tool.description).to_lowercase(),
        }
    }
}

impl HiddenToolSource {
    fn as_str(self) -> &'static str {
        match self {
            HiddenToolSource::Builtin => "builtin",
            HiddenToolSource::Skill => "skill",
            HiddenToolSource::Mcp => "mcp",
        }
    }
}

fn hidden_tool_match_score(
    entry: &HiddenToolEntry,
    normalized_query: &str,
    query_terms: &[&str],
) -> i32 {
    let name = entry.tool.name.to_lowercase();
    let mut score = 0;

    if name == normalized_query {
        score += 200;
    } else if name.contains(normalized_query) {
        score += 120;
    }

    if entry.search_text.contains(normalized_query) {
        score += 80;
    }

    for term in query_terms {
        if name.contains(term) {
            score += 40;
        } else if entry.search_text.contains(term) {
            score += 15;
        }
    }

    score
}

/// Execute a tool by name with the given input, returning a ToolResult.
///
/// The optional `kernel` handle enables inter-agent tools. If `None`,
/// agent tools will return an error indicating the kernel is not available.
///
/// `allowed_tools` enforces capability-based security: if provided, only
/// tools in the list may execute. This prevents an LLM from hallucinating
/// tool names outside the agent's capability grants.
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool(
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    allowed_tools: Option<&[String]>,
    caller_agent_id: Option<&str>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    allowed_env_vars: Option<&[String]>,
    workspace_root: Option<&Path>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    exec_policy: Option<&openfang_types::config::ExecPolicy>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&openfang_types::config::DockerSandboxConfig>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
) -> ToolResult {
    // Normalize the tool name through compat mappings so LLM-hallucinated aliases
    // (e.g. "fs-write" → "file_write") resolve to the canonical OpenFang name.
    let tool_name = normalize_tool_name(tool_name);

    // Capability enforcement: reject tools not in the allowed list
    if let Some(allowed) = allowed_tools {
        if !allowed.iter().any(|t| t == tool_name) {
            warn!(tool_name, "Capability denied: tool not in allowed list");
            return ToolResult {
                tool_use_id: tool_use_id.to_string(),
                content: format!(
                    "Permission denied: agent does not have capability to use tool '{tool_name}'"
                ),
                is_error: true,
            };
        }
    }

    // Approval gate: check if this tool requires human approval before execution
    if let Some(kh) = kernel {
        if kh.requires_approval(tool_name) {
            let agent_id_str = caller_agent_id.unwrap_or("unknown");
            let input_str = input.to_string();
            let summary = format!(
                "{}: {}",
                tool_name,
                openfang_types::truncate_str(&input_str, 200)
            );
            match kh.request_approval(agent_id_str, tool_name, &summary).await {
                Ok(true) => {
                    debug!(tool_name, "Approval granted — proceeding with execution");
                }
                Ok(false) => {
                    warn!(tool_name, "Approval denied — blocking tool execution");
                    return ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!(
                            "Execution denied: '{}' requires human approval and was denied or timed out. The operation was not performed.",
                            tool_name
                        ),
                        is_error: true,
                    };
                }
                Err(e) => {
                    warn!(tool_name, error = %e, "Approval system error");
                    return ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!("Approval system error: {e}"),
                        is_error: true,
                    };
                }
            }
        }
    }

    debug!(tool_name, "Executing tool");
    let result = match tool_name {
        // Filesystem tools
        "file_read" => tool_file_read(input, workspace_root).await,
        "file_write" => tool_file_write(input, workspace_root).await,
        "file_list" => tool_file_list(input, workspace_root).await,
        "apply_patch" => tool_apply_patch(input, workspace_root).await,

        // Web tools (upgraded: multi-provider search, SSRF-protected fetch)
        "web_fetch" => {
            // Taint check: block URLs containing secrets/PII from being exfiltrated
            let url = input["url"].as_str().unwrap_or("");
            if let Some(violation) = check_taint_net_fetch(url) {
                return ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content: format!("Taint violation: {violation}"),
                    is_error: true,
                };
            }
            let method = input["method"].as_str().unwrap_or("GET");
            let headers = input.get("headers").and_then(|v| v.as_object());
            let body = input["body"].as_str();
            if let Some(ctx) = web_ctx {
                ctx.fetch.fetch_with_options(url, method, headers, body).await
            } else {
                tool_web_fetch_legacy(input).await
            }
        }
        "web_search" => {
            if let Some(ctx) = web_ctx {
                let query = input["query"].as_str().unwrap_or("");
                let max_results = input["max_results"].as_u64().unwrap_or(5) as usize;
                ctx.search.search(query, max_results).await
            } else {
                tool_web_search_legacy(input).await
            }
        }

        // Shell tool — metacharacter check + exec policy + taint check
        "shell_exec" => {
            let command = input["command"].as_str().unwrap_or("");

            // SECURITY: Always check for shell metacharacters, even in Full mode.
            // These enable command injection regardless of exec policy.
            if let Some(reason) = crate::subprocess_sandbox::contains_shell_metacharacters(command) {
                return ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content: format!(
                        "shell_exec blocked: command contains {reason}. \
                         Shell metacharacters are never allowed."
                    ),
                    is_error: true,
                };
            }

            // Exec policy enforcement (allowlist / deny / full)
            if let Some(policy) = exec_policy {
                if let Err(reason) =
                    crate::subprocess_sandbox::validate_command_allowlist(command, policy)
                {
                    return ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!(
                            "shell_exec blocked: {reason}. Current exec_policy.mode = '{:?}'. \
                             To allow shell commands, set exec_policy.mode = 'full' in the agent manifest or config.toml.",
                            policy.mode
                        ),
                        is_error: true,
                    };
                }
            }
            // Skip heuristic taint patterns for Full exec policy (e.g. hand agents that need curl)
            let is_full_exec = exec_policy
                .is_some_and(|p| p.mode == openfang_types::config::ExecSecurityMode::Full);
            if !is_full_exec {
                if let Some(violation) = check_taint_shell_exec(command) {
                    return ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!("Taint violation: {violation}"),
                        is_error: true,
                    };
                }
            }
            tool_shell_exec(
                input,
                allowed_env_vars.unwrap_or(&[]),
                workspace_root,
                exec_policy,
            )
            .await
        }

        // Inter-agent tools (require kernel handle)
        "agent_send" => tool_agent_send(input, kernel).await,
        "agent_spawn" => tool_agent_spawn(input, kernel, caller_agent_id).await,
        "agent_list" => tool_agent_list(kernel),
        "agent_kill" => tool_agent_kill(input, kernel),

        // Shared memory tools
        "memory_store" => tool_memory_store(input, kernel),
        "memory_recall" => tool_memory_recall(input, kernel),
        "memory_list" => tool_memory_list(input, kernel),
        "memory_cleanup" => tool_memory_cleanup(input, kernel),
        "memory_autoconverge" => {
            tool_memory_autoconverge(input, kernel, caller_agent_id, workspace_root)
        }

        // Collaboration tools
        "agent_find" => tool_agent_find(input, kernel),
        "task_post" => tool_task_post(input, kernel, caller_agent_id).await,
        "task_claim" => tool_task_claim(kernel, caller_agent_id).await,
        "task_complete" => tool_task_complete(input, kernel).await,
        "task_list" => tool_task_list(input, kernel).await,
        "event_publish" => tool_event_publish(input, kernel).await,

        // Scheduling tools
        "schedule_create" => tool_schedule_create(input, kernel).await,
        "schedule_list" => tool_schedule_list(kernel).await,
        "schedule_delete" => tool_schedule_delete(input, kernel).await,

        // Knowledge graph tools
        "knowledge_add_entity" => tool_knowledge_add_entity(input, kernel).await,
        "knowledge_add_relation" => tool_knowledge_add_relation(input, kernel).await,
        "knowledge_query" => tool_knowledge_query(input, kernel).await,

        // Image analysis tool
        "image_analyze" => tool_image_analyze(input).await,

        // Media understanding tools
        "media_describe" => tool_media_describe(input, media_engine).await,
        "media_transcribe" => tool_media_transcribe(input, media_engine).await,

        // Image generation tool
        "image_generate" => tool_image_generate(input, workspace_root).await,

        // TTS/STT tools
        "text_to_speech" => tool_text_to_speech(input, tts_engine, workspace_root).await,
        "speech_to_text" => tool_speech_to_text(input, media_engine, workspace_root).await,

        // Docker sandbox tool
        "docker_exec" => {
            tool_docker_exec(input, docker_config, workspace_root, caller_agent_id).await
        }

        // Location tool
        "location_get" => tool_location_get().await,

        // System time tool
        "system_time" => Ok(tool_system_time()),

        // Cron scheduling tools
        "cron_create" => tool_cron_create(input, kernel, caller_agent_id).await,
        "cron_list" => tool_cron_list(kernel, caller_agent_id).await,
        "cron_cancel" => tool_cron_cancel(input, kernel).await,

        // Channel send tool (proactive outbound messaging)
        "channel_send" => tool_channel_send(input, kernel, workspace_root).await,

        // Persistent process tools
        "process_start" => tool_process_start(input, process_manager, caller_agent_id).await,
        "process_poll" => tool_process_poll(input, process_manager).await,
        "process_write" => tool_process_write(input, process_manager).await,
        "process_kill" => tool_process_kill(input, process_manager).await,
        "process_list" => tool_process_list(process_manager, caller_agent_id).await,

        // Hand tools (curated autonomous capability packages)
        "hand_list" => tool_hand_list(kernel).await,
        "hand_activate" => tool_hand_activate(input, kernel).await,
        "hand_status" => tool_hand_status(input, kernel).await,
        "hand_deactivate" => tool_hand_deactivate(input, kernel).await,

        // A2A outbound tools (cross-instance agent communication)
        "a2a_discover" => tool_a2a_discover(input).await,
        "a2a_send" => tool_a2a_send(input, kernel).await,

        // Browser automation tools
        "browser_navigate" => {
            let url = input["url"].as_str().unwrap_or("");
            if let Some(violation) = check_taint_net_fetch(url) {
                return ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content: format!("Taint violation: {violation}"),
                    is_error: true,
                };
            }
            match browser_ctx {
                Some(mgr) => {
                    let aid = caller_agent_id.unwrap_or("default");
                    crate::browser::tool_browser_navigate(input, mgr, aid).await
                }
                None => Err(
                    "Browser tools not available. Ensure Chrome/Chromium is installed."
                        .to_string(),
                ),
            }
        }
        "browser_click" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_click(input, mgr, aid).await
            }
            None => Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string()),
        },
        "browser_type" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_type(input, mgr, aid).await
            }
            None => Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string()),
        },
        "browser_screenshot" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_screenshot(input, mgr, aid).await
            }
            None => Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string()),
        },
        "browser_read_page" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_read_page(input, mgr, aid).await
            }
            None => Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string()),
        },
        "browser_close" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_close(input, mgr, aid).await
            }
            None => Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string()),
        },
        "browser_scroll" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_scroll(input, mgr, aid).await
            }
            None => Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string()),
        },
        "browser_wait" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_wait(input, mgr, aid).await
            }
            None => Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string()),
        },
        "browser_run_js" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_run_js(input, mgr, aid).await
            }
            None => Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string()),
        },
        "browser_back" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_back(input, mgr, aid).await
            }
            None => Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string()),
        },

        // Canvas / A2UI tool
        "canvas_present" => tool_canvas_present(input, workspace_root).await,

        // Tool discovery requires the loop-local ToolRunner state.
        "tool_search" => Err(
            "tool_search requires the active agent loop and is not available through the stateless execution path."
                .to_string(),
        ),

        // Skill documentation tool
        "tool_get_instructions" => {
            let name = input["skill_name"].as_str().unwrap_or("");
            if let Some(registry) = skill_registry {
                if let Some(skill) = registry.get(name) {
                    if let Some(ref ctx) = skill.manifest.prompt_context {
                        Ok(ctx.clone())
                    } else {
                        Ok(format!("Skill '{}' has no additional instructions.", name))
                    }
                } else {
                    Err(format!("Skill '{}' not found.", name))
                }
            } else {
                Err("Skill registry not available.".to_string())
            }
        }

        "skill_install" => {
            let source = input["source"].as_str().unwrap_or("");
            let scope = input["scope"].as_str();
            if let Some(kh) = kernel {
                kh.skill_install(source, caller_agent_id, scope).await
            } else {
                Err("Kernel handle not available for skill installation.".to_string())
            }
        }

        "skill_create" => {
            let name = input["name"].as_str().unwrap_or("");
            let description = input["description"].as_str().unwrap_or("");
            let prompt = input["prompt"].as_str().unwrap_or("");
            let scope = input["scope"].as_str();
            if let Some(kh) = kernel {
                kh.skill_create(name, description, prompt, caller_agent_id, scope).await
            } else {
                Err("Kernel handle not available for skill creation.".to_string())
            }
        }

        other => {
            // Fallback 1: MCP tools (mcp_{server}_{tool} prefix)
            if mcp::is_mcp_tool(other) {
                if let Some(mcp_conns) = mcp_connections {
                    if let Some(server_name) = mcp::extract_mcp_server(other) {
                        let mut conns = mcp_conns.lock().await;
                        if let Some(conn) = conns
                            .iter_mut()
                            .find(|c| mcp::normalize_name(c.name()) == server_name)
                        {
                            debug!(
                                tool = other,
                                server = server_name,
                                "Dispatching to MCP server"
                            );
                            match conn.call_tool(other, input).await {
                                Ok(content) => Ok(content),
                                Err(e) => Err(format!("MCP tool call failed: {e}")),
                            }
                        } else {
                            Err(format!("MCP server '{server_name}' not connected"))
                        }
                    } else {
                        Err(format!("Invalid MCP tool name: {other}"))
                    }
                } else {
                    Err(format!("MCP not available for tool: {other}"))
                }
            }
            // Fallback 2: Skill registry tool providers
            else if let Some(registry) = skill_registry {
                if let Some((skill, tool_def)) = registry.find_tool_provider_with_def(other) {
                    if !tool_def.policy.model_invocable() {
                        Err(format!(
                            "Skill command '{}' is not invocable by the model.",
                            other
                        ))
                    } else if let Some(allowed) = allowed_tools {
                        if let Some(missing) = tool_def
                            .policy
                            .host_tools
                            .iter()
                            .find(|required| !allowed.iter().any(|t| t == *required))
                        {
                            Err(format!(
                                "Skill command '{}' requires unavailable host tool '{}'.",
                                other, missing
                            ))
                        } else {
                            debug!(tool = other, skill = %skill.manifest.skill.name, "Dispatching to skill");
                            match openfang_skills::loader::execute_skill_tool(
                                &skill.manifest,
                                &skill.path,
                                other,
                                input,
                            )
                            .await
                            {
                                Ok(skill_result) => {
                                    let content = serde_json::to_string(&skill_result.output)
                                        .unwrap_or_else(|_| skill_result.output.to_string());
                                    if skill_result.is_error {
                                        Err(content)
                                    } else {
                                        Ok(content)
                                    }
                                }
                                Err(e) => Err(format!("Skill execution failed: {e}")),
                            }
                        }
                    } else {
                        debug!(tool = other, skill = %skill.manifest.skill.name, "Dispatching to skill");
                        match openfang_skills::loader::execute_skill_tool(
                            &skill.manifest,
                            &skill.path,
                            other,
                            input,
                        )
                        .await
                        {
                            Ok(skill_result) => {
                                let content = serde_json::to_string(&skill_result.output)
                                    .unwrap_or_else(|_| skill_result.output.to_string());
                                if skill_result.is_error {
                                    Err(content)
                                } else {
                                    Ok(content)
                                }
                            }
                            Err(e) => Err(format!("Skill execution failed: {e}")),
                        }
                    }
                } else {
                    Err(format!("Unknown tool: {other}"))
                }
            } else {
                Err(format!("Unknown tool: {other}"))
            }
        }
    };

    match result {
        Ok(content) => ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content,
            is_error: false,
        },
        Err(err) => ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: format!("Error: {err}"),
            is_error: true,
        },
    }
}

/// Get definitions for all built-in tools.
pub fn builtin_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        // --- Filesystem tools ---
        tool_definition! {
            name: "file_read".to_string(),
            description: "Read the contents of a file. Paths are relative to the agent workspace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file path to read" }
                },
                "required": ["path"]
            }),
        },
        tool_definition! {
            name: "file_write".to_string(),
            description: "Write content to a file. Paths are relative to the agent workspace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file path to write to" },
                    "content": { "type": "string", "description": "The content to write" }
                },
                "required": ["path", "content"]
            }),
        },
        tool_definition! {
            name: "file_list".to_string(),
            description: "List files in a directory. Paths are relative to the agent workspace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The directory path to list" }
                },
                "required": ["path"]
            }),
        },
        tool_definition! {
            name: "apply_patch".to_string(),
            description: "Apply a multi-hunk diff patch to add, update, move, or delete files. Use this for targeted edits instead of full file overwrites.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "The patch in *** Begin Patch / *** End Patch format. Use *** Add File:, *** Update File:, *** Delete File: markers. Hunks use @@ headers with space (context), - (remove), + (add) prefixed lines."
                    }
                },
                "required": ["patch"]
            }),
        },
        // --- Web tools ---
        tool_definition! {
            name: "web_fetch".to_string(),
            description: "Fetch a specific URL with SSRF protection. Use this when retrieving that URL's contents is itself the task. Supports GET/POST/PUT/PATCH/DELETE. For GET, HTML is converted to Markdown. For other methods, returns raw response body.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The URL to fetch (http/https only)" },
                    "method": { "type": "string", "enum": ["GET","POST","PUT","PATCH","DELETE"], "description": "HTTP method (default: GET)" },
                    "headers": { "type": "object", "description": "Custom HTTP headers as key-value pairs" },
                    "body": { "type": "string", "description": "Request body for POST/PUT/PATCH" }
                },
                "required": ["url"]
            }),
        },
        tool_definition! {
            name: "web_search".to_string(),
            description: "Search public web sources using multiple providers (Tavily, Brave, Perplexity, DuckDuckGo) with automatic fallback. Use this for current or external information that is not already available locally or through specialized tools. Returns structured results with titles, URLs, and snippets.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The search query" },
                    "max_results": { "type": "integer", "description": "Maximum number of results to return (default: 5, max: 20)" }
                },
                "required": ["query"]
            }),
        },
        // --- Shell tool ---
        tool_definition! {
            name: "shell_exec".to_string(),
            description: "Execute a shell command and return its output.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute" },
                    "timeout_seconds": { "type": "integer", "description": "Timeout in seconds (default: 30)" }
                },
                "required": ["command"]
            }),
        },
        // --- Inter-agent tools ---
        tool_definition! {
            name: "agent_send".to_string(),
            description: "Send a message to another agent and receive their response. Accepts UUID or agent name. Use agent_find first to discover agents.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "The target agent's UUID or name" },
                    "message": { "type": "string", "description": "The message to send to the agent" }
                },
                "required": ["agent_id", "message"]
            }),
        },
        tool_definition! {
            name: "agent_spawn".to_string(),
            description: "Spawn a new agent from a TOML manifest. Returns the new agent's ID and name.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "manifest_toml": {
                        "type": "string",
                        "description": "The agent manifest in TOML format (must include name, module, [model], and [capabilities])"
                    }
                },
                "required": ["manifest_toml"]
            }),
        },
        tool_definition! {
            name: "agent_list".to_string(),
            description: "List all currently running agents with their IDs, names, states, and models.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        tool_definition! {
            name: "agent_kill".to_string(),
            description: "Kill (terminate) another agent by its ID.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "The agent's UUID to kill" }
                },
                "required": ["agent_id"]
            }),
        },
        // --- Shared memory tools ---
        tool_definition! {
            name: "memory_store".to_string(),
            description: "Store a value in shared memory accessible by all agents. Prefer namespaced keys such as `project.alpha.decision` or `pref.editor.theme` for durable memory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The storage key. Bare keys are normalized into the `general.` namespace." },
                    "namespace": { "type": "string", "description": "Optional namespace for a bare key. For example `project` with key `alpha.status` becomes `project.alpha.status`." },
                    "kind": { "type": "string", "description": "Optional governance kind such as `fact`, `preference`, `decision`, or `project_state`." },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional governance tags, normalized to lowercase and deduplicated." },
                    "freshness": { "type": "string", "enum": ["rolling", "durable", "archival"], "description": "Optional freshness class for lifecycle management." },
                    "conflict_policy": { "type": "string", "enum": ["overwrite", "skip_if_exists"], "description": "How to behave when the key already exists. Default: `overwrite`." },
                    "value": { "type": "string", "description": "The value to store (JSON-encode objects/arrays, or pass a plain string)" }
                },
                "required": ["key", "value"]
            }),
        },
        tool_definition! {
            name: "memory_recall".to_string(),
            description: "Recall a value from shared memory by key. Bare keys automatically fall back to the `general.` namespace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The storage key to recall" },
                    "namespace": { "type": "string", "description": "Optional namespace when recalling a bare key." }
                },
                "required": ["key"]
            }),
        },
        tool_definition! {
            name: "memory_list".to_string(),
            description: "List shared memory entries, optionally filtered by namespace, key prefix, tags, or lifecycle. Use when you need to discover available memory keys before recalling a specific one.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "namespace": { "type": "string", "description": "Optional namespace to restrict results, for example `project` or `pref`." },
                    "prefix": { "type": "string", "description": "Optional key prefix to filter by (for example `project.alpha.` or `pref.`)" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional governance tags filter. Returned entries must include all requested tags." },
                    "lifecycle": { "type": "string", "enum": ["active", "stale", "expired"], "description": "Optional lifecycle state filter for governed records." },
                    "limit": { "type": "integer", "description": "Maximum number of entries to return (default 20, max 100)" },
                    "include_values": { "type": "boolean", "description": "Whether to include stored values in the result (default true)" },
                    "include_internal": { "type": "boolean", "description": "Whether to include internal OpenFang keys such as session summaries and scheduler state (default false)." }
                }
            }),
        },
        tool_definition! {
            name: "memory_cleanup".to_string(),
            description: "Audit or apply governance cleanup for shared memory. Use to find legacy bare keys, orphan metadata sidecars, or canonical entries missing governed metadata.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "apply": { "type": "boolean", "description": "Whether to apply the cleanup plan. Default false returns audit findings only." },
                    "limit": { "type": "integer", "description": "Maximum number of findings to inspect or apply (default 200, max 200)." }
                }
            }),
        },
        tool_definition! {
            name: "memory_autoconverge".to_string(),
            description: "Audit or apply the managed MEMORY.md snapshot generated from governed shared memory promotion candidates. Use this to review what should be written into the agent workspace MEMORY.md before applying it.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "apply": { "type": "boolean", "description": "Whether to write the managed snapshot into MEMORY.md. Default false returns the review plan only." },
                    "limit": { "type": "integer", "description": "Maximum number of governed promotion candidates to include in the review (default 24, max 64)." }
                }
            }),
        },
        // --- Collaboration tools ---
        tool_definition! {
            name: "agent_find".to_string(),
            description: "Discover agents by name, tag, tool, or description. Use to find specialists before delegating work.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query (matches agent name, tags, tools, description)" }
                },
                "required": ["query"]
            }),
        },
        tool_definition! {
            name: "task_post".to_string(),
            description: "Post a task to the shared task queue for another agent to pick up.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short task title" },
                    "description": { "type": "string", "description": "Detailed task description" },
                    "assigned_to": { "type": "string", "description": "Agent name or ID to assign the task to (optional)" }
                },
                "required": ["title", "description"]
            }),
        },
        tool_definition! {
            name: "task_claim".to_string(),
            description: "Claim the next available task from the task queue assigned to you or unassigned.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        tool_definition! {
            name: "task_complete".to_string(),
            description: "Mark a previously claimed task as completed with a result.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "The task ID to complete" },
                    "result": { "type": "string", "description": "The result or outcome of the task" }
                },
                "required": ["task_id", "result"]
            }),
        },
        tool_definition! {
            name: "task_list".to_string(),
            description: "List tasks in the shared queue, optionally filtered by status (pending, in_progress, completed).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": { "type": "string", "description": "Filter by status: pending, in_progress, completed (optional)" }
                }
            }),
        },
        tool_definition! {
            name: "event_publish".to_string(),
            description: "Publish a custom event that can trigger proactive agents. Use to broadcast signals to the agent fleet.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "event_type": { "type": "string", "description": "Type identifier for the event (e.g., 'code_review_requested')" },
                    "payload": { "type": "object", "description": "JSON payload data for the event" }
                },
                "required": ["event_type"]
            }),
        },
        // --- Scheduling tools ---
        tool_definition! {
            name: "schedule_create".to_string(),
            description: "Schedule a recurring task using natural language or cron syntax. Examples: 'every 5 minutes', 'daily at 9am', 'weekdays at 6pm', '0 */5 * * *'.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "What this schedule does (e.g., 'Check for new emails')" },
                    "schedule": { "type": "string", "description": "Natural language or cron expression (e.g., 'every 5 minutes', 'daily at 9am', '0 */5 * * *')" },
                    "agent": { "type": "string", "description": "Agent name or ID to run this task (optional, defaults to self)" }
                },
                "required": ["description", "schedule"]
            }),
        },
        tool_definition! {
            name: "schedule_list".to_string(),
            description: "List all scheduled tasks with their IDs, descriptions, schedules, and next run times.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        tool_definition! {
            name: "schedule_delete".to_string(),
            description: "Remove a scheduled task by its ID.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The schedule ID to remove" }
                },
                "required": ["id"]
            }),
        },
        // --- Knowledge graph tools ---
        tool_definition! {
            name: "knowledge_add_entity".to_string(),
            description: "Add an entity to the knowledge graph. Entities represent people, organizations, projects, concepts, locations, tools, etc.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Display name of the entity" },
                    "entity_type": { "type": "string", "description": "Type: person, organization, project, concept, event, location, document, tool, or a custom type" },
                    "properties": { "type": "object", "description": "Arbitrary key-value properties (optional)" }
                },
                "required": ["name", "entity_type"]
            }),
        },
        tool_definition! {
            name: "knowledge_add_relation".to_string(),
            description: "Add a relation between two entities in the knowledge graph.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Source entity ID or name" },
                    "relation": { "type": "string", "description": "Relation type: works_at, knows_about, related_to, depends_on, owned_by, created_by, located_in, part_of, uses, produces, or a custom type" },
                    "target": { "type": "string", "description": "Target entity ID or name" },
                    "confidence": { "type": "number", "description": "Confidence score 0.0-1.0 (default: 1.0)" },
                    "properties": { "type": "object", "description": "Arbitrary key-value properties (optional)" }
                },
                "required": ["source", "relation", "target"]
            }),
        },
        tool_definition! {
            name: "knowledge_query".to_string(),
            description: "Query the knowledge graph. Filter by source entity, relation type, and/or target entity. Returns matching entity-relation-entity triples.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Filter by source entity name or ID (optional)" },
                    "relation": { "type": "string", "description": "Filter by relation type (optional)" },
                    "target": { "type": "string", "description": "Filter by target entity name or ID (optional)" },
                    "max_depth": { "type": "integer", "description": "Maximum traversal depth (default: 1)" }
                }
            }),
        },
        // --- Image analysis tool ---
        tool_definition! {
            name: "image_analyze".to_string(),
            description: "Analyze an image file — returns format, dimensions, file size, and a base64 preview. For vision-model analysis, include a prompt.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the image file" },
                    "prompt": { "type": "string", "description": "Optional prompt for vision analysis (e.g., 'Describe what you see')" }
                },
                "required": ["path"]
            }),
        },
        tool_definition! {
            name: "tool_search".to_string(),
            description: "Discover deferred tools, skills, and manuals relevant to the current task, and expand matching tools so they can be called immediately in the current turn. Use this when a specialized capability may exist but is not currently visible.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural-language description of the task or intent" },
                    "top_k": { "type": "integer", "description": "Maximum number of matches to return (default: 3)" }
                },
                "required": ["query"]
            }),
        },
        tool_definition! {
            name: "tool_get_instructions".to_string(),
            description: "Retrieve additional guidance for a discovered instructional resource. Use this after tool_search returns a result with has_instructions=true and you need the full guidance.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "skill_name": { "type": "string", "description": "The identifier of the discovered instructional resource to load" }
                },
                "required": ["skill_name"]
            }),
        },
        tool_definition! {
            name: "skill_install".to_string(),
            description: "Install a new skill. Source can be a FangHub slug ('agent-reach'), an arbitrary GitHub repo ('owner/repo'), a direct ZIP/TAR URL, or a local absolute directory path. Note: Use scope='workspace' for agent-specific tools, or 'global' (default) to make it available to the entire fleet in ~/.openfang/skills/.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "The skill source (slug, owner/repo, URL, or local path)" },
                    "scope": { "type": "string", "enum": ["global", "workspace"], "description": "Installation scope (default: global)" }
                },
                "required": ["source"]
            }),
        },
        tool_definition! {
            name: "skill_create".to_string(),
            description: "Install a new prompt-only skill natively by providing its prompt context. This is the preferred way to convert instructions from a webpage, document, or tweet into a reusable agent capability.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name for the skill (lowercase, kebab-case, e.g. 'tweet-reviewer')" },
                    "description": { "type": "string", "description": "A summary of what this skill adds to the agent fleet" },
                    "prompt": { "type": "string", "description": "Detailed prompt context instructions captured from the source" },
                    "scope": { "type": "string", "enum": ["global", "workspace"], "description": "Storage scope (default: global)" }
                },
                "required": ["name", "description", "prompt"]
            }),
        },
        // --- Location tool ---
        tool_definition! {
            name: "location_get".to_string(),
            description: "Get approximate geographic location based on IP address. Returns city, country, coordinates, and timezone.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- Browser automation tools ---
        tool_definition! {
            name: "browser_navigate".to_string(),
            description: "Navigate a browser to a URL. Returns the page title and readable content as markdown. Opens a persistent browser session.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The URL to navigate to (http/https only)" }
                },
                "required": ["url"]
            }),
        },
        tool_definition! {
            name: "browser_click".to_string(),
            description: "Click an element on the current browser page by CSS selector or visible text. Returns the resulting page state.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector (e.g., '#submit-btn', '.add-to-cart') or visible text to click" }
                },
                "required": ["selector"]
            }),
        },
        tool_definition! {
            name: "browser_type".to_string(),
            description: "Type text into an input field on the current browser page.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector for the input field (e.g., 'input[name=\"email\"]', '#search-box')" },
                    "text": { "type": "string", "description": "The text to type into the field" }
                },
                "required": ["selector", "text"]
            }),
        },
        tool_definition! {
            name: "browser_screenshot".to_string(),
            description: "Take a screenshot of the current browser page. Returns a base64-encoded PNG image.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        tool_definition! {
            name: "browser_read_page".to_string(),
            description: "Read the current browser page content as structured markdown. Use after clicking or navigating to see the updated page.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        tool_definition! {
            name: "browser_close".to_string(),
            description: "Close the browser session. The browser will also auto-close when the agent loop ends.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        tool_definition! {
            name: "browser_scroll".to_string(),
            description: "Scroll the browser page. Use this to see content below the fold or navigate long pages.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "direction": { "type": "string", "description": "Scroll direction: 'up', 'down', 'left', 'right' (default: 'down')" },
                    "amount": { "type": "integer", "description": "Pixels to scroll (default: 600)" }
                }
            }),
        },
        tool_definition! {
            name: "browser_wait".to_string(),
            description: "Wait for a CSS selector to appear on the page. Useful for dynamic content that loads asynchronously.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector to wait for" },
                    "timeout_ms": { "type": "integer", "description": "Max wait time in milliseconds (default: 5000, max: 30000)" }
                },
                "required": ["selector"]
            }),
        },
        tool_definition! {
            name: "browser_run_js".to_string(),
            description: "Run JavaScript on the current browser page and return the result. For advanced interactions that other browser tools cannot handle.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": { "type": "string", "description": "JavaScript expression to run in the page context" }
                },
                "required": ["expression"]
            }),
        },
        tool_definition! {
            name: "browser_back".to_string(),
            description: "Go back to the previous page in browser history.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- Media understanding tools ---
        tool_definition! {
            name: "media_describe".to_string(),
            description: "Describe an image using a vision-capable LLM. Auto-selects the best available provider (Anthropic, OpenAI, or Gemini). Returns a text description of the image content.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the image file (relative to workspace)" },
                    "prompt": { "type": "string", "description": "Optional prompt to guide the description (e.g., 'Extract all text from this image')" }
                },
                "required": ["path"]
            }),
        },
        tool_definition! {
            name: "media_transcribe".to_string(),
            description: "Transcribe audio to text using speech-to-text. Auto-selects the best available provider (Groq Whisper or OpenAI Whisper). Returns the transcript.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the audio file (relative to workspace). Supported: mp3, wav, ogg, flac, m4a, webm." },
                    "language": { "type": "string", "description": "Optional ISO-639-1 language code (e.g., 'en', 'es', 'ja')" }
                },
                "required": ["path"]
            }),
        },
        // --- Image generation tool ---
        tool_definition! {
            name: "image_generate".to_string(),
            description: "Generate images from a text prompt using DALL-E 3, DALL-E 2, or GPT-Image-1. Requires OPENAI_API_KEY. Generated images are saved to the workspace output/ directory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Text description of the image to generate (max 4000 chars)" },
                    "model": { "type": "string", "description": "Model to use: 'dall-e-3' (default), 'dall-e-2', or 'gpt-image-1'" },
                    "size": { "type": "string", "description": "Image size: '1024x1024' (default), '1024x1792', '1792x1024', '256x256', '512x512'" },
                    "quality": { "type": "string", "description": "Quality: 'hd' (default for dall-e-3) or 'standard'" },
                    "count": { "type": "integer", "description": "Number of images to generate (1-4, default: 1). DALL-E 3 only supports 1." }
                },
                "required": ["prompt"]
            }),
        },
        // --- Cron scheduling tools ---
        tool_definition! {
            name: "cron_create".to_string(),
            description: "Create a scheduled/cron job. Supports one-shot (at), recurring (every N seconds), and cron expressions. Max 50 jobs per agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Job name (max 128 chars, alphanumeric + spaces/hyphens/underscores)" },
                    "schedule": {
                        "type": "object",
                        "description": "Schedule: {\"kind\":\"at\",\"at\":\"2025-01-01T00:00:00Z\"} or {\"kind\":\"every\",\"every_secs\":300} or {\"kind\":\"cron\",\"expr\":\"0 */6 * * *\"}"
                    },
                    "action": {
                        "type": "object",
                        "description": "Action: {\"kind\":\"system_event\",\"text\":\"...\"} or {\"kind\":\"agent_turn\",\"message\":\"...\",\"timeout_secs\":300}"
                    },
                    "delivery": {
                        "type": "object",
                        "description": "Delivery target: {\"kind\":\"none\"} or {\"kind\":\"channel\",\"channel\":\"telegram\"} or {\"kind\":\"last_channel\"}"
                    },
                    "one_shot": { "type": "boolean", "description": "If true, auto-delete after execution. Default: false" }
                },
                "required": ["name", "schedule", "action"]
            }),
        },
        tool_definition! {
            name: "cron_list".to_string(),
            description: "List all scheduled/cron jobs for the current agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        tool_definition! {
            name: "cron_cancel".to_string(),
            description: "Cancel a scheduled/cron job by its ID.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "The UUID of the cron job to cancel" }
                },
                "required": ["job_id"]
            }),
        },
        // --- Channel send tool (proactive outbound messaging) ---
        tool_definition! {
            name: "channel_send".to_string(),
            description: "Send a message or media to a user on a configured channel (email, telegram, slack, etc). For email: recipient is the email address; optionally set subject. For media: set image_url, file_url, or file_path to send an image or file instead of (or alongside) text. Use thread_id to reply in a specific thread/topic.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "channel": { "type": "string", "description": "Channel adapter name (e.g., 'email', 'telegram', 'slack', 'discord')" },
                    "recipient": { "type": "string", "description": "Platform-specific recipient identifier (email address, user ID, etc.)" },
                    "subject": { "type": "string", "description": "Optional subject line (used for email; ignored for other channels)" },
                    "message": { "type": "string", "description": "The message body to send (required for text, optional caption for media)" },
                    "image_url": { "type": "string", "description": "URL of an image to send (supported on Telegram, Discord, Slack)" },
                    "file_url": { "type": "string", "description": "URL of a file to send as attachment" },
                    "file_path": { "type": "string", "description": "Local file path to send as attachment (reads from disk; use instead of file_url for local files)" },
                    "filename": { "type": "string", "description": "Filename for file attachments (defaults to the basename of file_path, or 'file')" },
                    "thread_id": { "type": "string", "description": "Thread/topic ID to reply in (e.g., Telegram message_thread_id, Slack thread_ts)" }
                },
                "required": ["channel", "recipient"]
            }),
        },
        // --- Hand tools (curated autonomous capability packages) ---
        tool_definition! {
            name: "hand_list".to_string(),
            description: "List available Hands (curated autonomous packages) and their activation status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        tool_definition! {
            name: "hand_activate".to_string(),
            description: "Activate a Hand — spawns a specialized autonomous agent with curated tools and skills.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "hand_id": { "type": "string", "description": "The ID of the hand to activate (e.g. 'researcher', 'clip', 'browser')" },
                    "config": { "type": "object", "description": "Optional configuration overrides for the hand's settings" }
                },
                "required": ["hand_id"]
            }),
        },
        tool_definition! {
            name: "hand_status".to_string(),
            description: "Check the status and metrics of an active Hand.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "hand_id": { "type": "string", "description": "The ID of the hand to check status for" }
                },
                "required": ["hand_id"]
            }),
        },
        tool_definition! {
            name: "hand_deactivate".to_string(),
            description: "Deactivate a running Hand and stop its agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "instance_id": { "type": "string", "description": "The UUID of the hand instance to deactivate" }
                },
                "required": ["instance_id"]
            }),
        },
        // --- A2A outbound tools ---
        tool_definition! {
            name: "a2a_discover".to_string(),
            description: "Discover an external A2A agent by fetching its agent card from a URL. Returns the agent's name, description, skills, and supported protocols.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Base URL of the remote OpenFang/A2A-compatible agent (e.g., 'https://agent.example.com')" }
                },
                "required": ["url"]
            }),
        },
        tool_definition! {
            name: "a2a_send".to_string(),
            description: "Send a task/message to an external A2A agent and get the response. Use agent_name to send to a previously discovered agent, or agent_url for direct addressing.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "The task/message to send to the remote agent" },
                    "agent_url": { "type": "string", "description": "Direct URL of the remote agent's A2A endpoint" },
                    "agent_name": { "type": "string", "description": "Name of a previously discovered A2A agent (looked up from kernel)" },
                    "session_id": { "type": "string", "description": "Optional session ID for multi-turn conversations" }
                },
                "required": ["message"]
            }),
        },
        // --- TTS/STT tools ---
        tool_definition! {
            name: "text_to_speech".to_string(),
            description: "Convert text to speech audio. Auto-selects OpenAI or ElevenLabs. Saves audio to workspace output/ directory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "The text to convert to speech (max 4096 chars)" },
                    "voice": { "type": "string", "description": "Voice name: 'alloy', 'echo', 'fable', 'onyx', 'nova', 'shimmer' (default: 'alloy')" },
                    "format": { "type": "string", "description": "Output format: 'mp3', 'opus', 'aac', 'flac' (default: 'mp3')" }
                },
                "required": ["text"]
            }),
        },
        tool_definition! {
            name: "speech_to_text".to_string(),
            description: "Transcribe audio to text using speech-to-text. Auto-selects Groq Whisper or OpenAI Whisper. Supported formats: mp3, wav, ogg, flac, m4a, webm.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the audio file (relative to workspace)" },
                    "language": { "type": "string", "description": "Optional ISO-639-1 language code (e.g., 'en', 'es', 'ja')" }
                },
                "required": ["path"]
            }),
        },
        // --- Docker sandbox tool ---
        tool_definition! {
            name: "docker_exec".to_string(),
            description: "Execute a command inside a Docker container sandbox. Provides OS-level isolation with resource limits, network isolation, and capability dropping. Requires Docker to be installed and docker.enabled=true.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute inside the container" }
                },
                "required": ["command"]
            }),
        },
        // --- Persistent process tools ---
        tool_definition! {
            name: "process_start".to_string(),
            description: "Start a long-running process (REPL, server, watcher). Returns a process_id for subsequent poll/write/kill operations. Max 5 processes per agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The executable to run (e.g. 'python', 'node', 'npm')" },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Command-line arguments (e.g. ['-i'] for interactive Python)"
                    }
                },
                "required": ["command"]
            }),
        },
        tool_definition! {
            name: "process_poll".to_string(),
            description: "Read accumulated stdout/stderr from a running process. Non-blocking: returns whatever output has buffered since the last poll.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID returned by process_start" }
                },
                "required": ["process_id"]
            }),
        },
        tool_definition! {
            name: "process_write".to_string(),
            description: "Write data to a running process's stdin. A newline is appended automatically if not present.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID returned by process_start" },
                    "data": { "type": "string", "description": "The data to write to stdin" }
                },
                "required": ["process_id", "data"]
            }),
        },
        tool_definition! {
            name: "process_kill".to_string(),
            description: "Terminate a running process and clean up its resources.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID returned by process_start" }
                },
                "required": ["process_id"]
            }),
        },
        tool_definition! {
            name: "process_list".to_string(),
            description: "List all running processes for the current agent, including their IDs, commands, uptime, and alive status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- System time tool ---
        tool_definition! {
            name: "system_time".to_string(),
            description: "Get the current date, time, and timezone. Returns ISO 8601 timestamp, Unix epoch seconds, and timezone info.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        // --- Canvas / A2UI tool ---
        tool_definition! {
            name: "canvas_present".to_string(),
            description: "Present an interactive HTML canvas to the user. The HTML is sanitized (no scripts, no event handlers) and saved to the workspace. The dashboard will render it in a panel. Use for rich data visualizations, formatted reports, or interactive UI.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "html": { "type": "string", "description": "The HTML content to present. Must not contain <script> tags, event handlers, or javascript: URLs." },
                    "title": { "type": "string", "description": "Optional title for the canvas panel" }
                },
                "required": ["html"]
            }),
        },
    ]
}

// ---------------------------------------------------------------------------
// Filesystem tools
// ---------------------------------------------------------------------------

/// SECURITY: Reject path traversal attempts. Forbids `..` components in file paths.
fn validate_path(path: &str) -> Result<&str, String> {
    for component in std::path::Path::new(path).components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err("Path traversal denied: '..' components are forbidden".to_string());
        }
    }
    Ok(path)
}

/// Resolve a file path through the workspace sandbox (if available) or legacy validation.
fn resolve_file_path(raw_path: &str, workspace_root: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(root) = workspace_root {
        crate::workspace_sandbox::resolve_sandbox_path(raw_path, root)
    } else {
        let _ = validate_path(raw_path)?;
        Ok(PathBuf::from(raw_path))
    }
}

async fn tool_file_read(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let resolved = resolve_file_path(raw_path, workspace_root)?;
    tokio::fs::read_to_string(&resolved)
        .await
        .map_err(|e| format!("Failed to read file: {e}"))
}

async fn tool_file_write(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let resolved = resolve_file_path(raw_path, workspace_root)?;
    let content = input["content"]
        .as_str()
        .ok_or("Missing 'content' parameter")?;
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create directories: {e}"))?;
    }
    tokio::fs::write(&resolved, content)
        .await
        .map_err(|e| format!("Failed to write file: {e}"))?;
    Ok(format!(
        "Successfully wrote {} bytes to {}",
        content.len(),
        resolved.display()
    ))
}

async fn tool_file_list(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let resolved = resolve_file_path(raw_path, workspace_root)?;
    let mut entries = tokio::fs::read_dir(&resolved)
        .await
        .map_err(|e| format!("Failed to list directory: {e}"))?;
    let mut files = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read entry: {e}"))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.metadata().await;
        let suffix = match metadata {
            Ok(m) if m.is_dir() => "/",
            _ => "",
        };
        files.push(format!("{name}{suffix}"));
    }
    files.sort();
    Ok(files.join("\n"))
}

// ---------------------------------------------------------------------------
// Patch tool
// ---------------------------------------------------------------------------

async fn tool_apply_patch(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let patch_str = input["patch"].as_str().ok_or("Missing 'patch' parameter")?;
    let root = workspace_root.ok_or("apply_patch requires a workspace root")?;
    let ops = crate::apply_patch::parse_patch(patch_str)?;
    let result = crate::apply_patch::apply_patch(&ops, root).await;
    if result.is_ok() {
        Ok(result.summary())
    } else {
        Err(format!(
            "Patch partially applied: {}. Errors: {}",
            result.summary(),
            result.errors.join("; ")
        ))
    }
}

// ---------------------------------------------------------------------------
// Web tools
// ---------------------------------------------------------------------------

/// Legacy web fetch (no SSRF protection, no readability). Used when WebToolsContext is unavailable.
async fn tool_web_fetch_legacy(input: &serde_json::Value) -> Result<String, String> {
    let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;
    let status = resp.status();
    // Reject responses larger than 10MB to prevent memory exhaustion
    if let Some(len) = resp.content_length() {
        if len > 10 * 1024 * 1024 {
            return Err(format!("Response too large: {len} bytes (max 10MB)"));
        }
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;
    let max_len = 50_000;
    let truncated = if body.len() > max_len {
        format!(
            "{}... [truncated, {} total bytes]",
            crate::str_utils::safe_truncate_str(&body, max_len),
            body.len()
        )
    } else {
        body
    };
    Ok(format!("HTTP {status}\n\n{truncated}"))
}

/// Legacy web search via DuckDuckGo HTML only. Used when WebToolsContext is unavailable.
async fn tool_web_search_legacy(input: &serde_json::Value) -> Result<String, String> {
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
    let max_results = input["max_results"].as_u64().unwrap_or(5) as usize;

    debug!(query, "Executing web search via DuckDuckGo HTML");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let resp = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header("User-Agent", "Mozilla/5.0 (compatible; OpenFangAgent/0.1)")
        .send()
        .await
        .map_err(|e| format!("Search request failed: {e}"))?;

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read search response: {e}"))?;

    // Parse DuckDuckGo HTML results
    let results = parse_ddg_results(&body, max_results);

    if results.is_empty() {
        return Ok(format!("No results found for '{query}'."));
    }

    let mut output = format!("Search results for '{query}':\n\n");
    for (i, (title, url, snippet)) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   URL: {}\n   {}\n\n",
            i + 1,
            title,
            url,
            snippet
        ));
    }

    Ok(output)
}

// ---------------------------------------------------------------------------
// Shell tool
// ---------------------------------------------------------------------------

async fn tool_shell_exec(
    input: &serde_json::Value,
    allowed_env: &[String],
    workspace_root: Option<&Path>,
    exec_policy: Option<&openfang_types::config::ExecPolicy>,
) -> Result<String, String> {
    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;
    // Use LLM-specified timeout, or fall back to exec policy timeout, or default 30s
    let policy_timeout = exec_policy.map(|p| p.timeout_secs).unwrap_or(30);
    let timeout_secs = input["timeout_seconds"].as_u64().unwrap_or(policy_timeout);

    // SECURITY: Determine execution strategy based on exec policy.
    //
    // In Allowlist mode (default): Use direct execution via shlex argv splitting.
    // This avoids invoking a shell interpreter, which eliminates an entire class
    // of injection attacks (encoding tricks, $IFS, glob expansion, etc.).
    //
    // In Full mode: User explicitly opted into unrestricted shell access,
    // so we use sh -c / cmd /C as before.
    let use_direct_exec = exec_policy
        .map(|p| p.mode == openfang_types::config::ExecSecurityMode::Allowlist)
        .unwrap_or(true); // Default to safe mode

    let mut cmd = if use_direct_exec {
        // SAFE PATH: Split command into argv using POSIX shell lexer rules,
        // then execute the binary directly — no shell interpreter involved.
        let argv = shlex::split(command).ok_or_else(|| {
            "Command contains unmatched quotes or invalid shell syntax".to_string()
        })?;
        if argv.is_empty() {
            return Err("Empty command after parsing".to_string());
        }
        let mut c = tokio::process::Command::new(&argv[0]);
        if argv.len() > 1 {
            c.args(&argv[1..]);
        }
        c
    } else {
        // UNSAFE PATH: Full mode — user explicitly opted in to shell interpretation.
        // Shell resolution: prefer sh (Git Bash/MSYS2) on Windows.
        #[cfg(windows)]
        let git_sh: Option<&str> = {
            const SH_PATHS: &[&str] = &[
                "C:\\Program Files\\Git\\usr\\bin\\sh.exe",
                "C:\\Program Files (x86)\\Git\\usr\\bin\\sh.exe",
            ];
            SH_PATHS
                .iter()
                .copied()
                .find(|p| std::path::Path::new(p).exists())
        };
        let (shell, shell_arg) = if cfg!(windows) {
            #[cfg(windows)]
            {
                if let Some(sh) = git_sh {
                    (sh, "-c")
                } else {
                    ("cmd", "/C")
                }
            }
            #[cfg(not(windows))]
            {
                ("sh", "-c")
            }
        } else {
            ("sh", "-c")
        };
        let mut c = tokio::process::Command::new(shell);
        c.arg(shell_arg).arg(command);
        c
    };

    // Set working directory to agent workspace so files are created there
    if let Some(ws) = workspace_root {
        cmd.current_dir(ws);
    }

    // SECURITY: Isolate environment to prevent credential leakage.
    // Hand settings may grant access to specific provider API keys.
    crate::subprocess_sandbox::sandbox_command(&mut cmd, allowed_env);

    // Ensure UTF-8 output on Windows
    #[cfg(windows)]
    cmd.env("PYTHONIOENCODING", "utf-8");

    // Prevent child from inheriting stdin (avoids blocking on Windows)
    cmd.stdin(std::process::Stdio::null());

    let result =
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output()).await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);

            // Truncate very long outputs to prevent memory issues
            let max_output = 100_000;
            let stdout_str = if stdout.len() > max_output {
                format!(
                    "{}...\n[truncated, {} total bytes]",
                    crate::str_utils::safe_truncate_str(&stdout, max_output),
                    stdout.len()
                )
            } else {
                stdout.to_string()
            };
            let stderr_str = if stderr.len() > max_output {
                format!(
                    "{}...\n[truncated, {} total bytes]",
                    crate::str_utils::safe_truncate_str(&stderr, max_output),
                    stderr.len()
                )
            } else {
                stderr.to_string()
            };

            Ok(format!(
                "Exit code: {exit_code}\n\nSTDOUT:\n{stdout_str}\nSTDERR:\n{stderr_str}"
            ))
        }
        Ok(Err(e)) => Err(format!("Failed to execute command: {e}")),
        Err(_) => Err(format!("Command timed out after {timeout_secs}s")),
    }
}

// ---------------------------------------------------------------------------
// Inter-agent tools
// ---------------------------------------------------------------------------

fn require_kernel(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<&Arc<dyn KernelHandle>, String> {
    kernel.ok_or_else(|| {
        "Kernel handle not available. Inter-agent tools require a running kernel.".to_string()
    })
}

async fn tool_agent_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or("Missing 'agent_id' parameter")?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;

    // Check + increment inter-agent call depth
    let current_depth = AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0);
    if current_depth >= MAX_AGENT_CALL_DEPTH {
        return Err(format!(
            "Inter-agent call depth exceeded (max {}). \
             A->B->C chain is too deep. Use the task queue instead.",
            MAX_AGENT_CALL_DEPTH
        ));
    }

    AGENT_CALL_DEPTH
        .scope(std::cell::Cell::new(current_depth + 1), async {
            kh.send_to_agent(agent_id, message).await
        })
        .await
}

async fn tool_agent_spawn(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    parent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let manifest_toml = input["manifest_toml"]
        .as_str()
        .ok_or("Missing 'manifest_toml' parameter")?;
    let (id, name) = kh.spawn_agent(manifest_toml, parent_id).await?;
    Ok(format!(
        "Agent spawned successfully.\n  ID: {id}\n  Name: {name}"
    ))
}

fn tool_agent_list(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agents = kh.list_agents();
    if agents.is_empty() {
        return Ok("No agents currently running.".to_string());
    }
    let mut output = format!("Running agents ({}):\n", agents.len());
    for a in &agents {
        output.push_str(&format!(
            "  - {} (id: {}, state: {}, model: {}:{})\n",
            a.name, a.id, a.state, a.model_provider, a.model_name
        ));
    }
    Ok(output)
}

fn tool_agent_kill(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or("Missing 'agent_id' parameter")?;
    kh.kill_agent(agent_id)?;
    Ok(format!("Agent {agent_id} killed successfully."))
}

// ---------------------------------------------------------------------------
// Shared memory tools
// ---------------------------------------------------------------------------

fn resolve_memory_store_key(input: &serde_json::Value) -> Result<(String, String), String> {
    let raw_key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    let raw_key = raw_key.trim().to_string();

    if is_internal_memory_key(&raw_key) {
        return Err(
            "Reserved internal memory keys cannot be written via memory_store.".to_string(),
        );
    }

    if let Some(namespace) = input.get("namespace").and_then(|v| v.as_str()) {
        if raw_key.contains('.') {
            return Err(
                "Provide either a fully namespaced key or 'namespace' + bare key, not both."
                    .to_string(),
            );
        }

        let namespace = canonicalize_memory_namespace(namespace)?;
        let canonical = canonicalize_user_memory_key(&format!("{namespace}.{raw_key}"))?;
        return Ok((raw_key, canonical));
    }

    let canonical = canonicalize_user_memory_key(&raw_key)?;
    Ok((raw_key, canonical))
}

fn resolve_memory_lookup_keys(input: &serde_json::Value) -> Result<(String, Vec<String>), String> {
    let raw_key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    let raw_key = raw_key.trim().to_string();

    if let Some(namespace) = input.get("namespace").and_then(|v| v.as_str()) {
        if is_internal_memory_key(&raw_key) {
            return Err("Internal memory keys cannot be combined with 'namespace'.".to_string());
        }
        if raw_key.contains('.') {
            return Err(
                "Provide either a fully namespaced key or 'namespace' + bare key, not both."
                    .to_string(),
            );
        }

        let namespace = canonicalize_memory_namespace(namespace)?;
        return Ok((
            raw_key.clone(),
            vec![canonicalize_user_memory_key(&format!(
                "{namespace}.{raw_key}"
            ))?],
        ));
    }

    Ok((raw_key.clone(), memory_lookup_candidates(&raw_key)?))
}

fn memory_entry_exists(kh: &Arc<dyn KernelHandle>, key: &str) -> Result<bool, String> {
    for candidate in memory_lookup_candidates(key)? {
        if kh.memory_recall(&candidate)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn tool_memory_store(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let (requested_key, key) = resolve_memory_store_key(input)?;
    let value = input.get("value").ok_or("Missing 'value' parameter")?;
    let kind = input.get("kind").and_then(|v| v.as_str());
    let tags: Vec<String> = input
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let freshness: Option<MemoryFreshness> = input
        .get("freshness")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| format!("Invalid 'freshness' parameter: {e}"))?;
    let conflict_policy: MemoryConflictPolicy = input
        .get("conflict_policy")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| format!("Invalid 'conflict_policy' parameter: {e}"))?
        .unwrap_or_default();

    if matches!(conflict_policy, MemoryConflictPolicy::SkipIfExists)
        && memory_entry_exists(kh, &key)?
    {
        return Ok(format!(
            "Skipped storing key '{key}' because it already exists and conflict_policy is 'skip_if_exists'."
        ));
    }

    kh.memory_store(&key, value.clone())?;
    let metadata = build_memory_record_metadata(&key, kind, &tags, freshness, "memory_store_tool")?;
    let metadata_key = memory_metadata_key(&key)?;
    kh.memory_store(
        &metadata_key,
        serde_json::to_value(&metadata).map_err(|e| e.to_string())?,
    )?;
    if key == requested_key {
        Ok(format!("Stored value under key '{key}'."))
    } else {
        Ok(format!(
            "Stored value under key '{key}' (normalized from '{requested_key}')."
        ))
    }
}

fn tool_memory_recall(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let (requested_key, candidates) = resolve_memory_lookup_keys(input)?;
    for key in candidates {
        if let Some(val) = kh.memory_recall(&key)? {
            let rendered = serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string());
            if key == requested_key {
                return Ok(rendered);
            }
            return Ok(format!(
                "Recalled key '{key}' (requested '{requested_key}').\n{rendered}"
            ));
        }
    }
    Ok(format!("No value found for key '{requested_key}'."))
}

fn tool_memory_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let namespace = input
        .get("namespace")
        .and_then(|v| v.as_str())
        .map(canonicalize_memory_namespace)
        .transpose()?;
    let prefix = input
        .get("prefix")
        .or_else(|| input.get("query"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let tag_filters = canonicalize_memory_tag_filters(&match input.get("tags") {
        Some(serde_json::Value::Array(tags)) => tags
            .iter()
            .map(|tag| {
                tag.as_str()
                    .map(ToOwned::to_owned)
                    .ok_or("Invalid 'tags' parameter: expected an array of strings.".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(serde_json::Value::String(tag)) => vec![tag.clone()],
        Some(_) => {
            return Err("Invalid 'tags' parameter: expected an array of strings.".to_string());
        }
        None => Vec::new(),
    })?;
    let lifecycle: Option<MemoryLifecycleState> = input
        .get("lifecycle")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| format!("Invalid 'lifecycle' parameter: {e}"))?;
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v.min(100) as usize)
        .or(Some(20));
    let include_values = input
        .get("include_values")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let include_internal = input
        .get("include_internal")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let entries = kh.memory_list(None, Some(500))?;
    let metadata_map = collect_memory_metadata(&entries);
    let mut filtered = Vec::new();
    for (key, value) in entries {
        if is_memory_metadata_key(&key) {
            continue;
        }
        let internal = is_internal_memory_key(&key);
        if internal && !include_internal {
            continue;
        }
        if let Some(namespace) = namespace.as_deref() {
            if memory_key_namespace(&key).as_deref() != Some(namespace) {
                continue;
            }
        }
        if let Some(prefix) = prefix {
            if !memory_key_matches_prefix(&key, prefix)? {
                continue;
            }
        }
        let metadata = metadata_map.get(&key);
        if !tag_filters.is_empty()
            && !metadata
                .map(|metadata| memory_tags_match(&metadata.tags, &tag_filters))
                .unwrap_or(false)
        {
            continue;
        }
        let lifecycle_snapshot = metadata_map
            .get(&key)
            .map(|metadata| memory_lifecycle_snapshot(metadata, chrono::Utc::now()));
        if let Some(lifecycle) = lifecycle {
            if lifecycle_snapshot.as_ref().map(|s| s.state) != Some(lifecycle) {
                continue;
            }
        }
        filtered.push((key, value));
    }

    filtered.sort_by(|a, b| a.0.cmp(&b.0));
    if let Some(limit) = limit {
        filtered.truncate(limit);
    }

    if filtered.is_empty() {
        return Ok(match prefix {
            Some(prefix) => format!("No memory entries found with prefix '{prefix}'."),
            None => match namespace {
                Some(namespace) => {
                    format!("No memory entries found in namespace '{namespace}'.")
                }
                None => "No memory entries found.".to_string(),
            },
        });
    }

    let rows: Vec<serde_json::Value> = filtered
        .into_iter()
        .map(|(key, value)| {
            let namespace = memory_key_namespace(&key);
            let internal = is_internal_memory_key(&key);
            let metadata = metadata_map.get(&key);
            let lifecycle = metadata.map(|m| memory_lifecycle_snapshot(m, chrono::Utc::now()));
            if include_values {
                serde_json::json!({
                    "key": key,
                    "namespace": namespace,
                    "internal": internal,
                    "governed": metadata.is_some(),
                    "kind": metadata.as_ref().map(|m| m.kind.clone()),
                    "tags": metadata.as_ref().map(|m| m.tags.clone()).unwrap_or_default(),
                    "freshness": metadata.as_ref().map(|m| m.freshness.clone()),
                    "source": metadata.as_ref().map(|m| m.source.clone()),
                    "updated_at": metadata.as_ref().map(|m| m.updated_at.to_rfc3339()),
                    "lifecycle_state": lifecycle.as_ref().map(|snapshot| snapshot.state),
                    "review_at": lifecycle.as_ref().map(|snapshot| snapshot.review_at.to_rfc3339()),
                    "expires_at": lifecycle.as_ref().and_then(|snapshot| snapshot.expires_at.map(|ts| ts.to_rfc3339())),
                    "promotion_candidate": lifecycle.as_ref().map(|snapshot| snapshot.promotion_candidate).unwrap_or(false),
                    "value": value
                })
            } else {
                serde_json::json!({
                    "key": key,
                    "namespace": namespace,
                    "internal": internal,
                    "governed": metadata.is_some(),
                    "kind": metadata.as_ref().map(|m| m.kind.clone()),
                    "tags": metadata.as_ref().map(|m| m.tags.clone()).unwrap_or_default(),
                    "freshness": metadata.as_ref().map(|m| m.freshness.clone()),
                    "source": metadata.as_ref().map(|m| m.source.clone()),
                    "updated_at": metadata.as_ref().map(|m| m.updated_at.to_rfc3339()),
                    "lifecycle_state": lifecycle.as_ref().map(|snapshot| snapshot.state),
                    "review_at": lifecycle.as_ref().map(|snapshot| snapshot.review_at.to_rfc3339()),
                    "expires_at": lifecycle.as_ref().and_then(|snapshot| snapshot.expires_at.map(|ts| ts.to_rfc3339())),
                    "promotion_candidate": lifecycle.as_ref().map(|snapshot| snapshot.promotion_candidate).unwrap_or(false)
                })
            }
        })
        .collect();

    serde_json::to_string_pretty(&rows).map_err(|e| format!("Failed to serialize memory list: {e}"))
}

fn tool_memory_cleanup(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let apply = input.get("apply").and_then(|v| v.as_bool()).unwrap_or(false);
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v.min(200) as usize)
        .unwrap_or(200);

    let mut plan = plan_memory_cleanup(&kh.memory_list(None, Some(500))?);
    if plan.findings.len() > limit {
        plan.findings.truncate(limit);
    }

    let mut migrated_legacy_keys = 0usize;
    let mut deleted_legacy_keys = 0usize;
    let mut deleted_orphan_metadata = 0usize;
    let mut backfilled_metadata = 0usize;

    if apply {
        for finding in &plan.findings {
            match finding.action {
                MemoryCleanupAction::MigrateLegacyKey => {
                    let Some(canonical_key) = finding.canonical_key.as_deref() else {
                        continue;
                    };
                    let Some(legacy_value) = kh.memory_recall(&finding.key)? else {
                        continue;
                    };
                    kh.memory_store(canonical_key, legacy_value)?;
                    let metadata = build_memory_record_metadata(
                        canonical_key,
                        None,
                        &[],
                        None,
                        MEMORY_CLEANUP_SOURCE,
                    )?;
                    let metadata_key = memory_metadata_key(canonical_key)?;
                    kh.memory_store(
                        &metadata_key,
                        serde_json::to_value(&metadata).map_err(|e| e.to_string())?,
                    )?;
                    kh.memory_delete(&finding.key)?;
                    migrated_legacy_keys += 1;
                }
                MemoryCleanupAction::DeleteLegacyKey => {
                    kh.memory_delete(&finding.key)?;
                    deleted_legacy_keys += 1;
                }
                MemoryCleanupAction::DeleteOrphanMetadata => {
                    let Some(metadata_key) = finding.metadata_key.as_deref() else {
                        continue;
                    };
                    kh.memory_delete(metadata_key)?;
                    deleted_orphan_metadata += 1;
                }
                MemoryCleanupAction::BackfillMetadata => {
                    let Some(canonical_key) = finding.canonical_key.as_deref() else {
                        continue;
                    };
                    let Some(metadata_key) = finding.metadata_key.as_deref() else {
                        continue;
                    };
                    let metadata = build_memory_record_metadata(
                        canonical_key,
                        None,
                        &[],
                        None,
                        MEMORY_CLEANUP_SOURCE,
                    )?;
                    kh.memory_store(
                        metadata_key,
                        serde_json::to_value(&metadata).map_err(|e| e.to_string())?,
                    )?;
                    backfilled_metadata += 1;
                }
            }
        }
    }

    let result = serde_json::json!({
        "apply": apply,
        "planned": plan.findings.len(),
        "counts": {
            "migrate_legacy_key": plan.findings.iter().filter(|finding| finding.action == MemoryCleanupAction::MigrateLegacyKey).count(),
            "delete_legacy_key": plan.findings.iter().filter(|finding| finding.action == MemoryCleanupAction::DeleteLegacyKey).count(),
            "delete_orphan_metadata": plan.findings.iter().filter(|finding| finding.action == MemoryCleanupAction::DeleteOrphanMetadata).count(),
            "backfill_metadata": plan.findings.iter().filter(|finding| finding.action == MemoryCleanupAction::BackfillMetadata).count(),
        },
        "applied_counts": {
            "migrated_legacy_keys": migrated_legacy_keys,
            "deleted_legacy_keys": deleted_legacy_keys,
            "deleted_orphan_metadata": deleted_orphan_metadata,
            "backfilled_metadata": backfilled_metadata,
        },
        "findings": plan.findings,
    });

    serde_json::to_string_pretty(&result).map_err(|e| format!("Failed to serialize cleanup result: {e}"))
}

fn tool_memory_autoconverge(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let workspace_root = workspace_root.ok_or("memory_autoconverge requires a workspace root")?;
    let apply = input.get("apply").and_then(|v| v.as_bool()).unwrap_or(false);
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v.min(64) as usize)
        .unwrap_or(24);
    let memory_path = workspace_root.join("MEMORY.md");
    let current_content = std::fs::read_to_string(&memory_path).unwrap_or_default();
    let plan = build_memory_autoconverge_plan(
        &current_content,
        &kh.memory_list(None, Some(500))?,
        chrono::Utc::now(),
        limit,
    );

    if apply && !plan.summary.can_apply {
        let result = serde_json::json!({
            "status": "blocked",
            "error": "Autoconverge is blocked until shared memory governance cleanup issues are repaired",
            "path": memory_path.display().to_string(),
            "summary": plan.summary,
            "candidates": plan.candidates,
            "stale_review": plan.stale_review,
            "cleanup_blockers": plan.cleanup_blockers,
            "current_content": plan.current_content,
            "proposed_content": plan.proposed_content,
        });
        return serde_json::to_string_pretty(&result)
            .map_err(|e| format!("Failed to serialize autoconverge result: {e}"));
    }

    if apply && plan.summary.changed {
        const MAX_FILE_SIZE: usize = 32_768;
        if plan.proposed_content.len() > MAX_FILE_SIZE {
            return Err("Autoconverged MEMORY.md would exceed 32KB".to_string());
        }

        let tmp_path = workspace_root.join(".MEMORY.md.tmp");
        std::fs::write(&tmp_path, &plan.proposed_content)
            .map_err(|e| format!("Write failed: {e}"))?;
        if let Err(e) = std::fs::rename(&tmp_path, &memory_path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(format!("Rename failed: {e}"));
        }

        if let Some(agent_id) = caller_agent_id {
            kh.record_audit_event(
                agent_id,
                crate::audit::AuditAction::ConfigChange,
                "memory autoconverge apply",
                &format!(
                    "managed_entries={} cleanup_blockers={} stale_review={}",
                    plan.summary.managed_entries,
                    plan.summary.cleanup_blockers,
                    plan.summary.stale_review
                ),
            )?;
        }
    }

    let result = serde_json::json!({
        "status": if apply { "applied" } else { "audit" },
        "path": memory_path.display().to_string(),
        "summary": plan.summary,
        "candidates": plan.candidates,
        "stale_review": plan.stale_review,
        "cleanup_blockers": plan.cleanup_blockers,
        "current_content": plan.current_content,
        "proposed_content": plan.proposed_content,
    });

    serde_json::to_string_pretty(&result)
        .map_err(|e| format!("Failed to serialize autoconverge result: {e}"))
}

// ---------------------------------------------------------------------------
// Collaboration tools
// ---------------------------------------------------------------------------

fn tool_agent_find(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
    let agents = kh.find_agents(query);
    if agents.is_empty() {
        return Ok(format!("No agents found matching '{query}'."));
    }
    let result: Vec<serde_json::Value> = agents
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "name": a.name,
                "state": a.state,
                "description": a.description,
                "tags": a.tags,
                "tools": a.tools,
                "model": format!("{}:{}", a.model_provider, a.model_name),
            })
        })
        .collect();
    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

async fn tool_task_post(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let title = input["title"].as_str().ok_or("Missing 'title' parameter")?;
    let description = input["description"]
        .as_str()
        .ok_or("Missing 'description' parameter")?;
    let assigned_to = input["assigned_to"].as_str();
    let task_id = kh
        .task_post(title, description, assigned_to, caller_agent_id)
        .await?;
    Ok(format!("Task created with ID: {task_id}"))
}

async fn tool_task_claim(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.unwrap_or("");
    match kh.task_claim(agent_id).await? {
        Some(task) => {
            serde_json::to_string_pretty(&task).map_err(|e| format!("Serialize error: {e}"))
        }
        None => Ok("No tasks available.".to_string()),
    }
}

async fn tool_task_complete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let task_id = input["task_id"]
        .as_str()
        .ok_or("Missing 'task_id' parameter")?;
    let result = input["result"]
        .as_str()
        .ok_or("Missing 'result' parameter")?;
    kh.task_complete(task_id, result).await?;
    Ok(format!("Task {task_id} marked as completed."))
}

async fn tool_task_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let status = input["status"].as_str();
    let tasks = kh.task_list(status).await?;
    if tasks.is_empty() {
        return Ok("No tasks found.".to_string());
    }
    serde_json::to_string_pretty(&tasks).map_err(|e| format!("Serialize error: {e}"))
}

async fn tool_event_publish(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let event_type = input["event_type"]
        .as_str()
        .ok_or("Missing 'event_type' parameter")?;
    let payload = input
        .get("payload")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    kh.publish_event(event_type, payload).await?;
    Ok(format!("Event '{event_type}' published successfully."))
}

// ---------------------------------------------------------------------------
// Knowledge graph tools
// ---------------------------------------------------------------------------

fn parse_entity_type(s: &str) -> openfang_types::memory::EntityType {
    use openfang_types::memory::EntityType;
    match s.to_lowercase().as_str() {
        "person" => EntityType::Person,
        "organization" | "org" => EntityType::Organization,
        "project" => EntityType::Project,
        "concept" => EntityType::Concept,
        "event" => EntityType::Event,
        "location" => EntityType::Location,
        "document" | "doc" => EntityType::Document,
        "tool" => EntityType::Tool,
        other => EntityType::Custom(other.to_string()),
    }
}

fn parse_relation_type(s: &str) -> openfang_types::memory::RelationType {
    use openfang_types::memory::RelationType;
    match s.to_lowercase().as_str() {
        "works_at" | "worksat" => RelationType::WorksAt,
        "knows_about" | "knowsabout" | "knows" => RelationType::KnowsAbout,
        "related_to" | "relatedto" | "related" => RelationType::RelatedTo,
        "depends_on" | "dependson" | "depends" => RelationType::DependsOn,
        "owned_by" | "ownedby" => RelationType::OwnedBy,
        "created_by" | "createdby" => RelationType::CreatedBy,
        "located_in" | "locatedin" => RelationType::LocatedIn,
        "part_of" | "partof" => RelationType::PartOf,
        "uses" => RelationType::Uses,
        "produces" => RelationType::Produces,
        other => RelationType::Custom(other.to_string()),
    }
}

async fn tool_knowledge_add_entity(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let entity_type_str = input["entity_type"]
        .as_str()
        .ok_or("Missing 'entity_type' parameter")?;
    let properties = input
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    let entity = openfang_types::memory::Entity {
        id: String::new(), // kernel/store assigns a real ID
        entity_type: parse_entity_type(entity_type_str),
        name: name.to_string(),
        properties,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    let id = kh.knowledge_add_entity(entity).await?;
    Ok(format!("Entity '{name}' added with ID: {id}"))
}

async fn tool_knowledge_add_relation(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let source = input["source"]
        .as_str()
        .ok_or("Missing 'source' parameter")?;
    let relation_str = input["relation"]
        .as_str()
        .ok_or("Missing 'relation' parameter")?;
    let target = input["target"]
        .as_str()
        .ok_or("Missing 'target' parameter")?;
    let confidence = input["confidence"].as_f64().unwrap_or(1.0) as f32;
    let properties = input
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    let relation = openfang_types::memory::Relation {
        source: source.to_string(),
        relation: parse_relation_type(relation_str),
        target: target.to_string(),
        properties,
        confidence,
        created_at: chrono::Utc::now(),
    };

    let id = kh.knowledge_add_relation(relation).await?;
    Ok(format!(
        "Relation '{source}' --[{relation_str}]--> '{target}' added with ID: {id}"
    ))
}

async fn tool_knowledge_query(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let source = input["source"].as_str().map(|s| s.to_string());
    let target = input["target"].as_str().map(|s| s.to_string());
    let relation = input["relation"].as_str().map(parse_relation_type);
    let max_depth = input["max_depth"].as_u64().unwrap_or(1) as u32;

    let pattern = openfang_types::memory::GraphPattern {
        source,
        relation,
        target,
        max_depth,
    };

    let matches = kh.knowledge_query(pattern).await?;
    if matches.is_empty() {
        return Ok("No matching knowledge graph entries found.".to_string());
    }

    let mut output = format!("Found {} match(es):\n", matches.len());
    for m in &matches {
        output.push_str(&format!(
            "\n  {} ({:?}) --[{:?} ({:.0}%)]--> {} ({:?})",
            m.source.name,
            m.source.entity_type,
            m.relation.relation,
            m.relation.confidence * 100.0,
            m.target.name,
            m.target.entity_type,
        ));
    }
    Ok(output)
}

// ---------------------------------------------------------------------------
// Scheduling tools
// ---------------------------------------------------------------------------

/// Parse a natural language schedule into a cron expression.
fn parse_schedule_to_cron(input: &str) -> Result<String, String> {
    let input = input.trim().to_lowercase();

    // If it already looks like a cron expression (5 space-separated fields), pass through
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() == 5
        && parts
            .iter()
            .all(|p| p.chars().all(|c| c.is_ascii_digit() || "*/,-".contains(c)))
    {
        return Ok(input);
    }

    // Natural language patterns
    if let Some(rest) = input.strip_prefix("every ") {
        if rest == "minute" || rest == "1 minute" {
            return Ok("* * * * *".to_string());
        }
        if let Some(mins) = rest.strip_suffix(" minutes") {
            let n: u32 = mins
                .trim()
                .parse()
                .map_err(|_| format!("Invalid number in '{input}'"))?;
            if n == 0 || n > 59 {
                return Err(format!("Minutes must be 1-59, got {n}"));
            }
            return Ok(format!("*/{n} * * * *"));
        }
        if rest == "hour" || rest == "1 hour" {
            return Ok("0 * * * *".to_string());
        }
        if let Some(hrs) = rest.strip_suffix(" hours") {
            let n: u32 = hrs
                .trim()
                .parse()
                .map_err(|_| format!("Invalid number in '{input}'"))?;
            if n == 0 || n > 23 {
                return Err(format!("Hours must be 1-23, got {n}"));
            }
            return Ok(format!("0 */{n} * * *"));
        }
        if rest == "day" || rest == "1 day" {
            return Ok("0 0 * * *".to_string());
        }
        if rest == "week" || rest == "1 week" {
            return Ok("0 0 * * 0".to_string());
        }
    }

    // "daily at Xam/pm"
    if let Some(time_str) = input.strip_prefix("daily at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * *"));
    }

    // "weekdays at Xam/pm"
    if let Some(time_str) = input.strip_prefix("weekdays at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * 1-5"));
    }

    // "weekends at Xam/pm"
    if let Some(time_str) = input.strip_prefix("weekends at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * 0,6"));
    }

    // "hourly" / "daily" / "weekly" / "monthly"
    match input.as_str() {
        "hourly" => return Ok("0 * * * *".to_string()),
        "daily" => return Ok("0 0 * * *".to_string()),
        "weekly" => return Ok("0 0 * * 0".to_string()),
        "monthly" => return Ok("0 0 1 * *".to_string()),
        _ => {}
    }

    Err(format!(
        "Could not parse schedule '{input}'. Try: 'every 5 minutes', 'daily at 9am', 'weekdays at 6pm', or a cron expression like '0 */5 * * *'"
    ))
}

/// Parse a time string like "9am", "6pm", "14:00", "9:30am" into an hour (0-23).
fn parse_time_to_hour(s: &str) -> Result<u32, String> {
    let s = s.trim().to_lowercase();

    // Handle "9am", "6pm", "12pm", "12am"
    if let Some(h) = s.strip_suffix("am") {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        return match hour {
            12 => Ok(0),
            1..=11 => Ok(hour),
            _ => Err(format!("Invalid hour: {hour}")),
        };
    }
    if let Some(h) = s.strip_suffix("pm") {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        return match hour {
            12 => Ok(12),
            1..=11 => Ok(hour + 12),
            _ => Err(format!("Invalid hour: {hour}")),
        };
    }

    // Handle "14:00" or "9:30"
    if let Some((h, _m)) = s.split_once(':') {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        if hour > 23 {
            return Err(format!("Hour must be 0-23, got {hour}"));
        }
        return Ok(hour);
    }

    // Plain number
    let hour: u32 = s.parse().map_err(|_| format!("Invalid time: {s}"))?;
    if hour > 23 {
        return Err(format!("Hour must be 0-23, got {hour}"));
    }
    Ok(hour)
}

const SCHEDULES_KEY: &str = "__openfang_schedules";

async fn tool_schedule_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let description = input["description"]
        .as_str()
        .ok_or("Missing 'description' parameter")?;
    let schedule_str = input["schedule"]
        .as_str()
        .ok_or("Missing 'schedule' parameter")?;
    let agent = input["agent"].as_str().unwrap_or("");

    let cron_expr = parse_schedule_to_cron(schedule_str)?;
    let schedule_id = uuid::Uuid::new_v4().to_string();

    let entry = serde_json::json!({
        "id": schedule_id,
        "description": description,
        "schedule_input": schedule_str,
        "cron": cron_expr,
        "agent": agent,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "enabled": true,
    });

    // Load existing schedules from shared memory
    let mut schedules: Vec<serde_json::Value> = match kh.memory_recall(SCHEDULES_KEY)? {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => Vec::new(),
    };

    schedules.push(entry);
    kh.memory_store(SCHEDULES_KEY, serde_json::Value::Array(schedules))?;

    Ok(format!(
        "Schedule created:\n  ID: {schedule_id}\n  Description: {description}\n  Cron: {cron_expr}\n  Original: {schedule_str}"
    ))
}

async fn tool_schedule_list(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
    let kh = require_kernel(kernel)?;

    let schedules: Vec<serde_json::Value> = match kh.memory_recall(SCHEDULES_KEY)? {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => Vec::new(),
    };

    if schedules.is_empty() {
        return Ok("No scheduled tasks.".to_string());
    }

    let mut output = format!("Scheduled tasks ({}):\n\n", schedules.len());
    for s in &schedules {
        let enabled = s["enabled"].as_bool().unwrap_or(true);
        let status = if enabled { "active" } else { "paused" };
        output.push_str(&format!(
            "  [{status}] {} — {}\n    Cron: {} | Agent: {}\n    Created: {}\n\n",
            s["id"].as_str().unwrap_or("?"),
            s["description"].as_str().unwrap_or("?"),
            s["cron"].as_str().unwrap_or("?"),
            s["agent"].as_str().unwrap_or("(self)"),
            s["created_at"].as_str().unwrap_or("?"),
        ));
    }

    Ok(output)
}

async fn tool_schedule_delete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;

    let mut schedules: Vec<serde_json::Value> = match kh.memory_recall(SCHEDULES_KEY)? {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => Vec::new(),
    };

    let before = schedules.len();
    schedules.retain(|s| s["id"].as_str() != Some(id));

    if schedules.len() == before {
        return Err(format!("Schedule '{id}' not found."));
    }

    kh.memory_store(SCHEDULES_KEY, serde_json::Value::Array(schedules))?;
    Ok(format!("Schedule '{id}' deleted."))
}

// ---------------------------------------------------------------------------
// Cron scheduling tools (delegated to kernel via KernelHandle trait)
// ---------------------------------------------------------------------------

async fn tool_cron_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for cron_create")?;
    kh.cron_create(agent_id, input.clone()).await
}

async fn tool_cron_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for cron_list")?;
    let jobs = kh.cron_list(agent_id).await?;
    serde_json::to_string_pretty(&jobs).map_err(|e| format!("Failed to serialize cron jobs: {e}"))
}

async fn tool_cron_cancel(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let job_id = input["job_id"]
        .as_str()
        .ok_or("Missing 'job_id' parameter")?;
    kh.cron_cancel(job_id).await?;
    Ok(format!("Cron job '{job_id}' cancelled."))
}

// ---------------------------------------------------------------------------
// Channel send tool (proactive outbound messaging via configured adapters)
// ---------------------------------------------------------------------------

async fn tool_channel_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;

    let channel = input["channel"]
        .as_str()
        .ok_or("Missing 'channel' parameter")?
        .trim()
        .to_lowercase();
    let recipient_input = input["recipient"]
        .as_str()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    // If recipient is empty, resolve from channel's default_chat_id config.
    let recipient = if recipient_input.is_empty() {
        let default_id = kh.get_channel_default_recipient(&channel).await;
        match default_id {
            Some(id) => id,
            None => return Err(format!(
                "Missing 'recipient' parameter. Set default_chat_id in [channels.{channel}] config \
                 or pass recipient explicitly."
            )),
        }
    } else {
        recipient_input
    };
    let recipient = recipient.as_str();

    let thread_id = input["thread_id"].as_str().filter(|s| !s.is_empty());

    // Check for media content (image_url, file_url, or file_path)
    let image_url = input["image_url"].as_str().filter(|s| !s.is_empty());
    let file_url = input["file_url"].as_str().filter(|s| !s.is_empty());
    let file_path = input["file_path"].as_str().filter(|s| !s.is_empty());

    if let Some(url) = image_url {
        let caption = input["message"].as_str().filter(|s| !s.is_empty());
        return kh
            .send_channel_media(&channel, recipient, "image", url, caption, None, thread_id)
            .await;
    }

    if let Some(url) = file_url {
        let caption = input["message"].as_str().filter(|s| !s.is_empty());
        let filename = input["filename"].as_str();
        return kh
            .send_channel_media(&channel, recipient, "file", url, caption, filename, thread_id)
            .await;
    }

    // Local file attachment: read from disk and send as FileData
    if let Some(raw_path) = file_path {
        let resolved = resolve_file_path(raw_path, workspace_root)?;
        let data = tokio::fs::read(&resolved)
            .await
            .map_err(|e| format!("Failed to read file '{}': {e}", resolved.display()))?;

        // Derive filename from the path if not explicitly provided
        let filename = input["filename"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                resolved
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file")
                    .to_string()
            });

        // Determine MIME type from extension
        let ext = resolved
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mime_type = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "pdf" => "application/pdf",
            "txt" => "text/plain",
            "csv" => "text/csv",
            "json" => "application/json",
            "xml" => "application/xml",
            "zip" => "application/zip",
            "gz" | "gzip" => "application/gzip",
            "tar" => "application/x-tar",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            "mp4" => "video/mp4",
            "doc" => "application/msword",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "xls" => "application/vnd.ms-excel",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            _ => "application/octet-stream",
        };

        return kh
            .send_channel_file_data(
                &channel, recipient, data, &filename, mime_type, thread_id,
            )
            .await;
    }

    // Text-only message
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter (required for text messages)")?;

    if message.is_empty() {
        return Err("Message cannot be empty".to_string());
    }

    // For email channels, validate email format and prepend subject
    let final_message = if channel == "email" {
        if !recipient.contains('@') || !recipient.contains('.') {
            return Err(format!("Invalid email address: '{recipient}'"));
        }
        if let Some(subject) = input["subject"].as_str() {
            if !subject.is_empty() {
                format!("Subject: {subject}\n\n{message}")
            } else {
                message.to_string()
            }
        } else {
            message.to_string()
        }
    } else {
        message.to_string()
    };

    kh.send_channel_message(&channel, recipient, &final_message, thread_id)
        .await
}

// ---------------------------------------------------------------------------
// Hand tools (delegated to kernel via KernelHandle trait)
// ---------------------------------------------------------------------------

async fn tool_hand_list(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let hands = kh.hand_list().await?;

    if hands.is_empty() {
        return Ok(
            "No Hands available. Install hands to enable curated autonomous packages.".to_string(),
        );
    }

    let mut lines = vec!["Available Hands:".to_string(), String::new()];
    for h in &hands {
        let icon = h["icon"].as_str().unwrap_or("");
        let name = h["name"].as_str().unwrap_or("?");
        let id = h["id"].as_str().unwrap_or("?");
        let status = h["status"].as_str().unwrap_or("unknown");
        let desc = h["description"].as_str().unwrap_or("");

        let status_marker = match status {
            "Active" => "[ACTIVE]",
            "Paused" => "[PAUSED]",
            _ => "[available]",
        };

        lines.push(format!("{} {} ({}) {}", icon, name, id, status_marker));
        if !desc.is_empty() {
            lines.push(format!("  {}", desc));
        }
        if let Some(iid) = h["instance_id"].as_str() {
            lines.push(format!("  Instance: {}", iid));
        }
        lines.push(String::new());
    }

    Ok(lines.join("\n"))
}

async fn tool_hand_activate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let hand_id = input["hand_id"]
        .as_str()
        .ok_or("Missing 'hand_id' parameter")?;
    let config: std::collections::HashMap<String, serde_json::Value> =
        if let Some(obj) = input["config"].as_object() {
            obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        } else {
            std::collections::HashMap::new()
        };

    let result = kh.hand_activate(hand_id, config).await?;

    let instance_id = result["instance_id"].as_str().unwrap_or("?");
    let agent_name = result["agent_name"].as_str().unwrap_or("?");
    let status = result["status"].as_str().unwrap_or("?");

    Ok(format!(
        "Hand '{}' activated!\n  Instance: {}\n  Agent: {} ({})",
        hand_id, instance_id, agent_name, status
    ))
}

async fn tool_hand_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let hand_id = input["hand_id"]
        .as_str()
        .ok_or("Missing 'hand_id' parameter")?;

    let result = kh.hand_status(hand_id).await?;

    let icon = result["icon"].as_str().unwrap_or("");
    let name = result["name"].as_str().unwrap_or(hand_id);
    let status = result["status"].as_str().unwrap_or("unknown");
    let instance_id = result["instance_id"].as_str().unwrap_or("?");
    let agent_name = result["agent_name"].as_str().unwrap_or("?");
    let activated = result["activated_at"].as_str().unwrap_or("?");

    Ok(format!(
        "{} {} — {}\n  Instance: {}\n  Agent: {}\n  Activated: {}",
        icon, name, status, instance_id, agent_name, activated
    ))
}

async fn tool_hand_deactivate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let instance_id = input["instance_id"]
        .as_str()
        .ok_or("Missing 'instance_id' parameter")?;
    kh.hand_deactivate(instance_id).await?;
    Ok(format!("Hand instance '{}' deactivated.", instance_id))
}

// ---------------------------------------------------------------------------
// A2A outbound tools (cross-instance agent communication)
// ---------------------------------------------------------------------------

/// Discover an external A2A agent by fetching its agent card.
async fn tool_a2a_discover(input: &serde_json::Value) -> Result<String, String> {
    let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;

    // SSRF protection: block private/metadata IPs
    if crate::web_fetch::check_ssrf(url).is_err() {
        return Err("SSRF blocked: URL resolves to a private or metadata address".to_string());
    }

    let client = crate::a2a::A2aClient::new();
    let card = client.discover(url).await?;

    serde_json::to_string_pretty(&card).map_err(|e| format!("Serialization error: {e}"))
}

/// Send a task to an external A2A agent.
async fn tool_a2a_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;

    // Resolve agent URL: either directly provided or looked up by name
    let url = if let Some(url) = input["agent_url"].as_str() {
        // SSRF protection
        if crate::web_fetch::check_ssrf(url).is_err() {
            return Err("SSRF blocked: URL resolves to a private or metadata address".to_string());
        }
        url.to_string()
    } else if let Some(name) = input["agent_name"].as_str() {
        kh.get_a2a_agent_url(name)
            .ok_or_else(|| format!("No known A2A agent with name '{name}'. Use a2a_discover first or provide agent_url directly."))?
    } else {
        return Err("Missing 'agent_url' or 'agent_name' parameter".to_string());
    };

    let session_id = input["session_id"].as_str();
    let client = crate::a2a::A2aClient::new();
    let task = client.send_task(&url, message, session_id).await?;

    serde_json::to_string_pretty(&task).map_err(|e| format!("Serialization error: {e}"))
}

// ---------------------------------------------------------------------------
// Image analysis tool
// ---------------------------------------------------------------------------

async fn tool_image_analyze(input: &serde_json::Value) -> Result<String, String> {
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let prompt = input["prompt"].as_str().unwrap_or("");

    let data = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Failed to read image '{path}': {e}"))?;

    let file_size = data.len();

    // Detect image format from magic bytes
    let format = detect_image_format(&data);

    // Extract dimensions for common formats
    let dimensions = extract_image_dimensions(&data, &format);

    // Base64-encode (truncate for very large images in the response)
    let base64_preview = if file_size <= 512 * 1024 {
        // Under 512KB — include full base64
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&data)
    } else {
        // Over 512KB — include first 64KB preview
        use base64::Engine;
        let preview_bytes = &data[..64 * 1024];
        format!(
            "{}... [truncated, {} total bytes]",
            base64::engine::general_purpose::STANDARD.encode(preview_bytes),
            file_size
        )
    };

    let mut result = serde_json::json!({
        "path": path,
        "format": format,
        "file_size_bytes": file_size,
        "file_size_human": format_file_size(file_size),
    });

    if let Some((w, h)) = dimensions {
        result["width"] = serde_json::json!(w);
        result["height"] = serde_json::json!(h);
    }

    if !prompt.is_empty() {
        result["prompt"] = serde_json::json!(prompt);
        result["note"] = serde_json::json!(
            "Vision analysis requires a vision-capable LLM. The base64 data is included for downstream processing."
        );
    }

    result["base64_preview"] = serde_json::json!(base64_preview);

    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

/// Detect image format from magic bytes.
fn detect_image_format(data: &[u8]) -> String {
    if data.len() < 4 {
        return "unknown".to_string();
    }
    if data.starts_with(b"\x89PNG") {
        "png".to_string()
    } else if data.starts_with(b"\xFF\xD8\xFF") {
        "jpeg".to_string()
    } else if data.starts_with(b"GIF8") {
        "gif".to_string()
    } else if data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"WEBP" {
        "webp".to_string()
    } else if data.starts_with(b"BM") {
        "bmp".to_string()
    } else if data.starts_with(b"\x00\x00\x01\x00") {
        "ico".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Extract image dimensions from common formats.
fn extract_image_dimensions(data: &[u8], format: &str) -> Option<(u32, u32)> {
    match format {
        "png" => {
            // PNG: IHDR chunk starts at byte 16, width at 16-19, height at 20-23
            if data.len() >= 24 {
                let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
                let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
                Some((w, h))
            } else {
                None
            }
        }
        "gif" => {
            // GIF: width at bytes 6-7, height at bytes 8-9 (little-endian)
            if data.len() >= 10 {
                let w = u16::from_le_bytes([data[6], data[7]]) as u32;
                let h = u16::from_le_bytes([data[8], data[9]]) as u32;
                Some((w, h))
            } else {
                None
            }
        }
        "bmp" => {
            // BMP: width at bytes 18-21, height at bytes 22-25 (little-endian)
            if data.len() >= 26 {
                let w = u32::from_le_bytes([data[18], data[19], data[20], data[21]]);
                let h = u32::from_le_bytes([data[22], data[23], data[24], data[25]]);
                Some((w, h))
            } else {
                None
            }
        }
        "jpeg" => {
            // JPEG: scan for SOF0 marker (0xFF 0xC0) to find dimensions
            extract_jpeg_dimensions(data)
        }
        _ => None,
    }
}

/// Extract JPEG dimensions by scanning for SOF markers.
fn extract_jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2; // Skip SOI marker
    while i + 1 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        // SOF0-SOF3 markers contain dimensions
        if (0xC0..=0xC3).contains(&marker) && i + 9 < data.len() {
            let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
            let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
            return Some((w, h));
        }
        if i + 3 < data.len() {
            let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            i += 2 + seg_len;
        } else {
            break;
        }
    }
    None
}

/// Format file size in human-readable form.
fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ---------------------------------------------------------------------------
// Location tool
// ---------------------------------------------------------------------------

async fn tool_location_get() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    // Use ip-api.com (free, no API key, JSON response)
    let resp = client
        .get("https://ip-api.com/json/?fields=status,message,country,regionName,city,zip,lat,lon,timezone,isp,query")
        .header("User-Agent", "OpenFang/0.1")
        .send()
        .await
        .map_err(|e| format!("Location request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Location API returned {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse location response: {e}"))?;

    if body["status"].as_str() != Some("success") {
        let msg = body["message"].as_str().unwrap_or("Unknown error");
        return Err(format!("Location lookup failed: {msg}"));
    }

    let result = serde_json::json!({
        "lat": body["lat"],
        "lon": body["lon"],
        "city": body["city"],
        "region": body["regionName"],
        "country": body["country"],
        "zip": body["zip"],
        "timezone": body["timezone"],
        "isp": body["isp"],
        "ip": body["query"],
    });

    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// System time tool
// ---------------------------------------------------------------------------

/// Return current date, time, timezone, and Unix epoch.
fn tool_system_time() -> String {
    let now_utc = chrono::Utc::now();
    let now_local = chrono::Local::now();
    let result = serde_json::json!({
        "utc": now_utc.to_rfc3339(),
        "local": now_local.to_rfc3339(),
        "unix_epoch": now_utc.timestamp(),
        "timezone": now_local.format("%Z").to_string(),
        "utc_offset": now_local.format("%:z").to_string(),
        "date": now_local.format("%Y-%m-%d").to_string(),
        "time": now_local.format("%H:%M:%S").to_string(),
        "day_of_week": now_local.format("%A").to_string(),
    });
    serde_json::to_string_pretty(&result).unwrap_or_else(|_| now_utc.to_rfc3339())
}

// ---------------------------------------------------------------------------
// Media understanding tools
// ---------------------------------------------------------------------------

/// Describe an image using a vision-capable LLM provider.
async fn tool_media_describe(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
) -> Result<String, String> {
    use base64::Engine;
    let engine = media_engine.ok_or("Media engine not available. Check media configuration.")?;
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let _ = validate_path(path)?;

    // Read image file
    let data = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Failed to read image file: {e}"))?;

    // Detect MIME type from extension
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => return Err(format!("Unsupported image format: .{ext}")),
    };

    let attachment = openfang_types::media::MediaAttachment {
        media_type: openfang_types::media::MediaType::Image,
        mime_type: mime.to_string(),
        source: openfang_types::media::MediaSource::Base64 {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            mime_type: mime.to_string(),
        },
        size_bytes: data.len() as u64,
    };

    let understanding = engine.describe_image(&attachment).await?;
    serde_json::to_string_pretty(&understanding).map_err(|e| format!("Serialize error: {e}"))
}

/// Transcribe audio to text using speech-to-text.
async fn tool_media_transcribe(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
) -> Result<String, String> {
    use base64::Engine;
    let engine = media_engine.ok_or("Media engine not available. Check media configuration.")?;
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let _ = validate_path(path)?;

    // Read audio file
    let data = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Failed to read audio file: {e}"))?;

    // Detect MIME type from extension
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = match ext.as_str() {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "webm" => "audio/webm",
        _ => return Err(format!("Unsupported audio format: .{ext}")),
    };

    let attachment = openfang_types::media::MediaAttachment {
        media_type: openfang_types::media::MediaType::Audio,
        mime_type: mime.to_string(),
        source: openfang_types::media::MediaSource::Base64 {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            mime_type: mime.to_string(),
        },
        size_bytes: data.len() as u64,
    };

    let understanding = engine.transcribe_audio(&attachment).await?;
    serde_json::to_string_pretty(&understanding).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// Image generation tool
// ---------------------------------------------------------------------------

/// Generate images from a text prompt.
async fn tool_image_generate(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let prompt = input["prompt"]
        .as_str()
        .ok_or("Missing 'prompt' parameter")?;

    let model_str = input["model"].as_str().unwrap_or("dall-e-3");
    let model = match model_str {
        "dall-e-3" | "dalle3" | "dalle-3" => openfang_types::media::ImageGenModel::DallE3,
        "dall-e-2" | "dalle2" | "dalle-2" => openfang_types::media::ImageGenModel::DallE2,
        "gpt-image-1" | "gpt_image_1" => openfang_types::media::ImageGenModel::GptImage1,
        _ => {
            return Err(format!(
                "Unknown image model: {model_str}. Use 'dall-e-3', 'dall-e-2', or 'gpt-image-1'."
            ))
        }
    };

    let size = input["size"].as_str().unwrap_or("1024x1024").to_string();
    let quality = input["quality"].as_str().unwrap_or("hd").to_string();
    let count = input["count"].as_u64().unwrap_or(1).min(4) as u8;

    let request = openfang_types::media::ImageGenRequest {
        prompt: prompt.to_string(),
        model,
        size,
        quality,
        count,
    };

    let result = crate::image_gen::generate_image(&request).await?;

    // Save images to workspace if available
    let saved_paths = if let Some(workspace) = workspace_root {
        match crate::image_gen::save_images_to_workspace(&result, workspace) {
            Ok(paths) => paths,
            Err(e) => {
                warn!("Failed to save images to workspace: {e}");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    // Also save to the uploads temp dir so the web UI can serve them via
    // GET /api/uploads/{file_id}.  Each image gets a UUID filename.
    let mut image_urls: Vec<String> = Vec::new();
    {
        use base64::Engine;
        let upload_dir = std::env::temp_dir().join("openfang_uploads");
        let _ = std::fs::create_dir_all(&upload_dir);
        for img in &result.images {
            let file_id = uuid::Uuid::new_v4().to_string();
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&img.data_base64)
            {
                let path = upload_dir.join(&file_id);
                if std::fs::write(&path, &decoded).is_ok() {
                    image_urls.push(format!("/api/uploads/{file_id}"));
                }
            }
        }
    }

    // Build response — include image_urls so the dashboard can render <img> tags
    let response = serde_json::json!({
        "model": result.model,
        "images_generated": result.images.len(),
        "saved_to": saved_paths,
        "revised_prompt": result.revised_prompt,
        "image_urls": image_urls,
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// TTS / STT tools
// ---------------------------------------------------------------------------

async fn tool_text_to_speech(
    input: &serde_json::Value,
    tts_engine: Option<&crate::tts::TtsEngine>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let engine =
        tts_engine.ok_or("TTS engine not available. Ensure tts.enabled=true in config.")?;
    let text = input["text"].as_str().ok_or("Missing 'text' parameter")?;
    let voice = input["voice"].as_str();
    let format = input["format"].as_str();

    let result = engine.synthesize(text, voice, format).await?;

    // Save audio to workspace
    let saved_path = if let Some(workspace) = workspace_root {
        let output_dir = workspace.join("output");
        tokio::fs::create_dir_all(&output_dir)
            .await
            .map_err(|e| format!("Failed to create output dir: {e}"))?;

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let filename = format!("tts_{timestamp}.{}", result.format);
        let path = output_dir.join(&filename);

        tokio::fs::write(&path, &result.audio_data)
            .await
            .map_err(|e| format!("Failed to write audio file: {e}"))?;

        Some(path.display().to_string())
    } else {
        None
    };

    let response = serde_json::json!({
        "saved_to": saved_path,
        "format": result.format,
        "provider": result.provider,
        "duration_estimate_ms": result.duration_estimate_ms,
        "size_bytes": result.audio_data.len(),
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

async fn tool_speech_to_text(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let engine = media_engine.ok_or("Media engine not available for speech-to-text")?;
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let _language = input["language"].as_str();

    let resolved = resolve_file_path(raw_path, workspace_root)?;

    // Read the audio file
    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read audio file: {e}"))?;

    // Determine MIME type from extension
    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp3");
    let mime_type = match ext {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "webm" => "audio/webm",
        _ => "audio/mpeg",
    };

    use openfang_types::media::{MediaAttachment, MediaSource, MediaType};
    let attachment = MediaAttachment {
        media_type: MediaType::Audio,
        mime_type: mime_type.to_string(),
        source: MediaSource::Base64 {
            data: {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(&data)
            },
            mime_type: mime_type.to_string(),
        },
        size_bytes: data.len() as u64,
    };

    let understanding = engine.transcribe_audio(&attachment).await?;

    let response = serde_json::json!({
        "transcript": understanding.description,
        "provider": understanding.provider,
        "model": understanding.model,
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// Docker sandbox tool
// ---------------------------------------------------------------------------

async fn tool_docker_exec(
    input: &serde_json::Value,
    docker_config: Option<&openfang_types::config::DockerSandboxConfig>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let config = docker_config.ok_or("Docker sandbox not configured")?;

    if !config.enabled {
        return Err("Docker sandbox is disabled. Set docker.enabled=true in config.".into());
    }

    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;

    let workspace = workspace_root.ok_or("Docker exec requires a workspace directory")?;
    let agent_id = caller_agent_id.unwrap_or("default");

    // Check Docker availability
    if !crate::docker_sandbox::is_docker_available().await {
        return Err(
            "Docker is not available on this system. Install Docker to use docker_exec.".into(),
        );
    }

    // Create sandbox container
    let container = crate::docker_sandbox::create_sandbox(config, agent_id, workspace).await?;

    // Execute command with timeout
    let timeout = std::time::Duration::from_secs(config.timeout_secs);
    let result = crate::docker_sandbox::exec_in_sandbox(&container, command, timeout).await;

    // Always destroy the container after execution
    if let Err(e) = crate::docker_sandbox::destroy_sandbox(&container).await {
        warn!("Failed to destroy Docker sandbox: {e}");
    }

    let exec_result = result?;

    let response = serde_json::json!({
        "exit_code": exec_result.exit_code,
        "stdout": exec_result.stdout,
        "stderr": exec_result.stderr,
        "container_id": container.container_id,
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// Persistent process tools
// ---------------------------------------------------------------------------

/// Start a long-running process (REPL, server, watcher).
async fn tool_process_start(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let agent_id = caller_agent_id.unwrap_or("default");
    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;
    let args: Vec<String> = input["args"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let proc_id = pm.start(agent_id, command, &args).await?;
    Ok(serde_json::json!({
        "process_id": proc_id,
        "status": "started"
    })
    .to_string())
}

/// Read accumulated stdout/stderr from a process (non-blocking drain).
async fn tool_process_poll(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let proc_id = input["process_id"]
        .as_str()
        .ok_or("Missing 'process_id' parameter")?;
    let (stdout, stderr) = pm.read(proc_id).await?;
    Ok(serde_json::json!({
        "stdout": stdout,
        "stderr": stderr,
    })
    .to_string())
}

/// Write data to a process's stdin.
async fn tool_process_write(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let proc_id = input["process_id"]
        .as_str()
        .ok_or("Missing 'process_id' parameter")?;
    let data = input["data"].as_str().ok_or("Missing 'data' parameter")?;
    // Always append newline if not present (common expectation for REPLs)
    let data = if data.ends_with('\n') {
        data.to_string()
    } else {
        format!("{data}\n")
    };
    pm.write(proc_id, &data).await?;
    Ok(r#"{"status": "written"}"#.to_string())
}

/// Terminate a process.
async fn tool_process_kill(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let proc_id = input["process_id"]
        .as_str()
        .ok_or("Missing 'process_id' parameter")?;
    pm.kill(proc_id).await?;
    Ok(r#"{"status": "killed"}"#.to_string())
}

/// List processes for the current agent.
async fn tool_process_list(
    pm: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let agent_id = caller_agent_id.unwrap_or("default");
    let procs = pm.list(agent_id);
    let list: Vec<serde_json::Value> = procs
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "command": p.command,
                "alive": p.alive,
                "uptime_secs": p.uptime_secs,
            })
        })
        .collect();
    Ok(serde_json::Value::Array(list).to_string())
}

// ---------------------------------------------------------------------------
// Canvas / A2UI tool
// ---------------------------------------------------------------------------

/// Sanitize HTML for canvas presentation.
///
/// SECURITY: Strips dangerous elements and attributes to prevent XSS:
/// - Rejects <script>, <iframe>, <object>, <embed>, <applet> tags
/// - Strips all on* event attributes (onclick, onload, onerror, etc.)
/// - Strips javascript:, data:text/html, vbscript: URLs
/// - Enforces size limit
pub fn sanitize_canvas_html(html: &str, max_bytes: usize) -> Result<String, String> {
    if html.is_empty() {
        return Err("Empty HTML content".to_string());
    }
    if html.len() > max_bytes {
        return Err(format!(
            "HTML too large: {} bytes (max {})",
            html.len(),
            max_bytes
        ));
    }

    let lower = html.to_lowercase();

    // Reject dangerous tags
    let dangerous_tags = [
        "<script", "</script", "<iframe", "</iframe", "<object", "</object", "<embed", "<applet",
        "</applet",
    ];
    for tag in &dangerous_tags {
        if lower.contains(tag) {
            return Err(format!("Forbidden HTML tag detected: {tag}"));
        }
    }

    // Reject event handler attributes (on*)
    // Match patterns like: onclick=, onload=, onerror=, onmouseover=, etc.
    static EVENT_PATTERN: std::sync::LazyLock<regex_lite::Regex> =
        std::sync::LazyLock::new(|| regex_lite::Regex::new(r"(?i)\bon[a-z]+\s*=").unwrap());
    if EVENT_PATTERN.is_match(html) {
        return Err(
            "Forbidden event handler attribute detected (on* attributes are not allowed)"
                .to_string(),
        );
    }

    // Reject dangerous URL schemes
    let dangerous_schemes = ["javascript:", "vbscript:", "data:text/html"];
    for scheme in &dangerous_schemes {
        if lower.contains(scheme) {
            return Err(format!("Forbidden URL scheme detected: {scheme}"));
        }
    }

    Ok(html.to_string())
}

/// Canvas presentation tool handler.
async fn tool_canvas_present(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let html = input["html"].as_str().ok_or("Missing 'html' parameter")?;
    let title = input["title"].as_str().unwrap_or("Canvas");

    // Use configured max from task-local (set by agent_loop from KernelConfig), or default 512KB.
    let max_bytes = CANVAS_MAX_BYTES.try_with(|v| *v).unwrap_or(512 * 1024);
    let sanitized = sanitize_canvas_html(html, max_bytes)?;

    // Generate canvas ID
    let canvas_id = uuid::Uuid::new_v4().to_string();

    // Save to workspace output directory
    let output_dir = if let Some(root) = workspace_root {
        root.join("output")
    } else {
        PathBuf::from("output")
    };
    let _ = tokio::fs::create_dir_all(&output_dir).await;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let filename = format!("canvas_{timestamp}_{}.html", crate::str_utils::safe_truncate_str(&canvas_id, 8));
    let filepath = output_dir.join(&filename);

    // Write the full HTML document
    let full_html = format!(
        "<!DOCTYPE html>\n<html>\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n{sanitized}\n</body>\n</html>"
    );
    tokio::fs::write(&filepath, &full_html)
        .await
        .map_err(|e| format!("Failed to save canvas: {e}"))?;

    let response = serde_json::json!({
        "canvas_id": canvas_id,
        "title": title,
        "saved_to": filepath.to_string_lossy(),
        "size_bytes": full_html.len(),
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use openfang_skills::registry::SkillRegistry;
    use std::collections::HashMap;
    use std::fs;
    use std::sync::Mutex;
    use tempfile::tempdir;

    fn make_skill_registry() -> SkillRegistry {
        let temp = tempdir().unwrap();
        let skill_dir = temp.path().join("github-helper");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
prompt_context = "Use this skill when working with GitHub pull requests."

[skill]
name = "github-helper"
description = "GitHub workflow helper"
tags = ["github", "pull-request"]

[runtime]
type = "promptonly"

[[tools.provided]]
name = "github_comment"
description = "Post a GitHub comment"
input_schema = { type = "object", properties = { issue = { type = "string" } } }
"#,
        )
        .unwrap();

        let mut registry = SkillRegistry::new(temp.path().to_path_buf());
        registry.load_skill(&skill_dir).unwrap();

        // Keep the tempdir alive by leaking it for the duration of the test process.
        std::mem::forget(temp);
        registry
    }

    #[derive(Default)]
    struct TestKernel {
        memory: Mutex<HashMap<String, serde_json::Value>>,
    }

    impl TestKernel {
        fn with_memory(entries: Vec<(String, serde_json::Value)>) -> Self {
            Self {
                memory: Mutex::new(entries.into_iter().collect()),
            }
        }
    }

    #[async_trait]
    impl KernelHandle for TestKernel {
        async fn spawn_agent(
            &self,
            _manifest_toml: &str,
            _parent_id: Option<&str>,
        ) -> Result<(String, String), String> {
            Err("not implemented".to_string())
        }

        async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, String> {
            Err("not implemented".to_string())
        }

        fn list_agents(&self) -> Vec<crate::kernel_handle::AgentInfo> {
            vec![]
        }

        fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
            Err("not implemented".to_string())
        }

        fn memory_store(&self, key: &str, value: serde_json::Value) -> Result<(), String> {
            self.memory.lock().unwrap().insert(key.to_string(), value);
            Ok(())
        }

        fn memory_recall(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(self.memory.lock().unwrap().get(key).cloned())
        }

        fn memory_delete(&self, key: &str) -> Result<(), String> {
            self.memory.lock().unwrap().remove(key);
            Ok(())
        }

        fn memory_list(
            &self,
            prefix: Option<&str>,
            limit: Option<usize>,
        ) -> Result<Vec<(String, serde_json::Value)>, String> {
            let mut entries: Vec<(String, serde_json::Value)> = self
                .memory
                .lock()
                .unwrap()
                .iter()
                .filter(|(key, _)| prefix.map(|prefix| key.starts_with(prefix)).unwrap_or(true))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            if let Some(limit) = limit {
                entries.truncate(limit);
            }
            Ok(entries)
        }

        fn find_agents(&self, _query: &str) -> Vec<crate::kernel_handle::AgentInfo> {
            vec![]
        }

        async fn task_post(
            &self,
            _title: &str,
            _description: &str,
            _assigned_to: Option<&str>,
            _created_by: Option<&str>,
        ) -> Result<String, String> {
            Err("not implemented".to_string())
        }

        async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
            Err("not implemented".to_string())
        }

        async fn task_complete(&self, _task_id: &str, _result: &str) -> Result<(), String> {
            Err("not implemented".to_string())
        }

        async fn task_list(&self, _status: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
            Err("not implemented".to_string())
        }

        async fn publish_event(
            &self,
            _event_type: &str,
            _payload: serde_json::Value,
        ) -> Result<(), String> {
            Err("not implemented".to_string())
        }

        async fn knowledge_add_entity(
            &self,
            _entity: openfang_types::memory::Entity,
        ) -> Result<String, String> {
            Err("not implemented".to_string())
        }

        async fn knowledge_add_relation(
            &self,
            _relation: openfang_types::memory::Relation,
        ) -> Result<String, String> {
            Err("not implemented".to_string())
        }

        async fn knowledge_query(
            &self,
            _pattern: openfang_types::memory::GraphPattern,
        ) -> Result<Vec<openfang_types::memory::GraphMatch>, String> {
            Err("not implemented".to_string())
        }
    }

    #[test]
    fn test_builtin_tool_definitions() {
        let tools = builtin_tool_definitions();
        assert!(
            tools.len() >= 40,
            "Expected at least 40 tools, got {}",
            tools.len()
        );
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        // Original 12
        assert!(names.contains(&"file_read"));
        assert!(names.contains(&"shell_exec"));
        assert!(names.contains(&"agent_send"));
        assert!(names.contains(&"agent_spawn"));
        assert!(names.contains(&"agent_list"));
        assert!(names.contains(&"agent_kill"));
        assert!(names.contains(&"memory_store"));
        assert!(names.contains(&"memory_recall"));
        assert!(names.contains(&"memory_list"));
        assert!(names.contains(&"memory_cleanup"));
        assert!(names.contains(&"memory_autoconverge"));
        // 6 collaboration tools
        assert!(names.contains(&"agent_find"));
        assert!(names.contains(&"task_post"));
        assert!(names.contains(&"task_claim"));
        assert!(names.contains(&"task_complete"));
        assert!(names.contains(&"task_list"));
        assert!(names.contains(&"event_publish"));
        // 5 new Phase 3 tools
        assert!(names.contains(&"schedule_create"));
        assert!(names.contains(&"schedule_list"));
        assert!(names.contains(&"schedule_delete"));
        assert!(names.contains(&"image_analyze"));
        assert!(names.contains(&"location_get"));
        assert!(names.contains(&"system_time"));
        // 6 browser tools
        assert!(names.contains(&"browser_navigate"));
        assert!(names.contains(&"browser_click"));
        assert!(names.contains(&"browser_type"));
        assert!(names.contains(&"browser_screenshot"));
        assert!(names.contains(&"browser_read_page"));
        assert!(names.contains(&"browser_close"));
        assert!(names.contains(&"browser_scroll"));
        assert!(names.contains(&"browser_wait"));
        assert!(names.contains(&"browser_run_js"));
        assert!(names.contains(&"browser_back"));
        // 3 media/image generation tools
        assert!(names.contains(&"media_describe"));
        assert!(names.contains(&"media_transcribe"));
        assert!(names.contains(&"image_generate"));
        // 3 cron tools
        assert!(names.contains(&"cron_create"));
        assert!(names.contains(&"cron_list"));
        assert!(names.contains(&"cron_cancel"));
        // 1 channel send tool
        assert!(names.contains(&"channel_send"));
        // 4 hand tools
        assert!(names.contains(&"hand_list"));
        assert!(names.contains(&"hand_activate"));
        assert!(names.contains(&"hand_status"));
        assert!(names.contains(&"hand_deactivate"));
        // 3 voice/docker tools
        assert!(names.contains(&"text_to_speech"));
        assert!(names.contains(&"speech_to_text"));
        assert!(names.contains(&"docker_exec"));
        // Canvas tool
        assert!(names.contains(&"canvas_present"));
        // Skill discovery tools
        assert!(names.contains(&"tool_search"));
        assert!(names.contains(&"tool_get_instructions"));
        let memory_list = tools
            .iter()
            .find(|tool| tool.name == "memory_list")
            .expect("memory_list tool should exist");
        assert!(memory_list.input_schema["properties"]["tags"].is_object());
        let memory_cleanup = tools
            .iter()
            .find(|tool| tool.name == "memory_cleanup")
            .expect("memory_cleanup tool should exist");
        assert!(memory_cleanup.input_schema["properties"]["apply"].is_object());
        let memory_autoconverge = tools
            .iter()
            .find(|tool| tool.name == "memory_autoconverge")
            .expect("memory_autoconverge tool should exist");
        assert!(memory_autoconverge.input_schema["properties"]["limit"].is_object());
        assert!(
            tools.iter().all(|tool| !tool.defer_loading),
            "builtin tools should remain visible by default; defer_loading is reserved for skill/MCP rollout"
        );
    }

    #[tokio::test]
    async fn test_memory_list_filters_by_all_requested_tags() {
        let themed_metadata = build_memory_record_metadata(
            "pref.editor.theme",
            Some("preference"),
            &["ui".to_string(), "profile".to_string()],
            Some(MemoryFreshness::Durable),
            "memory_store_tool",
        )
        .unwrap();
        let project_metadata = build_memory_record_metadata(
            "project.alpha.status",
            Some("project_state"),
            &["project".to_string(), "alpha".to_string()],
            Some(MemoryFreshness::Durable),
            "memory_store_tool",
        )
        .unwrap();
        let kernel: Arc<dyn KernelHandle> = Arc::new(TestKernel::with_memory(vec![
            ("pref.editor.theme".to_string(), serde_json::json!("dark")),
            (
                memory_metadata_key("pref.editor.theme").unwrap(),
                serde_json::to_value(themed_metadata).unwrap(),
            ),
            (
                "project.alpha.status".to_string(),
                serde_json::json!("green"),
            ),
            (
                memory_metadata_key("project.alpha.status").unwrap(),
                serde_json::to_value(project_metadata).unwrap(),
            ),
            ("general.ungoverned".to_string(), serde_json::json!("note")),
        ]));

        let result = execute_tool(
            "test-id",
            "memory_list",
            &serde_json::json!({"tags": ["profile", "ui"]}),
            Some(&kernel),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert!(!result.is_error, "{}", result.content);
        let rows: Vec<serde_json::Value> = serde_json::from_str(&result.content).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["key"], "pref.editor.theme");
        assert_eq!(rows[0]["tags"], serde_json::json!(["ui", "profile"]));
    }

    #[tokio::test]
    async fn test_memory_cleanup_audit_reports_findings() {
        let kernel = Arc::new(TestKernel::with_memory(vec![
            ("legacy_pref".to_string(), serde_json::json!("dark")),
            (
                "project.alpha.status".to_string(),
                serde_json::json!("green"),
            ),
            (
                "__openfang_memory_meta.pref.orphan".to_string(),
                serde_json::json!({
                    "schema_version": 1,
                    "key": "pref.orphan",
                    "namespace": "pref",
                    "kind": "preference",
                    "tags": [],
                    "freshness": "durable",
                    "source": "memory_store_tool",
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                }),
            ),
        ]));
        let kernel_handle: Arc<dyn KernelHandle> = kernel.clone();

        let result = execute_tool(
            "test-id",
            "memory_cleanup",
            &serde_json::json!({"apply": false}),
            Some(&kernel_handle),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert!(!result.is_error, "{}", result.content);
        let payload: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(payload["apply"], false);
        assert_eq!(payload["counts"]["migrate_legacy_key"], 1);
        assert_eq!(payload["counts"]["backfill_metadata"], 1);
        assert_eq!(payload["counts"]["delete_orphan_metadata"], 1);
        assert!(kernel
            .memory
            .lock()
            .unwrap()
            .contains_key("legacy_pref"));
    }

    #[tokio::test]
    async fn test_memory_cleanup_apply_repairs_governance_issues() {
        let kernel = Arc::new(TestKernel::with_memory(vec![
            ("legacy_pref".to_string(), serde_json::json!("dark")),
            (
                "project.alpha.status".to_string(),
                serde_json::json!("green"),
            ),
            (
                "__openfang_memory_meta.pref.orphan".to_string(),
                serde_json::json!({
                    "schema_version": 1,
                    "key": "pref.orphan",
                    "namespace": "pref",
                    "kind": "preference",
                    "tags": [],
                    "freshness": "durable",
                    "source": "memory_store_tool",
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                }),
            ),
        ]));
        let kernel_handle: Arc<dyn KernelHandle> = kernel.clone();

        let result = execute_tool(
            "test-id",
            "memory_cleanup",
            &serde_json::json!({"apply": true}),
            Some(&kernel_handle),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert!(!result.is_error, "{}", result.content);
        let payload: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(payload["applied_counts"]["migrated_legacy_keys"], 1);
        assert_eq!(payload["applied_counts"]["deleted_orphan_metadata"], 1);
        assert_eq!(payload["applied_counts"]["backfilled_metadata"], 1);

        let memory = kernel.memory.lock().unwrap();
        assert!(!memory.contains_key("legacy_pref"));
        assert_eq!(
            memory.get("general.legacy_pref"),
            Some(&serde_json::json!("dark"))
        );
        assert!(memory.contains_key("__openfang_memory_meta.general.legacy_pref"));
        assert!(memory.contains_key("__openfang_memory_meta.project.alpha.status"));
        assert!(!memory.contains_key("__openfang_memory_meta.pref.orphan"));
    }

    #[tokio::test]
    async fn test_memory_autoconverge_returns_blocked_when_cleanup_is_required() {
        let workspace = tempdir().unwrap();
        fs::write(
            workspace.path().join("MEMORY.md"),
            "# Long-Term Memory\n\n## Durable User Context\n- Existing note\n",
        )
        .unwrap();

        let metadata = build_memory_record_metadata(
            "pref.reply.style",
            Some("preference"),
            &["profile".to_string()],
            Some(MemoryFreshness::Durable),
            "memory_store_tool",
        )
        .unwrap();
        let kernel: Arc<dyn KernelHandle> = Arc::new(TestKernel::with_memory(vec![
            ("legacy_pref".to_string(), serde_json::json!("dark")),
            (
                "pref.reply.style".to_string(),
                serde_json::json!("Use concise bullet points."),
            ),
            (
                memory_metadata_key("pref.reply.style").unwrap(),
                serde_json::to_value(metadata).unwrap(),
            ),
        ]));

        let result = execute_tool(
            "test-id",
            "memory_autoconverge",
            &serde_json::json!({"apply": true}),
            Some(&kernel),
            None,
            Some("assistant-test"),
            None,
            None,
            None,
            None,
            None,
            Some(workspace.path()),
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert!(!result.is_error, "{}", result.content);
        let payload: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(payload["status"], "blocked");
        assert_eq!(payload["summary"]["can_apply"], false);
        assert_eq!(payload["summary"]["cleanup_blockers"], 1);
        let memory_md = fs::read_to_string(workspace.path().join("MEMORY.md")).unwrap();
        assert!(!memory_md.contains("OPENFANG_AUTOCONVERGE_BEGIN"));
    }

    #[tokio::test]
    async fn test_memory_autoconverge_apply_writes_managed_memory_block() {
        let workspace = tempdir().unwrap();
        fs::write(
            workspace.path().join("MEMORY.md"),
            "# Long-Term Memory\n\n## Durable User Context\n- Preferences worth reusing:\n",
        )
        .unwrap();

        let pref_metadata = build_memory_record_metadata(
            "pref.reply.style",
            Some("preference"),
            &["profile".to_string(), "ui".to_string()],
            Some(MemoryFreshness::Durable),
            "memory_store_tool",
        )
        .unwrap();
        let project_metadata = build_memory_record_metadata(
            "project.alpha.status",
            Some("project_state"),
            &["project".to_string(), "alpha".to_string()],
            Some(MemoryFreshness::Durable),
            "memory_store_tool",
        )
        .unwrap();
        let kernel: Arc<dyn KernelHandle> = Arc::new(TestKernel::with_memory(vec![
            (
                "pref.reply.style".to_string(),
                serde_json::json!("Use concise bullet points."),
            ),
            (
                memory_metadata_key("pref.reply.style").unwrap(),
                serde_json::to_value(pref_metadata).unwrap(),
            ),
            (
                "project.alpha.status".to_string(),
                serde_json::json!("Alpha launch is blocked on QA signoff."),
            ),
            (
                memory_metadata_key("project.alpha.status").unwrap(),
                serde_json::to_value(project_metadata).unwrap(),
            ),
        ]));

        let result = execute_tool(
            "test-id",
            "memory_autoconverge",
            &serde_json::json!({"apply": true, "limit": 8}),
            Some(&kernel),
            None,
            Some("assistant-test"),
            None,
            None,
            None,
            None,
            None,
            Some(workspace.path()),
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert!(!result.is_error, "{}", result.content);
        let payload: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(payload["status"], "applied");
        assert_eq!(payload["summary"]["managed_entries"], 2);

        let memory_md = fs::read_to_string(workspace.path().join("MEMORY.md")).unwrap();
        assert!(memory_md.contains("<!-- OPENFANG_AUTOCONVERGE_BEGIN -->"));
        assert!(memory_md.contains("## Autoconverged Memory Snapshot"));
        assert!(memory_md.contains("[pref.reply.style] Use concise bullet points."));
        assert!(memory_md.contains("[project.alpha.status] Alpha launch is blocked on QA signoff."));
    }

    #[test]
    fn test_tool_runner_exposes_authorized_tools_as_visible_by_default() {
        let authorized_tools = vec![
            tool_definition! {
                name: "file_read".to_string(),
                description: "Read file".to_string(),
                input_schema: serde_json::json!({}),
            },
            tool_definition! {
                name: "web_search".to_string(),
                description: "Search web".to_string(),
                input_schema: serde_json::json!({}),
            },
        ];

        let runner = ToolRunner::new(
            authorized_tools.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        assert_eq!(runner.visible_tools().len(), authorized_tools.len());
        assert_eq!(runner.visible_tool_names().len(), authorized_tools.len());
        assert_eq!(runner.visible_tool_names()[0], "file_read");
        assert_eq!(runner.visible_tool_names()[1], "web_search");
    }

    #[test]
    fn test_tool_runner_hides_skill_tools_until_search() {
        let registry = make_skill_registry();
        let authorized_tools = vec![
            tool_definition! {
                name: "tool_search".to_string(),
                description: "Search tools".to_string(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "github_comment".to_string(),
                description: "Post a GitHub comment".to_string(),
                input_schema: serde_json::json!({}),
                defer_loading: true,
            },
        ];

        let runner = ToolRunner::new(
            authorized_tools,
            None,
            None,
            Some(&registry),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        assert_eq!(runner.visible_tool_names(), ["tool_search"]);
    }

    #[tokio::test]
    async fn test_tool_search_expands_matching_skill_tools() {
        let registry = make_skill_registry();
        let authorized_tools = vec![
            tool_definition! {
                name: "tool_search".to_string(),
                description: "Search tools".to_string(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "github_comment".to_string(),
                description: "Post a GitHub comment".to_string(),
                input_schema: serde_json::json!({}),
                defer_loading: true,
            },
        ];

        let mut runner = ToolRunner::new(
            authorized_tools,
            None,
            None,
            Some(&registry),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        let result = runner
            .execute_tool_call(
                "toolu_1",
                "tool_search",
                &serde_json::json!({"query": "github pull request workflow", "top_k": 3}),
            )
            .await;

        assert!(!result.is_error);
        assert!(result.content.contains("github_comment"));
        assert!(result.content.contains("\"kind\": \"tool\""));
        assert!(result.content.contains("\"source\": \"skill\""));
        assert!(result.content.contains("github-helper"));
        assert!(runner
            .visible_tool_names()
            .contains(&"github_comment".to_string()));
    }

    #[tokio::test]
    async fn test_tool_search_returns_skill_manual_entries() {
        let registry = make_skill_registry();
        let authorized_tools = vec![tool_definition! {
            name: "tool_search".to_string(),
            description: "Search tools".to_string(),
            input_schema: serde_json::json!({}),
        }];

        let mut runner = ToolRunner::new(
            authorized_tools,
            None,
            None,
            Some(&registry),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        let result = runner
            .execute_tool_call(
                "toolu_1",
                "tool_search",
                &serde_json::json!({"query": "github workflow guidance", "top_k": 3}),
            )
            .await;

        assert!(!result.is_error);
        assert!(result.content.contains("\"kind\": \"skill_manual\""));
        assert!(result.content.contains("\"name\": \"github-helper\""));
        assert!(result.content.contains("\"has_instructions\": true"));
        assert_eq!(runner.visible_tool_names(), ["tool_search"]);
    }

    #[tokio::test]
    async fn test_tool_search_expands_generic_deferred_tools() {
        let authorized_tools = vec![
            tool_definition! {
                name: "tool_search".to_string(),
                description: "Search tools".to_string(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "docker_exec".to_string(),
                description: "Execute a command inside a Docker sandbox".to_string(),
                input_schema: serde_json::json!({}),
                defer_loading: true,
            },
        ];

        let mut runner = ToolRunner::new(
            authorized_tools,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        let result = runner
            .execute_tool_call(
                "toolu_1",
                "tool_search",
                &serde_json::json!({"query": "docker sandbox command execution", "top_k": 3}),
            )
            .await;

        assert!(!result.is_error);
        assert!(result.content.contains("docker_exec"));
        assert!(result.content.contains("\"source\": \"builtin\""));
        assert!(runner
            .visible_tool_names()
            .contains(&"docker_exec".to_string()));
    }

    #[tokio::test]
    async fn test_tool_search_expands_deferred_mcp_tools() {
        let authorized_tools = vec![
            tool_definition! {
                name: "tool_search".to_string(),
                description: "Search tools".to_string(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "mcp_minimax_web_search".to_string(),
                description: "[MCP:MiniMax] Search the web".to_string(),
                input_schema: serde_json::json!({}),
                defer_loading: true,
            },
        ];

        let mut runner = ToolRunner::new(
            authorized_tools,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        let result = runner
            .execute_tool_call(
                "toolu_1",
                "tool_search",
                &serde_json::json!({"query": "search the web with minimax", "top_k": 3}),
            )
            .await;

        assert!(!result.is_error);
        assert!(result.content.contains("mcp_minimax_web_search"));
        assert!(result.content.contains("\"source\": \"mcp\""));
        assert!(result.content.contains("\"provider\": \"minimax\""));
        assert!(runner
            .visible_tool_names()
            .contains(&"mcp_minimax_web_search".to_string()));
    }

    #[test]
    fn test_collaboration_tool_schemas() {
        let tools = builtin_tool_definitions();
        let collab_tools = [
            "agent_find",
            "task_post",
            "task_claim",
            "task_complete",
            "task_list",
            "event_publish",
        ];
        for name in &collab_tools {
            let tool = tools
                .iter()
                .find(|t| t.name == *name)
                .unwrap_or_else(|| panic!("Tool '{}' not found", name));
            // Verify each has a valid JSON schema
            assert!(
                tool.input_schema.is_object(),
                "Tool '{}' schema should be an object",
                name
            );
            assert_eq!(
                tool.input_schema["type"], "object",
                "Tool '{}' should have type=object",
                name
            );
        }
    }

    #[tokio::test]
    async fn test_file_read_missing() {
        let bad_path = std::env::temp_dir()
            .join("openfang_test_nonexistent_99999")
            .join("file.txt");
        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": bad_path.to_str().unwrap()}),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
        )
        .await;
        assert!(result.is_error, "Expected error but got: {}", result.content);
    }

    #[tokio::test]
    async fn test_file_read_path_traversal_blocked() {
        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": "../../etc/passwd"}),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_file_write_path_traversal_blocked() {
        let result = execute_tool(
            "test-id",
            "file_write",
            &serde_json::json!({"path": "../../../tmp/evil.txt", "content": "pwned"}),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_file_list_path_traversal_blocked() {
        let result = execute_tool(
            "test-id",
            "file_list",
            &serde_json::json!({"path": "/foo/../../etc"}),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_web_search() {
        let result = execute_tool(
            "test-id",
            "web_search",
            &serde_json::json!({"query": "rust programming"}),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
        )
        .await;
        // web_search now attempts a real fetch; may succeed or fail depending on network
        assert!(!result.tool_use_id.is_empty());
    }

    #[tokio::test]
    async fn test_unknown_tool() {
        let result = execute_tool(
            "test-id",
            "nonexistent_tool",
            &serde_json::json!({}),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn test_agent_tools_without_kernel() {
        let result = execute_tool(
            "test-id",
            "agent_list",
            &serde_json::json!({}),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Kernel handle not available"));
    }

    #[tokio::test]
    async fn test_capability_enforcement_denied() {
        let allowed = vec!["file_read".to_string(), "file_list".to_string()];
        let result = execute_tool(
            "test-id",
            "shell_exec",
            &serde_json::json!({"command": "ls"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Permission denied"));
    }

    #[tokio::test]
    async fn test_capability_enforcement_allowed() {
        let allowed = vec!["file_read".to_string()];
        // Use a cross-platform nonexistent path
        let bad_path = std::env::temp_dir()
            .join("openfang_test_nonexistent_12345")
            .join("file.txt");
        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": bad_path.to_str().unwrap()}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
        )
        .await;
        // Should fail for file-not-found, NOT for permission denied
        assert!(result.is_error, "Expected error but got: {}", result.content);
        assert!(
            result.content.contains("Failed to read") || result.content.contains("not found") || result.content.contains("No such file"),
            "Unexpected error: {}", result.content
        );
    }

    #[tokio::test]
    async fn test_capability_enforcement_aliased_tool_name() {
        // Agent has "file_write" in allowed tools, but LLM calls "fs-write".
        // After normalization, this should pass the capability check.
        let allowed = vec![
            "file_read".to_string(),
            "file_write".to_string(),
            "file_list".to_string(),
            "shell_exec".to_string(),
        ];
        let result = execute_tool(
            "test-id",
            "fs-write", // LLM-hallucinated alias
            &serde_json::json!({"path": "/nonexistent/file.txt", "content": "hello"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
        )
        .await;
        // Should NOT be "Permission denied" — it should normalize to file_write
        // and pass the capability check. It will fail for other reasons (path validation).
        assert!(
            !result.content.contains("Permission denied"),
            "fs-write should normalize to file_write and pass capability check, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_capability_enforcement_aliased_denied() {
        // Agent does NOT have file_write, and LLM calls "fs-write" — should be denied.
        let allowed = vec!["file_read".to_string()];
        let result = execute_tool(
            "test-id",
            "fs-write",
            &serde_json::json!({"path": "/tmp/test.txt", "content": "hello"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
        )
        .await;
        assert!(result.is_error);
        assert!(
            result.content.contains("Permission denied"),
            "fs-write should normalize to file_write which is not in allowed list"
        );
    }

    // --- Schedule parser tests ---
    #[test]
    fn test_parse_schedule_every_minutes() {
        assert_eq!(
            parse_schedule_to_cron("every 5 minutes").unwrap(),
            "*/5 * * * *"
        );
        assert_eq!(
            parse_schedule_to_cron("every 1 minute").unwrap(),
            "* * * * *"
        );
        assert_eq!(parse_schedule_to_cron("every minute").unwrap(), "* * * * *");
        assert_eq!(
            parse_schedule_to_cron("every 30 minutes").unwrap(),
            "*/30 * * * *"
        );
    }

    #[test]
    fn test_parse_schedule_every_hours() {
        assert_eq!(parse_schedule_to_cron("every hour").unwrap(), "0 * * * *");
        assert_eq!(parse_schedule_to_cron("every 1 hour").unwrap(), "0 * * * *");
        assert_eq!(
            parse_schedule_to_cron("every 2 hours").unwrap(),
            "0 */2 * * *"
        );
    }

    #[test]
    fn test_parse_schedule_daily() {
        assert_eq!(parse_schedule_to_cron("daily at 9am").unwrap(), "0 9 * * *");
        assert_eq!(
            parse_schedule_to_cron("daily at 6pm").unwrap(),
            "0 18 * * *"
        );
        assert_eq!(
            parse_schedule_to_cron("daily at 12am").unwrap(),
            "0 0 * * *"
        );
        assert_eq!(
            parse_schedule_to_cron("daily at 12pm").unwrap(),
            "0 12 * * *"
        );
    }

    #[test]
    fn test_parse_schedule_weekdays() {
        assert_eq!(
            parse_schedule_to_cron("weekdays at 9am").unwrap(),
            "0 9 * * 1-5"
        );
        assert_eq!(
            parse_schedule_to_cron("weekends at 10am").unwrap(),
            "0 10 * * 0,6"
        );
    }

    #[test]
    fn test_parse_schedule_shorthand() {
        assert_eq!(parse_schedule_to_cron("hourly").unwrap(), "0 * * * *");
        assert_eq!(parse_schedule_to_cron("daily").unwrap(), "0 0 * * *");
        assert_eq!(parse_schedule_to_cron("weekly").unwrap(), "0 0 * * 0");
        assert_eq!(parse_schedule_to_cron("monthly").unwrap(), "0 0 1 * *");
    }

    #[test]
    fn test_parse_schedule_cron_passthrough() {
        assert_eq!(
            parse_schedule_to_cron("0 */5 * * *").unwrap(),
            "0 */5 * * *"
        );
        assert_eq!(
            parse_schedule_to_cron("30 9 * * 1-5").unwrap(),
            "30 9 * * 1-5"
        );
    }

    #[test]
    fn test_parse_schedule_invalid() {
        assert!(parse_schedule_to_cron("whenever I feel like it").is_err());
        assert!(parse_schedule_to_cron("every 0 minutes").is_err());
    }

    // --- Image format detection tests ---
    #[test]
    fn test_detect_image_format_png() {
        let data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x10\x00\x00\x00\x10";
        assert_eq!(detect_image_format(data), "png");
    }

    #[test]
    fn test_detect_image_format_jpeg() {
        let data = b"\xFF\xD8\xFF\xE0\x00\x10JFIF";
        assert_eq!(detect_image_format(data), "jpeg");
    }

    #[test]
    fn test_detect_image_format_gif() {
        let data = b"GIF89a\x10\x00\x10\x00";
        assert_eq!(detect_image_format(data), "gif");
    }

    #[test]
    fn test_detect_image_format_bmp() {
        let data = b"BM\x00\x00\x00\x00";
        assert_eq!(detect_image_format(data), "bmp");
    }

    #[test]
    fn test_detect_image_format_unknown() {
        let data = b"\x00\x00\x00\x00";
        assert_eq!(detect_image_format(data), "unknown");
    }

    #[test]
    fn test_extract_png_dimensions() {
        // Minimal PNG header: signature (8) + IHDR length (4) + "IHDR" (4) + width (4) + height (4)
        let mut data = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]; // signature
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]); // IHDR length
        data.extend_from_slice(b"IHDR"); // chunk type
        data.extend_from_slice(&640u32.to_be_bytes()); // width
        data.extend_from_slice(&480u32.to_be_bytes()); // height
        assert_eq!(extract_image_dimensions(&data, "png"), Some((640, 480)));
    }

    #[test]
    fn test_extract_gif_dimensions() {
        let mut data = b"GIF89a".to_vec();
        data.extend_from_slice(&320u16.to_le_bytes()); // width
        data.extend_from_slice(&240u16.to_le_bytes()); // height
        assert_eq!(extract_image_dimensions(&data, "gif"), Some((320, 240)));
    }

    #[test]
    fn test_format_file_size() {
        assert_eq!(format_file_size(500), "500 B");
        assert_eq!(format_file_size(1536), "1.5 KB");
        assert_eq!(format_file_size(2 * 1024 * 1024), "2.0 MB");
    }

    #[tokio::test]
    async fn test_image_analyze_missing_file() {
        let result = execute_tool(
            "test-id",
            "image_analyze",
            &serde_json::json!({"path": "/nonexistent/image.png"}),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Failed to read"));
    }

    #[test]
    fn test_depth_limit_constant() {
        assert_eq!(MAX_AGENT_CALL_DEPTH, 5);
    }

    #[test]
    fn test_depth_limit_first_call_succeeds() {
        // Default depth is 0, which is < MAX_AGENT_CALL_DEPTH
        let default_depth = AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0);
        assert!(default_depth < MAX_AGENT_CALL_DEPTH);
    }

    #[test]
    fn test_task_local_compiles() {
        // Verify task_local macro works — just ensure the type exists
        let cell = std::cell::Cell::new(0u32);
        assert_eq!(cell.get(), 0);
    }

    #[tokio::test]
    async fn test_schedule_tools_without_kernel() {
        let result = execute_tool(
            "test-id",
            "schedule_list",
            &serde_json::json!({}),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Kernel handle not available"));
    }

    // ─── Canvas / A2UI tests ────────────────────────────────────────

    #[test]
    fn test_sanitize_canvas_basic_html() {
        let html = "<h1>Hello World</h1><p>This is a test.</p>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), html);
    }

    #[test]
    fn test_sanitize_canvas_rejects_script() {
        let html = "<div><script>alert('xss')</script></div>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("script"));
    }

    #[test]
    fn test_sanitize_canvas_rejects_iframe() {
        let html = "<iframe src='https://evil.com'></iframe>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("iframe"));
    }

    #[test]
    fn test_sanitize_canvas_rejects_event_handler() {
        let html = "<div onclick=\"alert('xss')\">click me</div>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("event handler"));
    }

    #[test]
    fn test_sanitize_canvas_rejects_onload() {
        let html = "<img src='x' onerror = \"alert(1)\">";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_canvas_rejects_javascript_url() {
        let html = "<a href=\"javascript:alert('xss')\">click</a>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("javascript:"));
    }

    #[test]
    fn test_sanitize_canvas_rejects_data_html() {
        let html = "<a href=\"data:text/html,<script>alert(1)</script>\">x</a>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_canvas_rejects_empty() {
        let result = sanitize_canvas_html("", 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty"));
    }

    #[test]
    fn test_sanitize_canvas_size_limit() {
        let html = "x".repeat(1024);
        let result = sanitize_canvas_html(&html, 100);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too large"));
    }

    #[tokio::test]
    async fn test_canvas_present_tool() {
        let input = serde_json::json!({
            "html": "<h1>Test Canvas</h1><p>Hello world</p>",
            "title": "Test"
        });
        let tmp = std::env::temp_dir().join("openfang_canvas_test");
        let _ = std::fs::create_dir_all(&tmp);
        let result = tool_canvas_present(&input, Some(tmp.as_path())).await;
        assert!(result.is_ok());
        let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert!(output["canvas_id"].is_string());
        assert_eq!(output["title"], "Test");
        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
