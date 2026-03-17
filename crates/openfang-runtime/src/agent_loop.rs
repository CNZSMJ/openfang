//! Core agent execution loop.
//!
//! The agent loop handles receiving a user message, recalling relevant memories,
//! calling the LLM, executing tool calls, and saving the conversation.

use crate::auth_cooldown::{CooldownVerdict, ProviderCooldown};
use crate::audit::AuditAction;
use crate::context_budget::{ContextBudget, apply_context_guard, truncate_tool_result_dynamic};
use crate::context_overflow::{RecoveryStage, recover_from_overflow};
use crate::embedding::EmbeddingDriver;
use crate::kernel_handle::KernelHandle;
use crate::llm_driver::{CompletionRequest, LlmDriver, LlmError, StreamEvent};
use crate::llm_errors;
use crate::loop_guard::{LoopGuard, LoopGuardConfig, LoopGuardVerdict};
use crate::mcp::McpConnection;
use crate::tool_runner;
use crate::web_search::WebToolsContext;
use openfang_memory::MemorySubstrate;
use openfang_memory::session::Session;
use openfang_skills::registry::SkillRegistry;
use openfang_types::agent::AgentId;
use openfang_types::agent::AgentManifest;
use openfang_types::error::{OpenFangError, OpenFangResult};
use openfang_types::memory::{
    Memory, MemoryContextRecallMode, MemoryFilter, MemorySource, PromptMemoryContextBuildOptions,
    PromptMemoryContextTrace, build_prompt_memory_context,
    build_prompt_memory_context_trace_telemetry, render_prompt_memory_context_trace,
};
use openfang_types::message::{
    ContentBlock, Message, MessageContent, Role, StopReason, TokenUsage,
};
use openfang_types::tool::{ToolCall, ToolDefinition};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Maximum iterations in the agent loop before giving up.
const MAX_ITERATIONS: u32 = 50;

/// Maximum retries for rate-limited or overloaded API calls.
const MAX_RETRIES: u32 = 3;

/// Base delay for exponential backoff (milliseconds).
const BASE_RETRY_DELAY_MS: u64 = 1000;

/// Timeout for individual tool executions (seconds).
/// Raised from 60s to 120s for browser automation and long-running builds.
const TOOL_TIMEOUT_SECS: u64 = 120;

/// Maximum consecutive MaxTokens continuations before returning partial response.
/// Raised from 3 to 5 to allow longer-form generation.
const MAX_CONTINUATIONS: u32 = 5;

/// Maximum message history size before auto-trimming to prevent context overflow.
const MAX_HISTORY_MESSAGES: usize = 20;

/// Logging: Maximum characters for INPUT/OUTPUT in llm.log.
const MAX_LLM_IO_LOG_CHARS: usize = 50_000;

/// Logging: Maximum characters for TOOL_RESULT in llm.log (kept small for readability).
const MAX_TOOL_RESULT_LOG_CHARS: usize = 1000;
/// Logging: Maximum characters for the rendered system prompt inside an INPUT entry.
const MAX_SYSTEM_PROMPT_LOG_CHARS: usize = 32_000;
/// Logging: Maximum characters reserved for rendered messages inside an INPUT entry.
const MAX_MESSAGES_LOG_CHARS: usize = 12_000;
/// Logging: Maximum characters for a plain-text message inside an INPUT entry.
const MAX_MESSAGE_TEXT_LOG_CHARS: usize = 4_000;
/// Logging: Maximum characters for a single block payload inside an INPUT entry.
const MAX_BLOCK_TEXT_LOG_CHARS: usize = 1_500;

/// Strip a provider prefix from a model ID before sending to the API.
///
/// Many models are stored as `provider/org/model` (e.g. `openrouter/google/gemini-2.5-flash`)
/// but the upstream API expects just `org/model` (e.g. `google/gemini-2.5-flash`).
pub fn strip_provider_prefix(model: &str, provider: &str) -> String {
    let slash_prefix = format!("{}/", provider);
    let colon_prefix = format!("{}:", provider);
    if model.starts_with(&slash_prefix) {
        model[slash_prefix.len()..].to_string()
    } else if model.starts_with(&colon_prefix) {
        model[colon_prefix.len()..].to_string()
    } else {
        model.to_string()
    }
}

/// Default context window size (tokens) for token-based trimming.
const DEFAULT_CONTEXT_WINDOW: usize = 200_000;

fn load_governed_memory_entries(
    memory: &MemorySubstrate,
    kernel: Option<&Arc<dyn KernelHandle>>,
    agent_id: AgentId,
) -> Vec<(String, serde_json::Value)> {
    if let Some(kh) = kernel {
        return kh.memory_list(None, Some(500)).unwrap_or_default();
    }

    memory.list_kv(agent_id).unwrap_or_default()
}

fn trim_messages_for_prepended_context(messages: &mut Vec<Message>, reserved_slots: usize) {
    let keep = MAX_HISTORY_MESSAGES.saturating_sub(reserved_slots);
    if messages.len() > keep {
        let trim_count = messages.len() - keep;
        messages.drain(..trim_count);
    }
}

/// Agent lifecycle phase within the execution loop.
/// Used for UX indicators (typing, reactions) without coupling to channel types.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopPhase {
    /// Agent is calling the LLM.
    Thinking,
    /// Agent is executing a tool.
    ToolUse { tool_name: String },
    /// Agent is streaming tokens.
    Streaming,
    /// Agent finished successfully.
    Done,
    /// Agent encountered an error.
    Error,
}

/// Callback for agent lifecycle phase changes.
/// Implementations should be non-blocking (fire-and-forget) to avoid slowing the loop.
pub type PhaseCallback = Arc<dyn Fn(LoopPhase) + Send + Sync>;

/// Result of an agent loop execution.
#[derive(Debug)]
pub struct AgentLoopResult {
    /// The final text response from the agent.
    pub response: String,
    /// Total token usage across all LLM calls.
    pub total_usage: TokenUsage,
    /// Number of iterations the loop ran.
    pub iterations: u32,
    /// Estimated cost in USD (populated by the kernel after the loop returns).
    pub cost_usd: Option<f64>,
    /// True when the agent intentionally chose not to reply (NO_REPLY token or [[silent]]).
    pub silent: bool,
    /// Reply directives extracted from the agent's response.
    pub directives: openfang_types::message::ReplyDirectives,
}

/// Run the agent execution loop for a single user message.
///
/// This is the core of OpenFang: it loads session context, recalls memories,
/// runs the LLM in a tool-use loop, and saves the updated session.
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_loop(
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut Session,
    memory: &MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    authorized_tools: &[ToolDefinition],
    kernel: Option<Arc<dyn KernelHandle>>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    workspace_root: Option<&Path>,
    on_phase: Option<&PhaseCallback>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&openfang_types::config::DockerSandboxConfig>,
    hooks: Option<&crate::hooks::HookRegistry>,
    context_window_tokens: Option<usize>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    user_content_blocks: Option<Vec<ContentBlock>>,
) -> OpenFangResult<AgentLoopResult> {
    run_agent_loop_with_session_message(
        manifest,
        user_message,
        Message::user(user_message),
        session,
        memory,
        driver,
        authorized_tools,
        kernel,
        skill_registry,
        mcp_connections,
        web_ctx,
        browser_ctx,
        embedding_driver,
        workspace_root,
        on_phase,
        media_engine,
        tts_engine,
        docker_config,
        hooks,
        context_window_tokens,
        process_manager,
        user_content_blocks,
    )
    .await
}

/// Run the agent loop, allowing the caller to control the exact user message persisted in session.
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_loop_with_session_message(
    manifest: &AgentManifest,
    user_message: &str,
    session_user_message: Message,
    session: &mut Session,
    memory: &MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    authorized_tools: &[ToolDefinition],
    kernel: Option<Arc<dyn KernelHandle>>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    workspace_root: Option<&Path>,
    on_phase: Option<&PhaseCallback>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&openfang_types::config::DockerSandboxConfig>,
    hooks: Option<&crate::hooks::HookRegistry>,
    context_window_tokens: Option<usize>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    user_content_blocks: Option<Vec<ContentBlock>>,
) -> OpenFangResult<AgentLoopResult> {
    info!(agent = %manifest.name, "Starting agent loop");

    // Extract hand-allowed env vars from manifest metadata (set by kernel for hand settings)
    let hand_allowed_env: Vec<String> = manifest
        .metadata
        .get("hand_allowed_env")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Recall relevant memories — prefer hybrid recall when an embedding driver is available
    let (semantic_recall_mode, _memories) = if let Some(emb) = embedding_driver {
        match emb.embed_one(user_message).await {
            Ok(query_vec) => {
                debug!("Using hybrid recall (dims={})", query_vec.len());
                (
                    MemoryContextRecallMode::Hybrid,
                    memory
                        .recall_with_embedding_async(
                            user_message,
                            5,
                            Some(MemoryFilter {
                                agent_id: Some(session.agent_id),
                                ..Default::default()
                            }),
                            Some(&query_vec),
                        )
                        .await
                        .unwrap_or_default(),
                )
            }
            Err(e) => {
                warn!("Embedding recall failed, falling back to text-only search: {e}");
                (
                    MemoryContextRecallMode::TextOnly,
                    memory
                        .recall(
                            user_message,
                            5,
                            Some(MemoryFilter {
                                agent_id: Some(session.agent_id),
                                ..Default::default()
                            }),
                        )
                        .await
                        .unwrap_or_default(),
                )
            }
        }
    } else {
        (
            MemoryContextRecallMode::TextOnly,
            memory
                .recall(
                    user_message,
                    5,
                    Some(MemoryFilter {
                        agent_id: Some(session.agent_id),
                        ..Default::default()
                    }),
                )
                .await
                .unwrap_or_default(),
        )
    };
    let structured_entries = memory.list_kv(session.agent_id).unwrap_or_default();
    let governed_entries = load_governed_memory_entries(memory, kernel.as_ref(), session.agent_id);
    let memory_context = build_prompt_memory_context(
        user_message,
        semantic_recall_mode,
        &_memories,
        &governed_entries,
        &structured_entries,
        &PromptMemoryContextBuildOptions::default(),
        chrono::Utc::now(),
    );
    let memory_context_msg =
        crate::prompt_builder::build_memory_context_message(&memory_context);
    emit_prompt_memory_context_trace(
        workspace_root,
        &manifest.name,
        session.agent_id,
        kernel.as_ref(),
        &memory_context.trace,
    )
    .await;

    // Fire BeforePromptBuild hook
    let agent_id_str = session.agent_id.0.to_string();
    if let Some(hook_reg) = hooks {
        let ctx = crate::hooks::HookContext {
            agent_name: &manifest.name,
            agent_id: agent_id_str.as_str(),
            event: openfang_types::agent::HookEvent::BeforePromptBuild,
            data: serde_json::json!({
                "system_prompt": &manifest.model.system_prompt,
                "user_message": user_message,
            }),
        };
        let _ = hook_reg.fire(&ctx);
    }

    // Build the system prompt. Dynamic memory context is injected as a separate user message
    // to keep the system prompt stable for provider prompt caching.
    let system_prompt = manifest.model.system_prompt.clone();

    // Add the user message to session history.
    // When content blocks are provided (e.g. text + image from a channel),
    // use multimodal message format so the LLM receives the image for vision.
    if let Some(blocks) = user_content_blocks {
        session.messages.push(Message::user_with_blocks(blocks));
    } else {
        session.messages.push(session_user_message);
    }

    // Build the messages for the LLM, filtering system messages
    // System prompt goes into the separate `system` field
    let llm_messages: Vec<Message> = session
        .messages
        .iter()
        .filter(|m| m.role != Role::System)
        .cloned()
        .collect();

    // Validate and repair session history (drop orphans, merge consecutive)
    let mut messages = crate::session_repair::validate_and_repair(&llm_messages);

    // Inject canonical and memory context as prepended user messages (not in system prompt)
    // to keep the system prompt stable across turns for provider prompt caching.
    let mut prepended_messages = Vec::new();
    if let Some(cc_msg) = manifest
        .metadata
        .get("canonical_context_msg")
        .and_then(|v| v.as_str())
    {
        if !cc_msg.is_empty() {
            prepended_messages.push(Message::user(cc_msg));
        }
    }
    if let Some(mem_msg) = memory_context_msg {
        prepended_messages.push(Message::user(mem_msg));
    }
    let mut total_usage = TokenUsage::default();
    let final_response;

    // Safety valve: trim excessively long message histories to prevent context overflow.
    // The full compaction system handles sophisticated summarization, but this prevents
    // the catastrophic case where 200+ messages cause instant context overflow.
    if messages.len() + prepended_messages.len() > MAX_HISTORY_MESSAGES {
        let keep = MAX_HISTORY_MESSAGES.saturating_sub(prepended_messages.len());
        let trim_count = messages.len().saturating_sub(keep);
        warn!(
            agent = %manifest.name,
            total_messages = messages.len(),
            trimming = trim_count,
            "Trimming old messages to prevent context overflow"
        );
        trim_messages_for_prepended_context(&mut messages, prepended_messages.len());
        // Re-validate after trimming: the drain may have split a ToolUse/ToolResult
        // pair across the cut boundary, leaving orphaned blocks that cause the LLM
        // to return empty responses (input_tokens=0).
        messages = crate::session_repair::validate_and_repair(&messages);
    }
    if !prepended_messages.is_empty() {
        messages.splice(0..0, prepended_messages);
    }

    // Use autonomous config max_iterations if set, else default
    let max_iterations = manifest
        .autonomous
        .as_ref()
        .map(|a| a.max_iterations)
        .unwrap_or(MAX_ITERATIONS);

    // Initialize loop guard — scale circuit breaker for autonomous agents
    let loop_guard_config = {
        let mut cfg = LoopGuardConfig::default();
        if max_iterations > cfg.global_circuit_breaker {
            cfg.global_circuit_breaker = max_iterations * 3;
        }
        cfg
    };
    let mut loop_guard = LoopGuard::new(loop_guard_config);
    let mut consecutive_max_tokens: u32 = 0;

    // Build context budget from model's actual context window (or fallback to default)
    let ctx_window = context_window_tokens.unwrap_or(DEFAULT_CONTEXT_WINDOW);
    let context_budget = ContextBudget::new(ctx_window);
    let mut any_tools_executed = false;
    let effective_exec_policy = manifest.exec_policy.as_ref();
    let mut tool_runner = tool_runner::ToolRunner::new(
        authorized_tools.to_vec(),
        kernel.as_ref(),
        Some(agent_id_str.as_str()),
        skill_registry,
        mcp_connections,
        web_ctx,
        browser_ctx,
        if hand_allowed_env.is_empty() {
            None
        } else {
            Some(hand_allowed_env.as_slice())
        },
        workspace_root,
        media_engine,
        effective_exec_policy,
        tts_engine,
        docker_config,
        process_manager,
    );

    for iteration in 0..max_iterations {
        debug!(iteration, "Agent loop iteration");

        // Context overflow recovery pipeline (replaces emergency_trim_messages)
        let recovery = recover_from_overflow(
            &mut messages,
            &system_prompt,
            tool_runner.visible_tools(),
            ctx_window,
        );
        if recovery == RecoveryStage::FinalError {
            warn!("Context overflow unrecoverable — suggest /reset or /compact");
        }

        // Re-validate tool_call/tool_result pairing after overflow drains
        // which may have broken assistant→tool ordering invariants.
        if recovery != RecoveryStage::None {
            messages = crate::session_repair::validate_and_repair(&messages);
        }

        // Context guard: compact oversized tool results before LLM call
        apply_context_guard(&mut messages, &context_budget, tool_runner.visible_tools());

        // Strip provider prefix: "openrouter/google/gemini-2.5-flash" → "google/gemini-2.5-flash"
        let api_model = strip_provider_prefix(&manifest.model.model, &manifest.model.provider);

        let request = CompletionRequest {
            model: api_model.clone(),
            messages: messages.clone(),
            tools: tool_runner.visible_tools().to_vec(),
            max_tokens: manifest.model.max_tokens,
            temperature: manifest.model.temperature,
            system: Some(system_prompt.clone()),
            thinking: None,
        };

        // Log LLM Input
        let input_log = format_llm_input_log(&system_prompt, &messages);
        log_llm_event(workspace_root, "INPUT", &api_model, &input_log).await;

        // Notify phase: Thinking
        if let Some(cb) = on_phase {
            cb(LoopPhase::Thinking);
        }

        // Call LLM with retry, error classification, and circuit breaker
        let provider_name = manifest.model.provider.as_str();
        let mut response = call_with_retry(&*driver, request, Some(provider_name), None).await?;

        // Log LLM Output
        let output_log = format!(
            "Response: {}\nTool Calls: {:?}\nStop Reason: {:?}\nUsage: {:?}",
            response.text(),
            response.tool_calls,
            response.stop_reason,
            response.usage
        );
        log_llm_event(workspace_root, "OUTPUT", &api_model, &output_log).await;

        total_usage.input_tokens += response.usage.input_tokens;
        total_usage.output_tokens += response.usage.output_tokens;

        // Recover tool calls output as text by models that don't use the tool_calls API field
        // (e.g. Groq/Llama, DeepSeek emit `<function=name>{json}</function>` in text)
        if matches!(
            response.stop_reason,
            StopReason::EndTurn | StopReason::StopSequence
        ) && response.tool_calls.is_empty()
        {
            let recovered = recover_text_tool_calls(&response.text(), tool_runner.visible_tools());
            if !recovered.is_empty() {
                info!(
                    count = recovered.len(),
                    "Recovered text-based tool calls → promoting to ToolUse"
                );
                response.tool_calls = recovered;
                response.stop_reason = StopReason::ToolUse;
                // Build ToolUse content blocks from recovered calls
                let mut new_blocks: Vec<ContentBlock> = Vec::new();
                for tc in &response.tool_calls {
                    new_blocks.push(ContentBlock::ToolUse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input: tc.input.clone(),
                        provider_metadata: None,
                    });
                }
                response.content = new_blocks;
            }
        }

        match response.stop_reason {
            StopReason::EndTurn | StopReason::StopSequence => {
                // LLM is done — extract text and save
                let text = response.text();

                // Parse reply directives from the response text
                let (cleaned_text, parsed_directives) =
                    crate::reply_directives::parse_directives(&text);
                let text = cleaned_text;

                // NO_REPLY: agent intentionally chose not to reply
                if text.trim() == "NO_REPLY" || parsed_directives.silent {
                    debug!(agent = %manifest.name, "Agent chose NO_REPLY/silent — silent completion");
                    session
                        .messages
                        .push(Message::assistant("[no reply needed]".to_string()));
                    memory
                        .save_session(session)
                        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
                    return Ok(AgentLoopResult {
                        response: String::new(),
                        total_usage,
                        iterations: iteration + 1,
                        cost_usd: None,
                        silent: true,
                        directives: openfang_types::message::ReplyDirectives {
                            reply_to: parsed_directives.reply_to,
                            current_thread: parsed_directives.current_thread,
                            silent: true,
                        },
                    });
                }

                // One-shot retry: if the LLM returns empty text with no tool use,
                // try once more before accepting the empty result.
                // Triggers on first call OR when input_tokens=0 (silently failed request).
                if text.trim().is_empty() && response.tool_calls.is_empty() {
                    let is_silent_failure =
                        response.usage.input_tokens == 0 && response.usage.output_tokens == 0;
                    if iteration == 0 || is_silent_failure {
                        warn!(
                            agent = %manifest.name,
                            iteration,
                            input_tokens = response.usage.input_tokens,
                            output_tokens = response.usage.output_tokens,
                            silent_failure = is_silent_failure,
                            "Empty response, retrying once"
                        );
                        // Re-validate messages before retry — the history may have
                        // broken tool_use/tool_result pairs that caused the failure.
                        if is_silent_failure {
                            messages = crate::session_repair::validate_and_repair(&messages);
                        }
                        messages.push(Message::assistant("[no response]".to_string()));
                        messages.push(Message::user("Please provide your response.".to_string()));
                        continue;
                    }
                }

                // Guard against empty response — covers both iteration 0 and post-tool cycles
                let text = if text.trim().is_empty() {
                    warn!(
                        agent = %manifest.name,
                        iteration,
                        input_tokens = total_usage.input_tokens,
                        output_tokens = total_usage.output_tokens,
                        messages_count = messages.len(),
                        "Empty response from LLM — guard activated"
                    );
                    if any_tools_executed {
                        "[Task completed — the agent executed tools but did not produce a text summary.]".to_string()
                    } else {
                        "[The model returned an empty response. This usually means the model is overloaded, the context is too large, or the API key lacks credits. Try again or check /status.]".to_string()
                    }
                } else {
                    text
                };
                final_response = text.clone();
                session.messages.push(Message::assistant(text));

                // Prune NO_REPLY heartbeat turns to save context budget
                crate::session_repair::prune_heartbeat_turns(&mut session.messages, 10);

                // Save session
                memory
                    .save_session(session)
                    .map_err(|e| OpenFangError::Memory(e.to_string()))?;

                // Remember this interaction (with embedding if available)
                let interaction_text = format!(
                    "User asked: {}\nI responded: {}",
                    user_message, final_response
                );
                if let Some(emb) = embedding_driver {
                    match emb.embed_one(&interaction_text).await {
                        Ok(vec) => {
                            let _ = memory
                                .remember_with_embedding_async(
                                    session.agent_id,
                                    &interaction_text,
                                    MemorySource::Conversation,
                                    "episodic",
                                    HashMap::new(),
                                    Some(&vec),
                                )
                                .await;
                        }
                        Err(e) => {
                            warn!("Embedding for remember failed: {e}");
                            let _ = memory
                                .remember(
                                    session.agent_id,
                                    &interaction_text,
                                    MemorySource::Conversation,
                                    "episodic",
                                    HashMap::new(),
                                )
                                .await;
                        }
                    }
                } else {
                    let _ = memory
                        .remember(
                            session.agent_id,
                            &interaction_text,
                            MemorySource::Conversation,
                            "episodic",
                            HashMap::new(),
                        )
                        .await;
                }

                // Notify phase: Done
                if let Some(cb) = on_phase {
                    cb(LoopPhase::Done);
                }

                info!(
                    agent = %manifest.name,
                    iterations = iteration + 1,
                    tokens = total_usage.total(),
                    "Agent loop completed"
                );

                // Fire AgentLoopEnd hook
                if let Some(hook_reg) = hooks {
                    let ctx = crate::hooks::HookContext {
                        agent_name: &manifest.name,
                        agent_id: agent_id_str.as_str(),
                        event: openfang_types::agent::HookEvent::AgentLoopEnd,
                        data: serde_json::json!({
                            "iterations": iteration + 1,
                            "response_length": final_response.len(),
                        }),
                    };
                    let _ = hook_reg.fire(&ctx);
                }

                return Ok(AgentLoopResult {
                    response: final_response,
                    total_usage,
                    iterations: iteration + 1,
                    cost_usd: None,
                    silent: false,
                    directives: Default::default(),
                });
            }
            StopReason::ToolUse => {
                // Reset MaxTokens continuation counter on tool use
                consecutive_max_tokens = 0;
                any_tools_executed = true;

                // Execute tool calls
                let assistant_blocks = response.content.clone();

                // Add assistant message with tool use blocks
                session.messages.push(Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(assistant_blocks.clone()),
                });
                messages.push(Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(assistant_blocks),
                });

                let caller_id_str = session.agent_id.to_string();

                // Execute each tool call with loop guard, timeout, and truncation
                let mut tool_result_blocks = Vec::new();
                for tool_call in &response.tool_calls {
                    // Loop guard check
                    let verdict = loop_guard.check(&tool_call.name, &tool_call.input);
                    match &verdict {
                        LoopGuardVerdict::CircuitBreak(msg) => {
                            warn!(tool = %tool_call.name, "Circuit breaker triggered");
                            // Save session before bailing
                            if let Err(e) = memory.save_session(session) {
                                warn!("Failed to save session on circuit break: {e}");
                            }
                            // Fire AgentLoopEnd hook on circuit break
                            if let Some(hook_reg) = hooks {
                                let ctx = crate::hooks::HookContext {
                                    agent_name: &manifest.name,
                                    agent_id: agent_id_str.as_str(),
                                    event: openfang_types::agent::HookEvent::AgentLoopEnd,
                                    data: serde_json::json!({
                                        "reason": "circuit_break",
                                        "error": msg.as_str(),
                                    }),
                                };
                                let _ = hook_reg.fire(&ctx);
                            }
                            return Err(OpenFangError::Internal(msg.clone()));
                        }
                        LoopGuardVerdict::Block(msg) => {
                            warn!(tool = %tool_call.name, "Tool call blocked by loop guard");
                            tool_result_blocks.push(ContentBlock::ToolResult {
                                tool_use_id: tool_call.id.clone(),
                                tool_name: tool_call.name.clone(),
                                content: msg.clone(),
                                is_error: true,
                            });
                            continue;
                        }
                        _ => {} // Allow or Warn — proceed with execution
                    }

                    debug!(tool = %tool_call.name, id = %tool_call.id, "Executing tool");

                    // Notify phase: ToolUse
                    if let Some(cb) = on_phase {
                        let sanitized: String = tool_call
                            .name
                            .chars()
                            .filter(|c| !c.is_control())
                            .take(64)
                            .collect();
                        cb(LoopPhase::ToolUse {
                            tool_name: sanitized,
                        });
                    }

                    // Fire BeforeToolCall hook (can block execution)
                    if let Some(hook_reg) = hooks {
                        let ctx = crate::hooks::HookContext {
                            agent_name: &manifest.name,
                            agent_id: &caller_id_str,
                            event: openfang_types::agent::HookEvent::BeforeToolCall,
                            data: serde_json::json!({
                                "tool_name": &tool_call.name,
                                "input": &tool_call.input,
                            }),
                        };
                        if let Err(reason) = hook_reg.fire(&ctx) {
                            tool_result_blocks.push(ContentBlock::ToolResult {
                                tool_use_id: tool_call.id.clone(),
                                tool_name: tool_call.name.clone(),
                                content: format!(
                                    "Hook blocked tool '{}': {}",
                                    tool_call.name, reason
                                ),
                                is_error: true,
                            });
                            continue;
                        }
                    }

                    // Timeout-wrapped execution
                    let result = match tokio::time::timeout(
                        Duration::from_secs(TOOL_TIMEOUT_SECS),
                        tool_runner.execute_tool_call(
                            &tool_call.id,
                            &tool_call.name,
                            &tool_call.input,
                        ),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => {
                            warn!(tool = %tool_call.name, "Tool execution timed out after {}s", TOOL_TIMEOUT_SECS);
                            openfang_types::tool::ToolResult {
                                tool_use_id: tool_call.id.clone(),
                                content: format!(
                                    "Tool '{}' timed out after {}s.",
                                    tool_call.name, TOOL_TIMEOUT_SECS
                                ),
                                is_error: true,
                            }
                        }
                    };

                    // Fire AfterToolCall hook
                    if let Some(hook_reg) = hooks {
                        let ctx = crate::hooks::HookContext {
                            agent_name: &manifest.name,
                            agent_id: caller_id_str.as_str(),
                            event: openfang_types::agent::HookEvent::AfterToolCall,
                            data: serde_json::json!({
                                "tool_name": &tool_call.name,
                                "result": &result.content,
                                "is_error": result.is_error,
                            }),
                        };
                        let _ = hook_reg.fire(&ctx);
                    }

                    // Dynamic truncation based on context budget (replaces flat MAX_TOOL_RESULT_CHARS)
                    let content = truncate_tool_result_dynamic(&result.content, &context_budget);

                    // Append warning if verdict was Warn
                    let final_content = if let LoopGuardVerdict::Warn(ref warn_msg) = verdict {
                        format!("{content}\n\n[LOOP GUARD] {warn_msg}")
                    } else {
                        content
                    };

                    tool_result_blocks.push(ContentBlock::ToolResult {
                        tool_use_id: result.tool_use_id,
                        tool_name: tool_call.name.clone(),
                        content: final_content,
                        is_error: result.is_error,
                    });
                }

                // Detect approval denials and inject guidance to prevent infinite retry loops
                let denial_count = tool_result_blocks
                    .iter()
                    .filter(|b| {
                        matches!(b, ContentBlock::ToolResult { content, is_error: true, .. }
                        if content.contains("requires human approval and was denied"))
                    })
                    .count();
                if denial_count > 0 {
                    tool_result_blocks.push(ContentBlock::Text {
                        text: format!(
                            "[System: {} tool call(s) were denied by approval policy. \
                             Do NOT retry denied tools. Explain to the user what you \
                             wanted to do and that it requires their approval.]",
                            denial_count
                        ),
                        provider_metadata: None,
                    });
                }

                // Detect tool errors and inject guidance to prevent fabrication
                let error_count = tool_result_blocks
                    .iter()
                    .filter(|b| matches!(b, ContentBlock::ToolResult { is_error: true, .. }))
                    .count();
                let non_denial_errors = error_count.saturating_sub(denial_count);
                if non_denial_errors > 0 {
                    tool_result_blocks.push(ContentBlock::Text {
                        text: format!(
                            "[System: {} tool(s) returned errors. Report the error honestly \
                             to the user. Do NOT fabricate results or pretend the tool succeeded. \
                             If a search or fetch failed, tell the user it failed and suggest \
                             alternatives instead of making up data.]",
                            non_denial_errors
                        ),
                        provider_metadata: None,
                    });
                }

                // Add tool results as a user message (Anthropic API requirement)
                let tool_results_msg = Message {
                    role: Role::User,
                    content: MessageContent::Blocks(tool_result_blocks.clone()),
                };

                // Log Tool Results
                let tool_log = format!("Results: {:?}", tool_result_blocks);
                log_llm_event(workspace_root, "TOOL_RESULT", "", &tool_log).await;

                session.messages.push(tool_results_msg.clone());
                messages.push(tool_results_msg);

                // Interim save after tool execution to prevent data loss on crash
                if let Err(e) = memory.save_session(session) {
                    warn!("Failed to interim-save session: {e}");
                }
            }
            StopReason::MaxTokens => {
                consecutive_max_tokens += 1;
                if consecutive_max_tokens >= MAX_CONTINUATIONS {
                    // Return partial response instead of continuing forever
                    let text = response.text();
                    let text = if text.trim().is_empty() {
                        "[Partial response — token limit reached with no text output.]".to_string()
                    } else {
                        text
                    };
                    session.messages.push(Message::assistant(&text));
                    if let Err(e) = memory.save_session(session) {
                        warn!("Failed to save session on max continuations: {e}");
                    }
                    warn!(
                        iteration,
                        consecutive_max_tokens,
                        "Max continuations reached, returning partial response"
                    );
                    // Fire AgentLoopEnd hook
                    if let Some(hook_reg) = hooks {
                        let ctx = crate::hooks::HookContext {
                            agent_name: &manifest.name,
                            agent_id: agent_id_str.as_str(),
                            event: openfang_types::agent::HookEvent::AgentLoopEnd,
                            data: serde_json::json!({
                                "iterations": iteration + 1,
                                "reason": "max_continuations",
                            }),
                        };
                        let _ = hook_reg.fire(&ctx);
                    }
                    return Ok(AgentLoopResult {
                        response: text,
                        total_usage,
                        iterations: iteration + 1,
                        cost_usd: None,
                        silent: false,
                        directives: Default::default(),
                    });
                }
                // Model hit token limit — add partial response and continue
                let text = response.text();
                session.messages.push(Message::assistant(&text));
                messages.push(Message::assistant(&text));
                session.messages.push(Message::user("Please continue."));
                messages.push(Message::user("Please continue."));
                warn!(iteration, "Max tokens hit, continuing");
            }
        }
    }

    // Save session before failing so conversation history is preserved
    if let Err(e) = memory.save_session(session) {
        warn!("Failed to save session on max iterations: {e}");
    }

    // Fire AgentLoopEnd hook on max iterations exceeded
    if let Some(hook_reg) = hooks {
        let ctx = crate::hooks::HookContext {
            agent_name: &manifest.name,
            agent_id: agent_id_str.as_str(),
            event: openfang_types::agent::HookEvent::AgentLoopEnd,
            data: serde_json::json!({
                "reason": "max_iterations_exceeded",
                "iterations": max_iterations,
            }),
        };
        let _ = hook_reg.fire(&ctx);
    }

    Err(OpenFangError::MaxIterationsExceeded(max_iterations))
}

/// Call an LLM driver with automatic retry on rate-limit and overload errors.
///
/// Uses the `llm_errors` classifier for smart error handling and the
/// `ProviderCooldown` circuit breaker to prevent request storms.
async fn call_with_retry(
    driver: &dyn LlmDriver,
    request: CompletionRequest,
    provider: Option<&str>,
    cooldown: Option<&ProviderCooldown>,
) -> OpenFangResult<crate::llm_driver::CompletionResponse> {
    // Check circuit breaker before calling
    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
        match cooldown.check(provider) {
            CooldownVerdict::Reject {
                reason,
                retry_after_secs,
            } => {
                return Err(OpenFangError::LlmDriver(format!(
                    "Provider '{provider}' is in cooldown ({reason}). Retry in {retry_after_secs}s."
                )));
            }
            CooldownVerdict::AllowProbe => {
                debug!(provider, "Allowing probe request through circuit breaker");
            }
            CooldownVerdict::Allow => {}
        }
    }

    let mut last_error = None;

    for attempt in 0..=MAX_RETRIES {
        match driver.complete(request.clone()).await {
            Ok(response) => {
                // Record success with circuit breaker
                if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                    cooldown.record_success(provider);
                }
                return Ok(response);
            }
            Err(LlmError::RateLimited { retry_after_ms }) => {
                if attempt == MAX_RETRIES {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    return Err(OpenFangError::LlmDriver(format!(
                        "Rate limited after {} retries",
                        MAX_RETRIES
                    )));
                }
                let delay = std::cmp::max(retry_after_ms, BASE_RETRY_DELAY_MS * 2u64.pow(attempt));
                warn!(
                    attempt,
                    delay_ms = delay,
                    "Rate limited, retrying after delay"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                last_error = Some("Rate limited".to_string());
            }
            Err(LlmError::Overloaded { retry_after_ms }) => {
                if attempt == MAX_RETRIES {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    return Err(OpenFangError::LlmDriver(format!(
                        "Model overloaded after {} retries",
                        MAX_RETRIES
                    )));
                }
                let delay = std::cmp::max(retry_after_ms, BASE_RETRY_DELAY_MS * 2u64.pow(attempt));
                warn!(
                    attempt,
                    delay_ms = delay,
                    "Model overloaded, retrying after delay"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                last_error = Some("Overloaded".to_string());
            }
            Err(e) => {
                // Use classifier for smarter error handling
                let raw_error = e.to_string();
                let status = match &e {
                    LlmError::Api { status, .. } => Some(*status),
                    _ => None,
                };
                let classified = llm_errors::classify_error(&raw_error, status);
                warn!(
                    category = ?classified.category,
                    retryable = classified.is_retryable,
                    raw = %raw_error,
                    "LLM error classified: {}",
                    classified.sanitized_message
                );

                if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                    cooldown.record_failure(provider, classified.is_billing);
                }

                // Include raw error detail so dashboard users can debug
                let user_msg = if classified.category == llm_errors::LlmErrorCategory::Format {
                    format!("{} — raw: {}", classified.sanitized_message, raw_error)
                } else {
                    classified.sanitized_message
                };
                return Err(OpenFangError::LlmDriver(user_msg));
            }
        }
    }

    Err(OpenFangError::LlmDriver(
        last_error.unwrap_or_else(|| "Unknown error".to_string()),
    ))
}

/// Call an LLM driver in streaming mode with automatic retry on rate-limit and overload errors.
///
/// Uses the `llm_errors` classifier and `ProviderCooldown` circuit breaker.
async fn stream_with_retry(
    driver: &dyn LlmDriver,
    request: CompletionRequest,
    tx: mpsc::Sender<StreamEvent>,
    provider: Option<&str>,
    cooldown: Option<&ProviderCooldown>,
) -> OpenFangResult<crate::llm_driver::CompletionResponse> {
    // Check circuit breaker before calling
    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
        match cooldown.check(provider) {
            CooldownVerdict::Reject {
                reason,
                retry_after_secs,
            } => {
                return Err(OpenFangError::LlmDriver(format!(
                    "Provider '{provider}' is in cooldown ({reason}). Retry in {retry_after_secs}s."
                )));
            }
            CooldownVerdict::AllowProbe => {
                debug!(
                    provider,
                    "Allowing probe request through circuit breaker (stream)"
                );
            }
            CooldownVerdict::Allow => {}
        }
    }

    let mut last_error = None;

    for attempt in 0..=MAX_RETRIES {
        match driver.stream(request.clone(), tx.clone()).await {
            Ok(response) => {
                if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                    cooldown.record_success(provider);
                }
                return Ok(response);
            }
            Err(LlmError::RateLimited { retry_after_ms }) => {
                if attempt == MAX_RETRIES {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    return Err(OpenFangError::LlmDriver(format!(
                        "Rate limited after {} retries",
                        MAX_RETRIES
                    )));
                }
                let delay = std::cmp::max(retry_after_ms, BASE_RETRY_DELAY_MS * 2u64.pow(attempt));
                warn!(
                    attempt,
                    delay_ms = delay,
                    "Rate limited (stream), retrying after delay"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                last_error = Some("Rate limited".to_string());
            }
            Err(LlmError::Overloaded { retry_after_ms }) => {
                if attempt == MAX_RETRIES {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    return Err(OpenFangError::LlmDriver(format!(
                        "Model overloaded after {} retries",
                        MAX_RETRIES
                    )));
                }
                let delay = std::cmp::max(retry_after_ms, BASE_RETRY_DELAY_MS * 2u64.pow(attempt));
                warn!(
                    attempt,
                    delay_ms = delay,
                    "Model overloaded (stream), retrying after delay"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                last_error = Some("Overloaded".to_string());
            }
            Err(e) => {
                let raw_error = e.to_string();
                let status = match &e {
                    LlmError::Api { status, .. } => Some(*status),
                    _ => None,
                };
                let classified = llm_errors::classify_error(&raw_error, status);
                warn!(
                    category = ?classified.category,
                    retryable = classified.is_retryable,
                    raw = %raw_error,
                    "LLM stream error classified: {}",
                    classified.sanitized_message
                );

                if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                    cooldown.record_failure(provider, classified.is_billing);
                }

                let user_msg = if classified.category == llm_errors::LlmErrorCategory::Format {
                    format!("{} — raw: {}", classified.sanitized_message, raw_error)
                } else {
                    classified.sanitized_message
                };
                return Err(OpenFangError::LlmDriver(user_msg));
            }
        }
    }

    Err(OpenFangError::LlmDriver(
        last_error.unwrap_or_else(|| "Unknown error".to_string()),
    ))
}

/// Run the agent execution loop with streaming support.
///
/// Like `run_agent_loop`, but sends `StreamEvent`s to the provided channel
/// as tokens arrive from the LLM. Tool execution happens between LLM calls
/// and is not streamed.
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_loop_streaming(
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut Session,
    memory: &MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    authorized_tools: &[ToolDefinition],
    kernel: Option<Arc<dyn KernelHandle>>,
    stream_tx: mpsc::Sender<StreamEvent>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    workspace_root: Option<&Path>,
    on_phase: Option<&PhaseCallback>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&openfang_types::config::DockerSandboxConfig>,
    hooks: Option<&crate::hooks::HookRegistry>,
    context_window_tokens: Option<usize>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    user_content_blocks: Option<Vec<ContentBlock>>,
) -> OpenFangResult<AgentLoopResult> {
    info!(agent = %manifest.name, "Starting streaming agent loop");

    // Extract hand-allowed env vars from manifest metadata (set by kernel for hand settings)
    let hand_allowed_env: Vec<String> = manifest
        .metadata
        .get("hand_allowed_env")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Recall relevant memories — prefer hybrid recall when an embedding driver is available
    let (semantic_recall_mode, _memories) = if let Some(emb) = embedding_driver {
        match emb.embed_one(user_message).await {
            Ok(query_vec) => {
                debug!("Using hybrid recall (streaming, dims={})", query_vec.len());
                (
                    MemoryContextRecallMode::Hybrid,
                    memory
                        .recall_with_embedding_async(
                            user_message,
                            5,
                            Some(MemoryFilter {
                                agent_id: Some(session.agent_id),
                                ..Default::default()
                            }),
                            Some(&query_vec),
                        )
                        .await
                        .unwrap_or_default(),
                )
            }
            Err(e) => {
                warn!("Embedding recall failed (streaming), falling back to text-only search: {e}");
                (
                    MemoryContextRecallMode::TextOnly,
                    memory
                        .recall(
                            user_message,
                            5,
                            Some(MemoryFilter {
                                agent_id: Some(session.agent_id),
                                ..Default::default()
                            }),
                        )
                        .await
                        .unwrap_or_default(),
                )
            }
        }
    } else {
        (
            MemoryContextRecallMode::TextOnly,
            memory
                .recall(
                    user_message,
                    5,
                    Some(MemoryFilter {
                        agent_id: Some(session.agent_id),
                        ..Default::default()
                    }),
                )
                .await
                .unwrap_or_default(),
        )
    };
    let structured_entries = memory.list_kv(session.agent_id).unwrap_or_default();
    let governed_entries = load_governed_memory_entries(memory, kernel.as_ref(), session.agent_id);
    let memory_context = build_prompt_memory_context(
        user_message,
        semantic_recall_mode,
        &_memories,
        &governed_entries,
        &structured_entries,
        &PromptMemoryContextBuildOptions::default(),
        chrono::Utc::now(),
    );
    let memory_context_msg =
        crate::prompt_builder::build_memory_context_message(&memory_context);
    emit_prompt_memory_context_trace(
        workspace_root,
        &manifest.name,
        session.agent_id,
        kernel.as_ref(),
        &memory_context.trace,
    )
    .await;

    // Fire BeforePromptBuild hook
    let agent_id_str = session.agent_id.0.to_string();
    if let Some(hook_reg) = hooks {
        let ctx = crate::hooks::HookContext {
            agent_name: &manifest.name,
            agent_id: agent_id_str.as_str(),
            event: openfang_types::agent::HookEvent::BeforePromptBuild,
            data: serde_json::json!({
                "system_prompt": &manifest.model.system_prompt,
                "user_message": user_message,
            }),
        };
        let _ = hook_reg.fire(&ctx);
    }

    // Build the system prompt. Dynamic memory context is injected as a separate user message
    // to keep the system prompt stable for provider prompt caching.
    let system_prompt = manifest.model.system_prompt.clone();

    // Add the user message to session history.
    // When content blocks are provided (e.g. text + image from a channel),
    // use multimodal message format so the LLM receives the image for vision.
    if let Some(blocks) = user_content_blocks {
        session.messages.push(Message::user_with_blocks(blocks));
    } else {
        session.messages.push(Message::user(user_message));
    }

    let llm_messages: Vec<Message> = session
        .messages
        .iter()
        .filter(|m| m.role != Role::System)
        .cloned()
        .collect();

    // Validate and repair session history (drop orphans, merge consecutive)
    let mut messages = crate::session_repair::validate_and_repair(&llm_messages);

    // Inject canonical and memory context as prepended user messages (not in system prompt)
    // to keep the system prompt stable across turns for provider prompt caching.
    let mut prepended_messages = Vec::new();
    if let Some(cc_msg) = manifest
        .metadata
        .get("canonical_context_msg")
        .and_then(|v| v.as_str())
    {
        if !cc_msg.is_empty() {
            prepended_messages.push(Message::user(cc_msg));
        }
    }
    if let Some(mem_msg) = memory_context_msg {
        prepended_messages.push(Message::user(mem_msg));
    }
    let mut total_usage = TokenUsage::default();
    let final_response;

    // Safety valve: trim excessively long message histories to prevent context overflow.
    if messages.len() + prepended_messages.len() > MAX_HISTORY_MESSAGES {
        let keep = MAX_HISTORY_MESSAGES.saturating_sub(prepended_messages.len());
        let trim_count = messages.len().saturating_sub(keep);
        warn!(
            agent = %manifest.name,
            total_messages = messages.len(),
            trimming = trim_count,
            "Trimming old messages to prevent context overflow (streaming)"
        );
        trim_messages_for_prepended_context(&mut messages, prepended_messages.len());
        // Re-validate after trimming: the drain may have split a ToolUse/ToolResult
        // pair across the cut boundary, leaving orphaned blocks that cause the LLM
        // to return empty responses (input_tokens=0).
        messages = crate::session_repair::validate_and_repair(&messages);
    }
    if !prepended_messages.is_empty() {
        messages.splice(0..0, prepended_messages);
    }

    // Use autonomous config max_iterations if set, else default
    let max_iterations = manifest
        .autonomous
        .as_ref()
        .map(|a| a.max_iterations)
        .unwrap_or(MAX_ITERATIONS);

    // Initialize loop guard — scale circuit breaker for autonomous agents
    let loop_guard_config = {
        let mut cfg = LoopGuardConfig::default();
        if max_iterations > cfg.global_circuit_breaker {
            cfg.global_circuit_breaker = max_iterations * 3;
        }
        cfg
    };
    let mut loop_guard = LoopGuard::new(loop_guard_config);
    let mut consecutive_max_tokens: u32 = 0;

    // Build context budget from model's actual context window (or fallback to default)
    let ctx_window = context_window_tokens.unwrap_or(DEFAULT_CONTEXT_WINDOW);
    let context_budget = ContextBudget::new(ctx_window);
    let mut any_tools_executed = false;
    let effective_exec_policy = manifest.exec_policy.as_ref();
    let mut tool_runner = tool_runner::ToolRunner::new(
        authorized_tools.to_vec(),
        kernel.as_ref(),
        Some(agent_id_str.as_str()),
        skill_registry,
        mcp_connections,
        web_ctx,
        browser_ctx,
        if hand_allowed_env.is_empty() {
            None
        } else {
            Some(hand_allowed_env.as_slice())
        },
        workspace_root,
        media_engine,
        effective_exec_policy,
        tts_engine,
        docker_config,
        process_manager,
    );

    for iteration in 0..max_iterations {
        debug!(iteration, "Streaming agent loop iteration");

        // Context overflow recovery pipeline (replaces emergency_trim_messages)
        let recovery = recover_from_overflow(
            &mut messages,
            &system_prompt,
            tool_runner.visible_tools(),
            ctx_window,
        );
        match &recovery {
            RecoveryStage::None => {}
            RecoveryStage::FinalError => {
                if stream_tx.send(StreamEvent::PhaseChange {
                    phase: "context_warning".to_string(),
                    detail: Some("Context overflow unrecoverable. Use /reset or /compact.".to_string()),
                }).await.is_err() {
                    warn!("Stream consumer disconnected while sending context overflow warning");
                }
            }
            _ => {
                if stream_tx.send(StreamEvent::PhaseChange {
                    phase: "context_warning".to_string(),
                    detail: Some("Older messages trimmed to stay within context limits. Use /compact for smarter summarization.".to_string()),
                }).await.is_err() {
                    warn!("Stream consumer disconnected while sending context trim warning");
                }
            }
        }

        // Context guard: compact oversized tool results before LLM call
        apply_context_guard(&mut messages, &context_budget, tool_runner.visible_tools());

        // Strip provider prefix: "openrouter/google/gemini-2.5-flash" → "google/gemini-2.5-flash"
        let api_model = strip_provider_prefix(&manifest.model.model, &manifest.model.provider);

        let request = CompletionRequest {
            model: api_model.clone(),
            messages: messages.clone(),
            tools: tool_runner.visible_tools().to_vec(),
            max_tokens: manifest.model.max_tokens,
            temperature: manifest.model.temperature,
            system: Some(system_prompt.clone()),
            thinking: None,
        };

        // Log LLM Input (streaming)
        let input_log = format_llm_input_log(&system_prompt, &messages);
        log_llm_event(workspace_root, "INPUT", &api_model, &input_log).await;

        // Notify phase: on first iteration emit Streaming; on subsequent
        // iterations (after tool execution) emit Thinking so the UI shows
        // "Thinking..." instead of overwriting streamed text with "streaming".
        if let Some(cb) = on_phase {
            if iteration == 0 {
                cb(LoopPhase::Streaming);
            } else {
                cb(LoopPhase::Thinking);
            }
        }

        // Stream LLM call with retry, error classification, and circuit breaker
        let provider_name = manifest.model.provider.as_str();
        let mut response = stream_with_retry(
            &*driver,
            request,
            stream_tx.clone(),
            Some(provider_name),
            None,
        )
        .await?;

        // Log LLM Output (streaming)
        let output_log = format!(
            "Response (concatenated): {}\nTool Calls: {:?}\nStop Reason: {:?}\nUsage: {:?}",
            response.text(),
            response.tool_calls,
            response.stop_reason,
            response.usage
        );
        log_llm_event(workspace_root, "OUTPUT", &api_model, &output_log).await;

        total_usage.input_tokens += response.usage.input_tokens;
        total_usage.output_tokens += response.usage.output_tokens;

        // Recover tool calls output as text (streaming path)
        if matches!(
            response.stop_reason,
            StopReason::EndTurn | StopReason::StopSequence
        ) && response.tool_calls.is_empty()
        {
            let recovered = recover_text_tool_calls(&response.text(), tool_runner.visible_tools());
            if !recovered.is_empty() {
                info!(
                    count = recovered.len(),
                    "Recovered text-based tool calls (streaming) → promoting to ToolUse"
                );
                response.tool_calls = recovered;
                response.stop_reason = StopReason::ToolUse;
                let mut new_blocks: Vec<ContentBlock> = Vec::new();
                for tc in &response.tool_calls {
                    new_blocks.push(ContentBlock::ToolUse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input: tc.input.clone(),
                        provider_metadata: None,
                    });
                }
                response.content = new_blocks;
            }
        }

        match response.stop_reason {
            StopReason::EndTurn | StopReason::StopSequence => {
                let text = response.text();

                // Parse reply directives from the streaming response text
                let (cleaned_text_s, parsed_directives_s) =
                    crate::reply_directives::parse_directives(&text);
                let text = cleaned_text_s;

                // NO_REPLY: agent intentionally chose not to reply
                if text.trim() == "NO_REPLY" || parsed_directives_s.silent {
                    debug!(agent = %manifest.name, "Agent chose NO_REPLY/silent (streaming) — silent completion");
                    session
                        .messages
                        .push(Message::assistant("[no reply needed]".to_string()));
                    memory
                        .save_session(session)
                        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
                    return Ok(AgentLoopResult {
                        response: String::new(),
                        total_usage,
                        iterations: iteration + 1,
                        cost_usd: None,
                        silent: true,
                        directives: openfang_types::message::ReplyDirectives {
                            reply_to: parsed_directives_s.reply_to,
                            current_thread: parsed_directives_s.current_thread,
                            silent: true,
                        },
                    });
                }

                // One-shot retry: if the LLM returns empty text with no tool use,
                // try once more before accepting the empty result.
                // Triggers on first call OR when input_tokens=0 (silently failed request).
                if text.trim().is_empty() && response.tool_calls.is_empty() {
                    let is_silent_failure =
                        response.usage.input_tokens == 0 && response.usage.output_tokens == 0;
                    if iteration == 0 || is_silent_failure {
                        warn!(
                            agent = %manifest.name,
                            iteration,
                            input_tokens = response.usage.input_tokens,
                            output_tokens = response.usage.output_tokens,
                            silent_failure = is_silent_failure,
                            "Empty response (streaming), retrying once"
                        );
                        // Re-validate messages before retry — the history may have
                        // broken tool_use/tool_result pairs that caused the failure.
                        if is_silent_failure {
                            messages = crate::session_repair::validate_and_repair(&messages);
                        }
                        messages.push(Message::assistant("[no response]".to_string()));
                        messages.push(Message::user("Please provide your response.".to_string()));
                        continue;
                    }
                }

                // Guard against empty response — covers both iteration 0 and post-tool cycles
                let text = if text.trim().is_empty() {
                    warn!(
                        agent = %manifest.name,
                        iteration,
                        input_tokens = total_usage.input_tokens,
                        output_tokens = total_usage.output_tokens,
                        messages_count = messages.len(),
                        "Empty response from LLM (streaming) — guard activated"
                    );
                    if any_tools_executed {
                        "[Task completed — the agent executed tools but did not produce a text summary.]".to_string()
                    } else {
                        "[The model returned an empty response. This usually means the model is overloaded, the context is too large, or the API key lacks credits. Try again or check /status.]".to_string()
                    }
                } else {
                    text
                };
                final_response = text.clone();
                session.messages.push(Message::assistant(text));

                // Prune NO_REPLY heartbeat turns to save context budget
                crate::session_repair::prune_heartbeat_turns(&mut session.messages, 10);

                memory
                    .save_session(session)
                    .map_err(|e| OpenFangError::Memory(e.to_string()))?;

                // Remember this interaction (with embedding if available)
                let interaction_text = format!(
                    "User asked: {}\nI responded: {}",
                    user_message, final_response
                );
                if let Some(emb) = embedding_driver {
                    match emb.embed_one(&interaction_text).await {
                        Ok(vec) => {
                            let _ = memory
                                .remember_with_embedding_async(
                                    session.agent_id,
                                    &interaction_text,
                                    MemorySource::Conversation,
                                    "episodic",
                                    HashMap::new(),
                                    Some(&vec),
                                )
                                .await;
                        }
                        Err(e) => {
                            warn!("Embedding for remember failed (streaming): {e}");
                            let _ = memory
                                .remember(
                                    session.agent_id,
                                    &interaction_text,
                                    MemorySource::Conversation,
                                    "episodic",
                                    HashMap::new(),
                                )
                                .await;
                        }
                    }
                } else {
                    let _ = memory
                        .remember(
                            session.agent_id,
                            &interaction_text,
                            MemorySource::Conversation,
                            "episodic",
                            HashMap::new(),
                        )
                        .await;
                }

                // Notify phase: Done
                if let Some(cb) = on_phase {
                    cb(LoopPhase::Done);
                }

                info!(
                    agent = %manifest.name,
                    iterations = iteration + 1,
                    tokens = total_usage.total(),
                    "Streaming agent loop completed"
                );

                // Fire AgentLoopEnd hook
                if let Some(hook_reg) = hooks {
                    let ctx = crate::hooks::HookContext {
                        agent_name: &manifest.name,
                        agent_id: agent_id_str.as_str(),
                        event: openfang_types::agent::HookEvent::AgentLoopEnd,
                        data: serde_json::json!({
                            "iterations": iteration + 1,
                            "response_length": final_response.len(),
                        }),
                    };
                    let _ = hook_reg.fire(&ctx);
                }

                return Ok(AgentLoopResult {
                    response: final_response,
                    total_usage,
                    iterations: iteration + 1,
                    cost_usd: None,
                    silent: false,
                    directives: Default::default(),
                });
            }
            StopReason::ToolUse => {
                // Reset MaxTokens continuation counter on tool use
                consecutive_max_tokens = 0;
                any_tools_executed = true;

                let assistant_blocks = response.content.clone();

                session.messages.push(Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(assistant_blocks.clone()),
                });
                messages.push(Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(assistant_blocks),
                });

                let caller_id_str = session.agent_id.to_string();

                // Execute each tool call with loop guard, timeout, and truncation
                let mut tool_result_blocks = Vec::new();
                for tool_call in &response.tool_calls {
                    // Loop guard check
                    let verdict = loop_guard.check(&tool_call.name, &tool_call.input);
                    match &verdict {
                        LoopGuardVerdict::CircuitBreak(msg) => {
                            warn!(tool = %tool_call.name, "Circuit breaker triggered (streaming)");
                            if let Err(e) = memory.save_session(session) {
                                warn!("Failed to save session on circuit break: {e}");
                            }
                            // Fire AgentLoopEnd hook on circuit break
                            if let Some(hook_reg) = hooks {
                                let ctx = crate::hooks::HookContext {
                                    agent_name: &manifest.name,
                                    agent_id: agent_id_str.as_str(),
                                    event: openfang_types::agent::HookEvent::AgentLoopEnd,
                                    data: serde_json::json!({
                                        "reason": "circuit_break",
                                        "error": msg.as_str(),
                                    }),
                                };
                                let _ = hook_reg.fire(&ctx);
                            }
                            return Err(OpenFangError::Internal(msg.clone()));
                        }
                        LoopGuardVerdict::Block(msg) => {
                            warn!(tool = %tool_call.name, "Tool call blocked by loop guard (streaming)");
                            tool_result_blocks.push(ContentBlock::ToolResult {
                                tool_use_id: tool_call.id.clone(),
                                tool_name: tool_call.name.clone(),
                                content: msg.clone(),
                                is_error: true,
                            });
                            continue;
                        }
                        _ => {} // Allow or Warn — proceed with execution
                    }

                    debug!(tool = %tool_call.name, id = %tool_call.id, "Executing tool (streaming)");

                    // Notify phase: ToolUse
                    if let Some(cb) = on_phase {
                        let sanitized: String = tool_call
                            .name
                            .chars()
                            .filter(|c| !c.is_control())
                            .take(64)
                            .collect();
                        cb(LoopPhase::ToolUse {
                            tool_name: sanitized,
                        });
                    }

                    // Fire BeforeToolCall hook (can block execution)
                    if let Some(hook_reg) = hooks {
                        let ctx = crate::hooks::HookContext {
                            agent_name: &manifest.name,
                            agent_id: &caller_id_str,
                            event: openfang_types::agent::HookEvent::BeforeToolCall,
                            data: serde_json::json!({
                                "tool_name": &tool_call.name,
                                "input": &tool_call.input,
                            }),
                        };
                        if let Err(reason) = hook_reg.fire(&ctx) {
                            tool_result_blocks.push(ContentBlock::ToolResult {
                                tool_use_id: tool_call.id.clone(),
                                tool_name: tool_call.name.clone(),
                                content: format!(
                                    "Hook blocked tool '{}': {}",
                                    tool_call.name, reason
                                ),
                                is_error: true,
                            });
                            continue;
                        }
                    }

                    // Timeout-wrapped execution
                    let result = match tokio::time::timeout(
                        Duration::from_secs(TOOL_TIMEOUT_SECS),
                        tool_runner.execute_tool_call(
                            &tool_call.id,
                            &tool_call.name,
                            &tool_call.input,
                        ),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => {
                            warn!(tool = %tool_call.name, "Tool execution timed out after {}s (streaming)", TOOL_TIMEOUT_SECS);
                            openfang_types::tool::ToolResult {
                                tool_use_id: tool_call.id.clone(),
                                content: format!(
                                    "Tool '{}' timed out after {}s.",
                                    tool_call.name, TOOL_TIMEOUT_SECS
                                ),
                                is_error: true,
                            }
                        }
                    };

                    // Fire AfterToolCall hook
                    if let Some(hook_reg) = hooks {
                        let ctx = crate::hooks::HookContext {
                            agent_name: &manifest.name,
                            agent_id: caller_id_str.as_str(),
                            event: openfang_types::agent::HookEvent::AfterToolCall,
                            data: serde_json::json!({
                                "tool_name": &tool_call.name,
                                "result": &result.content,
                                "is_error": result.is_error,
                            }),
                        };
                        let _ = hook_reg.fire(&ctx);
                    }

                    // Dynamic truncation based on context budget (replaces flat MAX_TOOL_RESULT_CHARS)
                    let content = truncate_tool_result_dynamic(&result.content, &context_budget);

                    // Append warning if verdict was Warn
                    let final_content = if let LoopGuardVerdict::Warn(ref warn_msg) = verdict {
                        format!("{content}\n\n[LOOP GUARD] {warn_msg}")
                    } else {
                        content
                    };

                    // Notify client of tool execution result (detect dead consumer)
                    let preview: String = final_content.chars().take(300).collect();
                    if stream_tx
                        .send(StreamEvent::ToolExecutionResult {
                            name: tool_call.name.clone(),
                            result_preview: preview,
                            is_error: result.is_error,
                        })
                        .await
                        .is_err()
                    {
                        warn!(agent = %manifest.name, "Stream consumer disconnected — continuing tool loop but will not stream further");
                    }

                    tool_result_blocks.push(ContentBlock::ToolResult {
                        tool_use_id: result.tool_use_id,
                        tool_name: tool_call.name.clone(),
                        content: final_content,
                        is_error: result.is_error,
                    });
                }

                // Detect approval denials and inject guidance to prevent infinite retry loops
                let denial_count = tool_result_blocks
                    .iter()
                    .filter(|b| {
                        matches!(b, ContentBlock::ToolResult { content, is_error: true, .. }
                        if content.contains("requires human approval and was denied"))
                    })
                    .count();
                if denial_count > 0 {
                    tool_result_blocks.push(ContentBlock::Text {
                        text: format!(
                            "[System: {} tool call(s) were denied by approval policy. \
                             Do NOT retry denied tools. Explain to the user what you \
                             wanted to do and that it requires their approval.]",
                            denial_count
                        ),
                        provider_metadata: None,
                    });
                }

                // Detect tool errors and inject guidance to prevent fabrication
                let error_count = tool_result_blocks
                    .iter()
                    .filter(|b| matches!(b, ContentBlock::ToolResult { is_error: true, .. }))
                    .count();
                let non_denial_errors = error_count.saturating_sub(denial_count);
                if non_denial_errors > 0 {
                    tool_result_blocks.push(ContentBlock::Text {
                        text: format!(
                            "[System: {} tool(s) returned errors. Report the error honestly \
                             to the user. Do NOT fabricate results or pretend the tool succeeded. \
                             If a search or fetch failed, tell the user it failed and suggest \
                             alternatives instead of making up data.]",
                            non_denial_errors
                        ),
                        provider_metadata: None,
                    });
                }

                let tool_results_msg = Message {
                    role: Role::User,
                    content: MessageContent::Blocks(tool_result_blocks.clone()),
                };

                // Log Tool Results (streaming)
                let tool_log = format!("Results: {:?}", tool_result_blocks);
                log_llm_event(workspace_root, "TOOL_RESULT", "", &tool_log).await;

                session.messages.push(tool_results_msg.clone());
                messages.push(tool_results_msg);

                if let Err(e) = memory.save_session(session) {
                    warn!("Failed to interim-save session: {e}");
                }
            }
            StopReason::MaxTokens => {
                consecutive_max_tokens += 1;
                if consecutive_max_tokens >= MAX_CONTINUATIONS {
                    let text = response.text();
                    let text = if text.trim().is_empty() {
                        "[Partial response — token limit reached with no text output.]".to_string()
                    } else {
                        text
                    };
                    session.messages.push(Message::assistant(&text));
                    if let Err(e) = memory.save_session(session) {
                        warn!("Failed to save session on max continuations: {e}");
                    }
                    warn!(
                        iteration,
                        consecutive_max_tokens,
                        "Max continuations reached (streaming), returning partial response"
                    );
                    // Fire AgentLoopEnd hook
                    if let Some(hook_reg) = hooks {
                        let ctx = crate::hooks::HookContext {
                            agent_name: &manifest.name,
                            agent_id: agent_id_str.as_str(),
                            event: openfang_types::agent::HookEvent::AgentLoopEnd,
                            data: serde_json::json!({
                                "iterations": iteration + 1,
                                "reason": "max_continuations",
                            }),
                        };
                        let _ = hook_reg.fire(&ctx);
                    }
                    return Ok(AgentLoopResult {
                        response: text,
                        total_usage,
                        iterations: iteration + 1,
                        cost_usd: None,
                        silent: false,
                        directives: Default::default(),
                    });
                }
                let text = response.text();
                session.messages.push(Message::assistant(&text));
                messages.push(Message::assistant(&text));
                session.messages.push(Message::user("Please continue."));
                messages.push(Message::user("Please continue."));
                warn!(iteration, "Max tokens hit (streaming), continuing");
            }
        }
    }

    if let Err(e) = memory.save_session(session) {
        warn!("Failed to save session on max iterations: {e}");
    }

    // Fire AgentLoopEnd hook on max iterations exceeded
    if let Some(hook_reg) = hooks {
        let ctx = crate::hooks::HookContext {
            agent_name: &manifest.name,
            agent_id: agent_id_str.as_str(),
            event: openfang_types::agent::HookEvent::AgentLoopEnd,
            data: serde_json::json!({
                "reason": "max_iterations_exceeded",
                "iterations": max_iterations,
            }),
        };
        let _ = hook_reg.fire(&ctx);
    }

    Err(OpenFangError::MaxIterationsExceeded(max_iterations))
}

/// Recover tool calls that LLMs output as plain text instead of the proper
/// `tool_calls` API field. Covers Groq/Llama, DeepSeek, Qwen, and Ollama models.
///
/// Supported patterns:
/// 1. `<function=tool_name>{"key":"value"}</function>`
/// 2. `<function>tool_name{"key":"value"}</function>`
/// 3. `<tool>tool_name{"key":"value"}</tool>`
/// 4. Markdown code blocks containing `tool_name {"key":"value"}`
/// 5. Backtick-wrapped `tool_name {"key":"value"}`
/// 6. `[TOOL_CALL]...[/TOOL_CALL]` blocks (JSON or arrow syntax) — issue #354
/// 7. `<tool_call>{"name":"tool","arguments":{...}}</tool_call>` — Qwen3, issue #332
/// 8. Bare JSON `{"name":"tool","arguments":{...}}` objects (last resort, only if no tags found)
/// 9. `<function name="tool" parameters="{...}" />` — XML attribute style (Groq/Llama)
/// 10. `<|plugin|>...<|endofblock|>` — Qwen/ChatGLM thinking-model format
/// 11. `Action: tool\nAction Input: {"key":"value"}` — ReAct-style (LM Studio, GPT-OSS)
/// 12. `tool_name\n{"key":"value"}` — bare name + JSON on next line (Llama 4 Scout)
/// 13. `<tool_use>{"name":"tool","arguments":{...}}</tool_use>` — Llama 3.1+ variant
///
/// Validates tool names against available tools and returns synthetic `ToolCall` entries.
fn recover_text_tool_calls(text: &str, available_tools: &[ToolDefinition]) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    let tool_names: Vec<&str> = available_tools.iter().map(|t| t.name.as_str()).collect();

    // Pattern 1: <function=TOOL_NAME>JSON_BODY</function>
    let mut search_from = 0;
    while let Some(start) = text[search_from..].find("<function=") {
        let abs_start = search_from + start;
        let after_prefix = abs_start + "<function=".len();

        // Extract tool name (ends at '>')
        let Some(name_end) = text[after_prefix..].find('>') else {
            search_from = after_prefix;
            continue;
        };
        let tool_name = &text[after_prefix..after_prefix + name_end];
        let json_start = after_prefix + name_end + 1;

        // Find closing </function>
        let Some(close_offset) = text[json_start..].find("</function>") else {
            search_from = json_start;
            continue;
        };
        let json_body = text[json_start..json_start + close_offset].trim();
        search_from = json_start + close_offset + "</function>".len();

        // Validate: tool name must be in the current visible tool surface
        if !tool_names.contains(&tool_name) {
            warn!(
                tool = tool_name,
                "Text-based tool call for unknown tool — skipping"
            );
            continue;
        }

        // Parse JSON input
        let input: serde_json::Value = match serde_json::from_str(json_body) {
            Ok(v) => v,
            Err(e) => {
                warn!(tool = tool_name, error = %e, "Failed to parse text-based tool call JSON — skipping");
                continue;
            }
        };

        info!(
            tool = tool_name,
            "Recovered text-based tool call → synthetic ToolUse"
        );
        calls.push(ToolCall {
            id: format!("recovered_{}", uuid::Uuid::new_v4()),
            name: tool_name.to_string(),
            input,
        });
    }

    // Pattern 2: <function>TOOL_NAME{JSON_BODY}</function>
    // (Groq/Llama variant — tool name immediately followed by JSON object)
    search_from = 0;
    while let Some(start) = text[search_from..].find("<function>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<function>".len();

        // Find closing </function>
        let Some(close_offset) = text[after_tag..].find("</function>") else {
            search_from = after_tag;
            continue;
        };
        let inner = &text[after_tag..after_tag + close_offset];
        search_from = after_tag + close_offset + "</function>".len();

        // The inner content is "tool_name{json}" — find the first '{' to split
        let Some(brace_pos) = inner.find('{') else {
            continue;
        };
        let tool_name = inner[..brace_pos].trim();
        let json_body = inner[brace_pos..].trim();

        if tool_name.is_empty() {
            continue;
        }

        // Validate: tool name must be in the current visible tool surface
        if !tool_names.contains(&tool_name) {
            warn!(
                tool = tool_name,
                "Text-based tool call (variant 2) for unknown tool — skipping"
            );
            continue;
        }

        // Parse JSON input
        let input: serde_json::Value = match serde_json::from_str(json_body) {
            Ok(v) => v,
            Err(e) => {
                warn!(tool = tool_name, error = %e, "Failed to parse text-based tool call JSON (variant 2) — skipping");
                continue;
            }
        };

        // Avoid duplicates if pattern 1 already captured this call
        if calls
            .iter()
            .any(|c| c.name == tool_name && c.input == input)
        {
            continue;
        }

        info!(
            tool = tool_name,
            "Recovered text-based tool call (variant 2) → synthetic ToolUse"
        );
        calls.push(ToolCall {
            id: format!("recovered_{}", uuid::Uuid::new_v4()),
            name: tool_name.to_string(),
            input,
        });
    }

    // Pattern 3: <tool>TOOL_NAME{JSON}</tool>  (Qwen / DeepSeek variant)
    search_from = 0;
    while let Some(start) = text[search_from..].find("<tool>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<tool>".len();

        let Some(close_offset) = text[after_tag..].find("</tool>") else {
            search_from = after_tag;
            continue;
        };
        let inner = &text[after_tag..after_tag + close_offset];
        search_from = after_tag + close_offset + "</tool>".len();

        let Some(brace_pos) = inner.find('{') else {
            continue;
        };
        let tool_name = inner[..brace_pos].trim();
        let json_body = inner[brace_pos..].trim();

        if tool_name.is_empty() || !tool_names.contains(&tool_name) {
            continue;
        }

        let input: serde_json::Value = match serde_json::from_str(json_body) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if calls
            .iter()
            .any(|c| c.name == tool_name && c.input == input)
        {
            continue;
        }

        info!(
            tool = tool_name,
            "Recovered text-based tool call (<tool> variant) → synthetic ToolUse"
        );
        calls.push(ToolCall {
            id: format!("recovered_{}", uuid::Uuid::new_v4()),
            name: tool_name.to_string(),
            input,
        });
    }

    // Pattern 4: Markdown code blocks containing tool_name {JSON}
    // Matches: ```\nexec {"command":"ls"}\n``` or ```bash\nexec {"command":"ls"}\n```
    {
        let mut in_block = false;
        let mut block_content = String::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                if in_block {
                    // End of block — try to extract tool call from content
                    let content = block_content.trim();
                    if let Some(brace_pos) = content.find('{') {
                        let potential_tool = content[..brace_pos].trim();
                        if tool_names.contains(&potential_tool) {
                            if let Ok(input) = serde_json::from_str::<serde_json::Value>(
                                content[brace_pos..].trim(),
                            ) {
                                if !calls
                                    .iter()
                                    .any(|c| c.name == potential_tool && c.input == input)
                                {
                                    info!(
                                        tool = potential_tool,
                                        "Recovered tool call from markdown code block"
                                    );
                                    calls.push(ToolCall {
                                        id: format!("recovered_{}", uuid::Uuid::new_v4()),
                                        name: potential_tool.to_string(),
                                        input,
                                    });
                                }
                            }
                        }
                    }
                    block_content.clear();
                    in_block = false;
                } else {
                    in_block = true;
                    block_content.clear();
                }
            } else if in_block {
                if !block_content.is_empty() {
                    block_content.push('\n');
                }
                block_content.push_str(trimmed);
            }
        }
    }

    // Pattern 5: Backtick-wrapped tool call: `tool_name {"key":"value"}`
    {
        let parts: Vec<&str> = text.split('`').collect();
        // Every odd-indexed element is inside backticks
        for chunk in parts.iter().skip(1).step_by(2) {
            let trimmed = chunk.trim();
            if let Some(brace_pos) = trimmed.find('{') {
                let potential_tool = trimmed[..brace_pos].trim();
                if !potential_tool.is_empty()
                    && !potential_tool.contains(' ')
                    && tool_names.contains(&potential_tool)
                {
                    if let Ok(input) =
                        serde_json::from_str::<serde_json::Value>(trimmed[brace_pos..].trim())
                    {
                        if !calls
                            .iter()
                            .any(|c| c.name == potential_tool && c.input == input)
                        {
                            info!(
                                tool = potential_tool,
                                "Recovered tool call from backtick-wrapped text"
                            );
                            calls.push(ToolCall {
                                id: format!("recovered_{}", uuid::Uuid::new_v4()),
                                name: potential_tool.to_string(),
                                input,
                            });
                        }
                    }
                }
            }
        }
    }

    // Pattern 6: [TOOL_CALL]...[/TOOL_CALL] blocks (Ollama models like Qwen, issue #354)
    // Handles both JSON args and custom `{tool => "name", args => {--key "value"}}` syntax.
    search_from = 0;
    while let Some(start) = text[search_from..].find("[TOOL_CALL]") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "[TOOL_CALL]".len();

        let Some(close_offset) = text[after_tag..].find("[/TOOL_CALL]") else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + "[/TOOL_CALL]".len();

        // Try standard JSON first: {"name":"tool","arguments":{...}}
        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, &tool_names) {
            if !calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from [TOOL_CALL] block (JSON)"
                );
                calls.push(ToolCall {
                    id: format!("recovered_{}", uuid::Uuid::new_v4()),
                    name: tool_name,
                    input,
                });
            }
            continue;
        }

        // Custom arrow syntax: {tool => "name", args => {--key "value"}}
        if let Some((tool_name, input)) = parse_arrow_syntax_tool_call(inner, &tool_names) {
            if !calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from [TOOL_CALL] block (arrow syntax)"
                );
                calls.push(ToolCall {
                    id: format!("recovered_{}", uuid::Uuid::new_v4()),
                    name: tool_name,
                    input,
                });
            }
        }
    }

    // Pattern 7: <tool_call>JSON</tool_call> (Qwen3 models on Ollama, issue #332)
    search_from = 0;
    while let Some(start) = text[search_from..].find("<tool_call>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<tool_call>".len();

        let Some(close_offset) = text[after_tag..].find("</tool_call>") else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + "</tool_call>".len();

        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, &tool_names) {
            if !calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from <tool_call> block"
                );
                calls.push(ToolCall {
                    id: format!("recovered_{}", uuid::Uuid::new_v4()),
                    name: tool_name,
                    input,
                });
            }
        }
    }

    // Pattern 9: <function name="tool" parameters="{...}" /> — XML attribute style
    // Groq/Llama sometimes emit self-closing XML with name/parameters attributes.
    // The parameters value is HTML-entity-escaped JSON (&quot; etc.).
    {
        use regex_lite::Regex;
        // Match both self-closing <function ... /> and <function ...></function>
        let re =
            Regex::new(r#"<function\s+name="([^"]+)"\s+parameters="([^"]*)"[^/]*/?>"#).unwrap();
        for caps in re.captures_iter(text) {
            let tool_name = caps.get(1).unwrap().as_str();
            let raw_params = caps.get(2).unwrap().as_str();

            if !tool_names.contains(&tool_name) {
                warn!(
                    tool = tool_name,
                    "XML-attribute tool call for unknown tool — skipping"
                );
                continue;
            }

            // Unescape HTML entities (&quot; &amp; &lt; &gt; &apos;)
            let unescaped = raw_params
                .replace("&quot;", "\"")
                .replace("&amp;", "&")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&apos;", "'");

            let input: serde_json::Value = match serde_json::from_str(&unescaped) {
                Ok(v) => v,
                Err(e) => {
                    warn!(tool = tool_name, error = %e, "Failed to parse XML-attribute tool call params — skipping");
                    continue;
                }
            };

            if calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                continue;
            }

            info!(
                tool = tool_name,
                "Recovered XML-attribute tool call → synthetic ToolUse"
            );
            calls.push(ToolCall {
                id: format!("recovered_{}", uuid::Uuid::new_v4()),
                name: tool_name.to_string(),
                input,
            });
        }
    }

    // Pattern 10: <|plugin|>...<|endofblock|> (Qwen/ChatGLM thinking-model format)
    search_from = 0;
    while let Some(start) = text[search_from..].find("<|plugin|>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<|plugin|>".len();

        let close_tag = "<|endofblock|>";
        let Some(close_offset) = text[after_tag..].find(close_tag) else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + close_tag.len();

        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, &tool_names) {
            if !calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from <|plugin|> block"
                );
                calls.push(ToolCall {
                    id: format!("recovered_{}", uuid::Uuid::new_v4()),
                    name: tool_name,
                    input,
                });
            }
        }
    }

    // Pattern 11: Action: tool_name\nAction Input: {JSON} (ReAct-style, LM Studio / GPT-OSS)
    {
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i].trim();
            if let Some(tool_part) = line
                .strip_prefix("Action:")
                .or_else(|| line.strip_prefix("action:"))
            {
                let tool_name = tool_part.trim();
                if tool_names.contains(&tool_name) {
                    // Look for "Action Input:" on the next line(s)
                    if i + 1 < lines.len() {
                        let next = lines[i + 1].trim();
                        if let Some(json_part) = next
                            .strip_prefix("Action Input:")
                            .or_else(|| next.strip_prefix("action input:"))
                            .or_else(|| next.strip_prefix("action_input:"))
                        {
                            let json_str = json_part.trim();
                            if let Ok(input) = serde_json::from_str::<serde_json::Value>(json_str) {
                                if !calls
                                    .iter()
                                    .any(|c| c.name == tool_name && c.input == input)
                                {
                                    info!(
                                        tool = tool_name,
                                        "Recovered tool call from Action/Action Input pattern"
                                    );
                                    calls.push(ToolCall {
                                        id: format!("recovered_{}", uuid::Uuid::new_v4()),
                                        name: tool_name.to_string(),
                                        input,
                                    });
                                }
                            }
                            i += 2;
                            continue;
                        }
                    }
                }
            }
            i += 1;
        }
    }

    // Pattern 12: tool_name\n{"key":"value"} — bare name + JSON on next line (Llama 4 Scout)
    {
        let lines: Vec<&str> = text.lines().collect();
        for i in 0..lines.len().saturating_sub(1) {
            let name_line = lines[i].trim();
            // Tool name must be a single word matching a known tool
            if name_line.contains(' ') || name_line.contains('{') || name_line.is_empty() {
                continue;
            }
            if !tool_names.contains(&name_line) {
                continue;
            }
            // Next line must be valid JSON
            let json_line = lines[i + 1].trim();
            if !json_line.starts_with('{') {
                continue;
            }
            if let Ok(input) = serde_json::from_str::<serde_json::Value>(json_line) {
                if !calls
                    .iter()
                    .any(|c| c.name == name_line && c.input == input)
                {
                    info!(
                        tool = name_line,
                        "Recovered tool call from name+JSON line pair"
                    );
                    calls.push(ToolCall {
                        id: format!("recovered_{}", uuid::Uuid::new_v4()),
                        name: name_line.to_string(),
                        input,
                    });
                }
            }
        }
    }

    // Pattern 13: <tool_use>JSON</tool_use> (Llama 3.1+ variant)
    search_from = 0;
    while let Some(start) = text[search_from..].find("<tool_use>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<tool_use>".len();

        let Some(close_offset) = text[after_tag..].find("</tool_use>") else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + "</tool_use>".len();

        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, &tool_names) {
            if !calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from <tool_use> block"
                );
                calls.push(ToolCall {
                    id: format!("recovered_{}", uuid::Uuid::new_v4()),
                    name: tool_name,
                    input,
                });
            }
        }
    }

    // Pattern 8: Bare JSON tool call objects in text (common Ollama fallback)
    // Matches: {"name":"tool_name","arguments":{"key":"value"}} not already inside tags
    // Only try this if no calls were found by tag-based patterns, to avoid false positives.
    if calls.is_empty() {
        // Scan for JSON objects that look like tool calls
        let mut scan_from = 0;
        while let Some(brace_start) = text[scan_from..].find('{') {
            let abs_brace = scan_from + brace_start;
            // Try to parse a JSON object starting here
            if let Some((tool_name, input)) =
                try_parse_bare_json_tool_call(&text[abs_brace..], &tool_names)
            {
                if !calls
                    .iter()
                    .any(|c| c.name == tool_name && c.input == input)
                {
                    info!(
                        tool = tool_name.as_str(),
                        "Recovered tool call from bare JSON object in text"
                    );
                    calls.push(ToolCall {
                        id: format!("recovered_{}", uuid::Uuid::new_v4()),
                        name: tool_name,
                        input,
                    });
                }
            }
            scan_from = abs_brace + 1;
        }
    }

    calls
}

fn format_llm_input_log(system_prompt: &str, messages: &[Message]) -> String {
    let system_prompt_display = truncate_for_log(system_prompt, MAX_SYSTEM_PROMPT_LOG_CHARS);
    let messages_display = format_messages_for_log(messages, MAX_MESSAGES_LOG_CHARS);
    format!(
        "System Prompt:\n{}\n\nMessages ({} total):\n{}",
        system_prompt_display,
        messages.len(),
        messages_display
    )
}

fn format_messages_for_log(messages: &[Message], max_chars: usize) -> String {
    if messages.is_empty() {
        return "[none]".to_string();
    }

    let mut rendered_rev = Vec::new();
    let mut used_chars = 0usize;
    let mut omitted = 0usize;

    for (idx, message) in messages.iter().enumerate().rev() {
        let rendered = format_message_for_log(idx, message);
        let rendered_len = rendered.chars().count();
        if !rendered_rev.is_empty() && used_chars + rendered_len > max_chars {
            omitted = idx + 1;
            break;
        }
        used_chars += rendered_len;
        rendered_rev.push(rendered);
    }

    rendered_rev.reverse();
    let mut out = String::new();
    if omitted > 0 {
        out.push_str(&format!(
            "... [{} earlier message(s) omitted from log] ...\n\n",
            omitted
        ));
    }
    out.push_str(&rendered_rev.join("\n\n"));
    out
}

fn format_message_for_log(index: usize, message: &Message) -> String {
    let role = match message.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    };

    let content = match &message.content {
        MessageContent::Text(text) => truncate_for_log(text, MAX_MESSAGE_TEXT_LOG_CHARS),
        MessageContent::Blocks(blocks) => format_blocks_for_log(blocks),
    };

    format!("[#{index}] {role}\n{content}")
}

fn format_blocks_for_log(blocks: &[ContentBlock]) -> String {
    if blocks.is_empty() {
        return "[no blocks]".to_string();
    }

    let mut lines = Vec::with_capacity(blocks.len());
    for (idx, block) in blocks.iter().enumerate() {
        let line = match block {
            ContentBlock::Text { text, .. } => format!(
                "- block[{idx}] text: {}",
                truncate_for_log(text, MAX_BLOCK_TEXT_LOG_CHARS)
            ),
            ContentBlock::Image { media_type, data } => format!(
                "- block[{idx}] image: media_type={media_type}, base64_chars={}",
                data.len()
            ),
            ContentBlock::ToolUse {
                id, name, input, ..
            } => format!(
                "- block[{idx}] tool_use: id={id}, name={name}, input={}",
                truncate_for_log(&input.to_string(), MAX_BLOCK_TEXT_LOG_CHARS)
            ),
            ContentBlock::ToolResult {
                tool_use_id,
                tool_name,
                content,
                is_error,
            } => format!(
                "- block[{idx}] tool_result: tool_use_id={tool_use_id}, tool_name={tool_name}, is_error={is_error}, content={}",
                truncate_for_log(content, MAX_BLOCK_TEXT_LOG_CHARS)
            ),
            ContentBlock::Thinking { thinking } => format!(
                "- block[{idx}] thinking: {}",
                truncate_for_log(thinking, MAX_BLOCK_TEXT_LOG_CHARS)
            ),
            ContentBlock::Unknown => format!("- block[{idx}] unknown"),
        };
        lines.push(line);
    }
    lines.join("\n")
}

fn truncate_for_log(content: &str, max_chars: usize) -> String {
    let char_count = content.chars().count();
    if char_count <= max_chars {
        return content.to_string();
    }

    let end = content
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(content.len());
    format!(
        "{}... [truncated, {} total chars]",
        &content[..end],
        char_count
    )
}

/// Log an LLM event (input, output, or tool result) to the agent's workspace.
async fn log_llm_event(
    workspace_root: Option<&Path>,
    event_type: &str,
    model: &str,
    content: &str,
) {
    // Check if logging is explicitly disabled via environment variable
    if let Ok(val) = std::env::var("OPENFANG_LLM_LOG") {
        if val == "0" || val.to_lowercase() == "false" {
            return;
        }
    }

    let Some(root) = workspace_root else { return };
    let log_dir = root.join("logs");
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        warn!("Failed to create logs directory: {e}");
        return;
    }

    let log_path = log_dir.join("llm.log");
    let mut file = match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(f) => f,
        Err(e) => {
            warn!("Failed to open llm.log for writing: {e}");
            return;
        }
    };

    let timestamp = chrono::Local::now().to_rfc3339();
    let separator = "=".repeat(80);
    let sub_separator = "-".repeat(80);

    // Differentiate truncation limits: TOOL_RESULT is smaller to keep logs concise
    let max_chars = if event_type == "TOOL_RESULT" {
        MAX_TOOL_RESULT_LOG_CHARS
    } else {
        MAX_LLM_IO_LOG_CHARS
    };

    // Use char boundary for UTF-8 truncation
    let (display_content, truncated) = if content.len() > max_chars {
        let truncated_str: String = content.chars().take(max_chars).collect();
        (truncated_str, true)
    } else {
        (content.to_string(), false)
    };

    let header = match event_type {
        "INPUT" => format!(
            "\n{}\n[{}] >>> INPUT (Model: {})\n{}\n",
            separator, timestamp, model, sub_separator
        ),
        "MEMORY_TRACE" => format!("\n[{}] *** MEMORY TRACE\n{}\n", timestamp, sub_separator),
        "OUTPUT" => format!("\n[{}] <<< OUTPUT\n{}\n", timestamp, sub_separator),
        "TOOL_RESULT" => format!("\n[{}] === TOOL RESULT\n{}\n", timestamp, sub_separator),
        _ => format!(
            "\n[{}] EVENT: {}\n{}\n",
            timestamp, event_type, sub_separator
        ),
    };

    if let Err(e) = write!(file, "{}{}", header, display_content) {
        warn!("Failed to write to llm.log: {e}");
    }

    if truncated {
        let _ = write!(
            file,
            "\n[... CONTENT TRUNCATED AT {} CHARS ...]\n",
            max_chars
        );
    }

    if event_type == "TOOL_RESULT" || event_type == "OUTPUT" {
        let _ = write!(file, "\n{}\n", separator);
    } else {
        let _ = writeln!(file);
    }
}

async fn emit_prompt_memory_context_trace(
    workspace_root: Option<&Path>,
    agent_name: &str,
    agent_id: AgentId,
    kernel: Option<&Arc<dyn KernelHandle>>,
    trace: &PromptMemoryContextTrace,
) {
    if let Some(telemetry) = build_prompt_memory_context_trace_telemetry(trace) {
        let selected_fused_recall = serde_json::to_string(&telemetry.selected_fused_recall)
            .unwrap_or_else(|_| "[]".to_string());
        info!(
            agent = %agent_name,
            agent_id = %agent_id,
            semantic_mode = telemetry.semantic_mode.as_str(),
            semantic_candidates = telemetry.semantic_candidates,
            shared_candidates = telemetry.shared_candidates,
            maintenance_signals = telemetry.maintenance_signals,
            attention_signals = telemetry.attention_signals,
            session_summaries = telemetry.session_summaries,
            selected_fused_recall_count = telemetry.selected_fused_recall.len(),
            selected_fused_recall = %selected_fused_recall,
            "Prompt memory context trace"
        );
        if let Some(kernel) = kernel {
            let detail = format!(
                "semantic_mode={} semantic_candidates={} shared_candidates={} maintenance_signals={} attention_signals={} session_summaries={} selected_fused_recall_count={}",
                telemetry.semantic_mode.as_str(),
                telemetry.semantic_candidates,
                telemetry.shared_candidates,
                telemetry.maintenance_signals,
                telemetry.attention_signals,
                telemetry.session_summaries,
                telemetry.selected_fused_recall.len(),
            );
            let outcome =
                serde_json::to_string(&telemetry).unwrap_or_else(|_| "{}".to_string());
            if let Err(error) = kernel.record_audit_event(
                &agent_id.to_string(),
                AuditAction::MemoryTrace,
                &detail,
                &outcome,
            ) {
                warn!(agent_id = %agent_id, error = %error, "Failed to record memory trace audit event");
            }
        }
    }

    if let Some(memory_trace) = render_prompt_memory_context_trace(trace) {
        log_llm_event(workspace_root, "MEMORY_TRACE", "", &memory_trace).await;
    }
}

/// Parse a JSON object that represents a tool call.
/// Supports formats:
/// - `{"name":"tool","arguments":{"key":"value"}}`
/// - `{"name":"tool","parameters":{"key":"value"}}`
/// - `{"function":"tool","arguments":{"key":"value"}}`
/// - `{"tool":"tool_name","args":{"key":"value"}}`
fn parse_json_tool_call_object(
    text: &str,
    tool_names: &[&str],
) -> Option<(String, serde_json::Value)> {
    let obj: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = obj.as_object()?;

    // Extract tool name from various field names
    let name = obj
        .get("name")
        .or_else(|| obj.get("function"))
        .or_else(|| obj.get("tool"))
        .and_then(|v| v.as_str())?;

    if !tool_names.contains(&name) {
        return None;
    }

    // Extract arguments from various field names
    let args = obj
        .get("arguments")
        .or_else(|| obj.get("parameters"))
        .or_else(|| obj.get("args"))
        .or_else(|| obj.get("input"))
        .cloned()
        .unwrap_or(serde_json::json!({}));

    // If arguments is a string (some models stringify it), try to parse it
    let args = if let Some(s) = args.as_str() {
        serde_json::from_str(s).unwrap_or(serde_json::json!({}))
    } else {
        args
    };

    Some((name.to_string(), args))
}

/// Parse the custom arrow syntax used by some Ollama models:
/// `{tool => "name", args => {--key "value"}}` or `{tool => "name", args => {"key":"value"}}`
fn parse_arrow_syntax_tool_call(
    text: &str,
    tool_names: &[&str],
) -> Option<(String, serde_json::Value)> {
    // Extract tool name: look for `tool => "name"` or `tool=>"name"`
    let tool_marker_pos = text.find("tool")?;
    let after_tool = &text[tool_marker_pos + 4..];
    // Skip whitespace and `=>`
    let after_arrow = after_tool.trim_start();
    let after_arrow = after_arrow.strip_prefix("=>")?;
    let after_arrow = after_arrow.trim_start();

    // Extract quoted tool name
    let tool_name = if let Some(stripped) = after_arrow.strip_prefix('"') {
        let end_quote = stripped.find('"')?;
        &stripped[..end_quote]
    } else {
        // Unquoted: take until comma, whitespace, or '}'
        let end = after_arrow
            .find(|c: char| c == ',' || c == '}' || c.is_whitespace())
            .unwrap_or(after_arrow.len());
        &after_arrow[..end]
    };

    if tool_name.is_empty() || !tool_names.contains(&tool_name) {
        return None;
    }

    // Extract args: look for `args => {` or `args=>{`
    let args_value = if let Some(args_pos) = text.find("args") {
        let after_args = &text[args_pos + 4..];
        let after_args = after_args.trim_start();
        let after_args = after_args.strip_prefix("=>")?;
        let after_args = after_args.trim_start();

        if after_args.starts_with('{') {
            // Try standard JSON parse first
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(after_args) {
                v
            } else {
                // Parse `--key "value"` / `--key value` style args
                parse_dash_dash_args(after_args)
            }
        } else {
            serde_json::json!({})
        }
    } else {
        serde_json::json!({})
    };

    Some((tool_name.to_string(), args_value))
}

/// Parse `{--key "value", --flag}` or `{--command "ls -F /"}` style arguments
/// into a JSON object.
fn parse_dash_dash_args(text: &str) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    // Strip outer braces — find matching close brace
    let inner = if text.starts_with('{') {
        let mut depth = 0;
        let mut end = text.len();
        for (i, c) in text.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        text[1..end].trim()
    } else {
        text.trim()
    };

    // Parse --key "value" or --key value pairs
    let mut remaining = inner;
    while let Some(dash_pos) = remaining.find("--") {
        remaining = &remaining[dash_pos + 2..];

        // Extract key: runs until whitespace, '=', '"', or end
        let key_end = remaining
            .find(|c: char| c.is_whitespace() || c == '=' || c == '"')
            .unwrap_or(remaining.len());
        let key = &remaining[..key_end];
        if key.is_empty() {
            continue;
        }
        remaining = &remaining[key_end..];
        remaining = remaining.trim_start();

        // Skip optional '='
        if remaining.starts_with('=') {
            remaining = remaining[1..].trim_start();
        }

        // Extract value
        if remaining.starts_with('"') {
            // Quoted value — find closing quote
            if let Some(end_quote) = remaining[1..].find('"') {
                let value = &remaining[1..1 + end_quote];
                map.insert(
                    key.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
                remaining = &remaining[2 + end_quote..];
            } else {
                // Unclosed quote — take rest
                let value = &remaining[1..];
                map.insert(
                    key.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
                break;
            }
        } else {
            // Unquoted value — take until next --, comma, }, or end
            let val_end = remaining
                .find([',', '}'])
                .or_else(|| remaining.find("--"))
                .unwrap_or(remaining.len());
            let value = remaining[..val_end].trim();
            if !value.is_empty() {
                map.insert(
                    key.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
            } else {
                // Flag with no value — set to true
                map.insert(key.to_string(), serde_json::Value::Bool(true));
            }
            remaining = &remaining[val_end..];
        }

        // Skip comma separator
        remaining = remaining.trim_start();
        if remaining.starts_with(',') {
            remaining = remaining[1..].trim_start();
        }
    }

    serde_json::Value::Object(map)
}

/// Try to parse a bare JSON object as a tool call.
/// The JSON must have a "name"/"function"/"tool" field matching a known tool.
fn try_parse_bare_json_tool_call(
    text: &str,
    tool_names: &[&str],
) -> Option<(String, serde_json::Value)> {
    // Find the end of this JSON object by counting braces
    let mut depth = 0;
    let mut end = 0;
    for (i, c) in text.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    if end == 0 {
        return None;
    }

    parse_json_tool_call_object(&text[..end], tool_names)
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! tool_definition {
        ($($tt:tt)*) => {
            ToolDefinition {
                defer_loading: false,
                $($tt)*
            }
        };
    }
    use crate::llm_driver::{CompletionResponse, LlmError};
    use async_trait::async_trait;
    use openfang_types::memory::{
        MemoryContextRecallMode, MemoryContextSource, MemoryFreshness, PromptMemoryContextBuildOptions,
        PromptMemoryContextTrace, build_prompt_memory_context,
        rank_governed_memory_context_candidates_for_query,
        rank_semantic_memory_context_candidates, render_governed_memory_orchestration_signals_for_query,
        render_memory_cleanup_orchestration_signals, render_prompt_memory_context_trace,
        render_recent_session_summaries_from_entries,
    };
    use openfang_types::tool::ToolCall;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn test_max_iterations_constant() {
        assert_eq!(MAX_ITERATIONS, 50);
    }

    #[test]
    fn test_retry_constants() {
        assert_eq!(MAX_RETRIES, 3);
        assert_eq!(BASE_RETRY_DELAY_MS, 1000);
    }

    #[test]
    fn test_dynamic_truncate_short_unchanged() {
        use crate::context_budget::{ContextBudget, truncate_tool_result_dynamic};
        let budget = ContextBudget::new(200_000);
        let short = "Hello, world!";
        assert_eq!(truncate_tool_result_dynamic(short, &budget), short);
    }

    #[test]
    fn test_dynamic_truncate_over_limit() {
        use crate::context_budget::{ContextBudget, truncate_tool_result_dynamic};
        let budget = ContextBudget::new(200_000);
        let long = "x".repeat(budget.per_result_cap() + 10_000);
        let result = truncate_tool_result_dynamic(&long, &budget);
        assert!(result.len() <= budget.per_result_cap() + 200);
        assert!(result.contains("[TRUNCATED:"));
    }

    #[test]
    fn test_dynamic_truncate_newline_boundary() {
        use crate::context_budget::{ContextBudget, truncate_tool_result_dynamic};
        // Small budget to force truncation
        let budget = ContextBudget::new(1_000);
        let content = (0..200)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = truncate_tool_result_dynamic(&content, &budget);
        // Should break at a newline, not mid-line
        let before_marker = result.split("[TRUNCATED:").next().unwrap();
        let trimmed = before_marker.trim_end();
        assert!(!trimmed.is_empty());
    }

    #[test]
    fn test_rank_semantic_memory_context_candidates_deduplicates_and_labels() {
        let agent_id = AgentId::new();
        let mut metadata = HashMap::new();
        metadata.insert(
            "key".to_string(),
            serde_json::Value::String("pref.editor".to_string()),
        );

        let memory = openfang_types::memory::MemoryFragment {
            id: openfang_types::memory::MemoryId::new(),
            agent_id,
            content: "Use rustfmt only when explicitly requested.".to_string(),
            embedding: None,
            metadata,
            source: openfang_types::memory::MemorySource::UserProvided,
            confidence: 1.0,
            created_at: chrono::Utc::now(),
            accessed_at: chrono::Utc::now(),
            access_count: 0,
            scope: "preferences".to_string(),
        };

        let rendered = rank_semantic_memory_context_candidates(&[memory.clone(), memory], 5);
        assert_eq!(rendered.len(), 1);
        assert!(rendered[0].rendered.starts_with("Semantic memory "));
        assert!(rendered[0].rendered.contains("[pref.editor]"));
        assert!(rendered[0].rendered.contains("rustfmt"));
    }

    #[test]
    fn test_build_prompt_memory_context_interleaves_semantic_and_shared_memory() {
        let now = chrono::Utc::now();
        let agent_id = AgentId::new();
        let semantic_memory = openfang_types::memory::MemoryFragment {
            id: openfang_types::memory::MemoryId::new(),
            agent_id,
            content: "Use concise summaries first.".to_string(),
            embedding: None,
            metadata: HashMap::from([(
                "key".to_string(),
                serde_json::Value::String("pref.reply.style".to_string()),
            )]),
            source: openfang_types::memory::MemorySource::UserProvided,
            confidence: 1.0,
            created_at: now,
            accessed_at: now,
            access_count: 0,
            scope: "preferences".to_string(),
        };
        let governed_metadata = openfang_types::memory::MemoryRecordMetadata {
            schema_version: openfang_types::memory::MEMORY_METADATA_SCHEMA_VERSION,
            key: "project.alpha.status".to_string(),
            namespace: "project".to_string(),
            kind: "project_state".to_string(),
            tags: vec!["project".to_string(), "alpha".to_string()],
            freshness: MemoryFreshness::Rolling,
            source: "memory_store_tool".to_string(),
            updated_at: now,
        };
        let entries = vec![
            (
                "project.alpha.status".to_string(),
                serde_json::json!("Alpha launch is blocked on QA signoff."),
            ),
            (
                openfang_types::memory::memory_metadata_key("project.alpha.status").unwrap(),
                serde_json::to_value(&governed_metadata).unwrap(),
            ),
        ];

        let rendered = build_prompt_memory_context(
            "What is blocking the alpha launch?",
            MemoryContextRecallMode::Hybrid,
            &[semantic_memory],
            &entries,
            &[],
            &PromptMemoryContextBuildOptions::default(),
            now,
        );

        assert_eq!(rendered.recalled_memories.len(), 2);
        assert!(rendered.recalled_memories[0].starts_with("Shared memory [project.alpha.status]"));
        assert!(rendered.recalled_memories[1].starts_with("Semantic memory "));
        assert_eq!(rendered.trace.semantic_candidates, 1);
        assert_eq!(rendered.trace.shared_candidates, 1);
        assert_eq!(rendered.trace.fused_candidates.len(), 2);
        assert_eq!(
            rendered.trace.fused_candidates[0].source,
            MemoryContextSource::Shared
        );
        assert!(rendered.trace.fused_candidates[0].source_weight > 1.0);
        assert_eq!(
            rendered.trace.fused_candidates[1].source,
            MemoryContextSource::Semantic
        );
    }

    #[test]
    fn test_render_prompt_memory_context_trace_includes_source_counts_and_selected_ranks() {
        let trace = PromptMemoryContextTrace {
            semantic_mode: MemoryContextRecallMode::Hybrid,
            semantic_candidates: 2,
            shared_candidates: 1,
            fused_candidates: vec![
                openfang_types::memory::FusedMemoryContextCandidate {
                    rendered: "Semantic memory [episodic] waiver code".to_string(),
                    source: MemoryContextSource::Semantic,
                    source_weight: 1.0,
                    fused_score: 0.01639,
                    source_rank: 0,
                    tie_break_priority: 3,
                },
                openfang_types::memory::FusedMemoryContextCandidate {
                    rendered: "Shared memory [project.alpha.status] qa signoff".to_string(),
                    source: MemoryContextSource::Shared,
                    source_weight: 1.32,
                    fused_score: 0.01613,
                    source_rank: 0,
                    tie_break_priority: 0,
                },
            ],
            maintenance_signals: 2,
            attention_signals: 1,
            session_summaries: 1,
        };

        let rendered = render_prompt_memory_context_trace(&trace).unwrap();

        assert!(rendered.contains("semantic_mode=hybrid"));
        assert!(rendered.contains("semantic_candidates=2"));
        assert!(rendered.contains("shared_candidates=1"));
        assert!(rendered.contains("maintenance_signals=2"));
        assert!(rendered.contains("attention_signals=1"));
        assert!(rendered.contains("session_summaries=1"));
        assert!(rendered.contains("1. source=semantic source_rank=1 source_weight=1.000"));
        assert!(rendered.contains("tie_break_priority=3"));
        assert!(rendered.contains("2. source=shared source_rank=1 source_weight=1.320"));
        assert!(rendered.contains("tie_break_priority=0"));
    }

    #[test]
    fn test_render_prompt_memory_context_trace_omitted_when_all_sources_empty() {
        let trace = PromptMemoryContextTrace {
            semantic_mode: MemoryContextRecallMode::TextOnly,
            semantic_candidates: 0,
            shared_candidates: 0,
            fused_candidates: Vec::new(),
            maintenance_signals: 0,
            attention_signals: 0,
            session_summaries: 0,
        };

        assert!(render_prompt_memory_context_trace(&trace).is_none());
    }

    #[test]
    fn test_render_recent_session_summaries_prefers_latest_keys() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = AgentId::new();
        memory
            .structured_set(
                agent_id,
                "session_2026-03-10_alpha",
                serde_json::Value::String("Older summary".to_string()),
            )
            .unwrap();
        memory
            .structured_set(
                agent_id,
                "session_2026-03-12_beta",
                serde_json::Value::String("Newest summary".to_string()),
            )
            .unwrap();
        memory
            .structured_set(
                agent_id,
                "project.alpha.status",
                serde_json::Value::String("in_progress".to_string()),
            )
            .unwrap();

        let summaries =
            render_recent_session_summaries_from_entries(&memory.list_kv(agent_id).unwrap(), 1);
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].contains("session_2026-03-12_beta"));
        assert!(summaries[0].contains("Newest summary"));
    }

    #[test]
    fn test_rank_governed_memory_context_candidates_surfaces_lifecycle_and_tags() {
        let now = chrono::Utc::now();
        let themed_metadata = openfang_types::memory::MemoryRecordMetadata {
            schema_version: openfang_types::memory::MEMORY_METADATA_SCHEMA_VERSION,
            key: "pref.editor.theme".to_string(),
            namespace: "pref".to_string(),
            kind: "preference".to_string(),
            tags: vec!["profile".to_string(), "ui".to_string()],
            freshness: MemoryFreshness::Durable,
            source: "memory_store_tool".to_string(),
            updated_at: now,
        };
        let fact_metadata = openfang_types::memory::MemoryRecordMetadata {
            schema_version: openfang_types::memory::MEMORY_METADATA_SCHEMA_VERSION,
            key: "general.note".to_string(),
            namespace: "general".to_string(),
            kind: "fact".to_string(),
            tags: vec![],
            freshness: MemoryFreshness::Durable,
            source: "memory_store_tool".to_string(),
            updated_at: now,
        };
        let entries = vec![
            (
                "pref.editor.theme".to_string(),
                serde_json::json!("solarized dark"),
            ),
            (
                openfang_types::memory::memory_metadata_key("pref.editor.theme").unwrap(),
                serde_json::to_value(&themed_metadata).unwrap(),
            ),
            ("general.note".to_string(), serde_json::json!("plain fact")),
            (
                openfang_types::memory::memory_metadata_key("general.note").unwrap(),
                serde_json::to_value(&fact_metadata).unwrap(),
            ),
        ];

        let rendered = rank_governed_memory_context_candidates_for_query(
            &entries,
            now,
            5,
            Some("What is the editor theme and ui preference?"),
        );

        assert_eq!(rendered.len(), 1);
        assert!(rendered[0].rendered.starts_with("Shared memory "));
        assert!(rendered[0].rendered.contains("[pref.editor.theme]"));
        assert!(rendered[0].rendered.contains("kind=preference"));
        assert!(rendered[0].rendered.contains("freshness=durable"));
        assert!(rendered[0].rendered.contains("lifecycle=active"));
        assert!(rendered[0].rendered.contains("tags=profile,ui"));
        assert!(rendered[0].rendered.contains("promotion_candidate"));
        assert!(rendered[0].source_weight > 1.0);
    }

    #[test]
    fn test_rank_governed_memory_context_candidates_query_prioritizes_matching_project_entry() {
        let now = chrono::Utc::now();
        let pref_metadata = openfang_types::memory::MemoryRecordMetadata {
            schema_version: openfang_types::memory::MEMORY_METADATA_SCHEMA_VERSION,
            key: "pref.editor.theme".to_string(),
            namespace: "pref".to_string(),
            kind: "preference".to_string(),
            tags: vec!["profile".to_string(), "ui".to_string()],
            freshness: MemoryFreshness::Durable,
            source: "memory_store_tool".to_string(),
            updated_at: now - chrono::Duration::days(1),
        };
        let project_metadata = openfang_types::memory::MemoryRecordMetadata {
            schema_version: openfang_types::memory::MEMORY_METADATA_SCHEMA_VERSION,
            key: "project.alpha.status".to_string(),
            namespace: "project".to_string(),
            kind: "project_state".to_string(),
            tags: vec!["project".to_string(), "alpha".to_string()],
            freshness: MemoryFreshness::Rolling,
            source: "memory_store_tool".to_string(),
            updated_at: now,
        };
        let entries = vec![
            (
                "pref.editor.theme".to_string(),
                serde_json::json!("Use compact bullet points."),
            ),
            (
                openfang_types::memory::memory_metadata_key("pref.editor.theme").unwrap(),
                serde_json::to_value(&pref_metadata).unwrap(),
            ),
            (
                "project.alpha.status".to_string(),
                serde_json::json!("Alpha project is in progress."),
            ),
            (
                openfang_types::memory::memory_metadata_key("project.alpha.status").unwrap(),
                serde_json::to_value(&project_metadata).unwrap(),
            ),
        ];

        let rendered = rank_governed_memory_context_candidates_for_query(
            &entries,
            now,
            2,
            Some("What is the alpha project status?"),
        );

        assert_eq!(rendered.len(), 2);
        assert!(rendered[0].rendered.starts_with("Shared memory "));
        assert!(rendered[0].rendered.contains("[project.alpha.status]"));
        assert!(rendered[0].source_weight > rendered[1].source_weight);
        assert!(rendered[1].rendered.contains("[pref.editor.theme]"));
    }

    #[test]
    fn test_render_governed_memory_orchestration_signals_surfaces_review_and_promotion() {
        let now = chrono::Utc::now();
        let stale_project = openfang_types::memory::MemoryRecordMetadata {
            schema_version: openfang_types::memory::MEMORY_METADATA_SCHEMA_VERSION,
            key: "project.alpha.status".to_string(),
            namespace: "project".to_string(),
            kind: "project_state".to_string(),
            tags: vec!["project".to_string(), "alpha".to_string()],
            freshness: MemoryFreshness::Rolling,
            source: "memory_store_tool".to_string(),
            updated_at: now - chrono::Duration::days(8),
        };
        let durable_pref = openfang_types::memory::MemoryRecordMetadata {
            schema_version: openfang_types::memory::MEMORY_METADATA_SCHEMA_VERSION,
            key: "pref.editor.theme".to_string(),
            namespace: "pref".to_string(),
            kind: "preference".to_string(),
            tags: vec!["profile".to_string(), "ui".to_string()],
            freshness: MemoryFreshness::Durable,
            source: "memory_store_tool".to_string(),
            updated_at: now - chrono::Duration::days(1),
        };
        let entries = vec![
            (
                "project.alpha.status".to_string(),
                serde_json::json!("Alpha is blocked on QA."),
            ),
            (
                openfang_types::memory::memory_metadata_key("project.alpha.status").unwrap(),
                serde_json::to_value(&stale_project).unwrap(),
            ),
            (
                "pref.editor.theme".to_string(),
                serde_json::json!("Use solarized dark."),
            ),
            (
                openfang_types::memory::memory_metadata_key("pref.editor.theme").unwrap(),
                serde_json::to_value(&durable_pref).unwrap(),
            ),
        ];

        let rendered = render_governed_memory_orchestration_signals_for_query(
            &entries,
            now,
            2,
            Some("What is the alpha project status and ui preference?"),
        );

        assert_eq!(rendered.len(), 2);
        assert!(rendered[0].contains("Review stale memory before reuse"));
        assert!(rendered[0].contains("[project.alpha.status]"));
        assert!(rendered[1].contains("Consider promoting to MEMORY.md"));
        assert!(rendered[1].contains("[pref.editor.theme]"));
    }

    #[test]
    fn test_render_memory_cleanup_orchestration_signals_surfaces_maintenance_actions() {
        let entries = vec![
            ("legacy_theme".to_string(), serde_json::json!("solarized")),
            (
                "project.alpha.note".to_string(),
                serde_json::json!("Alpha note"),
            ),
            (
                openfang_types::memory::memory_metadata_key("pref.orphan").unwrap(),
                serde_json::json!({
                    "schema_version": openfang_types::memory::MEMORY_METADATA_SCHEMA_VERSION,
                    "key": "pref.orphan",
                    "namespace": "pref",
                    "kind": "preference",
                    "tags": ["profile"],
                    "freshness": "durable",
                    "source": "memory_store_tool",
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                }),
            ),
        ];

        let rendered = render_memory_cleanup_orchestration_signals(&entries, 2);

        assert_eq!(rendered.len(), 3);
        assert!(rendered[0].contains("Run memory_cleanup before reuse"));
        assert!(rendered[0].contains("[legacy_theme]"));
        assert!(rendered[1].contains("backfill governed metadata"));
        assert!(rendered[1].contains("[project.alpha.note]"));
        assert!(rendered[2].contains("remove orphan metadata sidecar"));
        assert!(rendered[2].contains("__openfang_memory_meta.pref.orphan"));
    }

    #[test]
    fn test_trim_messages_preserves_slots_for_prepended_context() {
        let mut messages: Vec<Message> = (0..MAX_HISTORY_MESSAGES)
            .map(|i| Message::user(format!("message {i}")))
            .collect();

        trim_messages_for_prepended_context(&mut messages, 2);

        assert_eq!(messages.len(), MAX_HISTORY_MESSAGES - 2);
        assert_eq!(
            messages.first().unwrap().content.text_content(),
            "message 2"
        );
        assert_eq!(
            messages.last().unwrap().content.text_content(),
            format!("message {}", MAX_HISTORY_MESSAGES - 1)
        );
    }

    #[test]
    fn test_max_continuations_constant() {
        assert_eq!(MAX_CONTINUATIONS, 5);
    }

    #[test]
    fn test_tool_timeout_constant() {
        assert_eq!(TOOL_TIMEOUT_SECS, 120);
    }

    #[test]
    fn test_max_history_messages() {
        assert_eq!(MAX_HISTORY_MESSAGES, 20);
    }

    // --- Integration tests for empty response guards ---

    fn test_manifest() -> AgentManifest {
        AgentManifest {
            name: "test-agent".to_string(),
            model: openfang_types::agent::ModelConfig {
                system_prompt: "You are a test agent.".to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Mock driver that simulates: first call returns ToolUse with no text,
    /// second call returns EndTurn with empty text. This reproduces the bug
    /// where the LLM ends with no text after a tool-use cycle.
    struct EmptyAfterToolUseDriver {
        call_count: AtomicU32,
    }

    impl EmptyAfterToolUseDriver {
        fn new() -> Self {
            Self {
                call_count: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmDriver for EmptyAfterToolUseDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let call = self.call_count.fetch_add(1, Ordering::Relaxed);
            if call == 0 {
                // First call: LLM wants to use a tool (with no text block)
                Ok(CompletionResponse {
                    content: vec![ContentBlock::ToolUse {
                        id: "tool_1".to_string(),
                        name: "fake_tool".to_string(),
                        input: serde_json::json!({"query": "test"}),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::ToolUse,
                    tool_calls: vec![ToolCall {
                        id: "tool_1".to_string(),
                        name: "fake_tool".to_string(),
                        input: serde_json::json!({"query": "test"}),
                    }],
                    usage: TokenUsage {
                        input_tokens: 10,
                        output_tokens: 5,
                    },
                })
            } else {
                // Second call: LLM returns EndTurn with EMPTY text (the bug)
                Ok(CompletionResponse {
                    content: vec![],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 10,
                        output_tokens: 0,
                    },
                })
            }
        }
    }

    /// Mock driver that returns empty text with MaxTokens stop reason,
    /// repeated MAX_CONTINUATIONS times to trigger the max continuations path.
    struct EmptyMaxTokensDriver;

    #[async_trait]
    impl LlmDriver for EmptyMaxTokensDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![],
                stop_reason: StopReason::MaxTokens,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 0,
                },
            })
        }
    }

    /// Mock driver that returns normal text (sanity check).
    struct NormalDriver;

    #[async_trait]
    impl LlmDriver for NormalDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "Hello from the agent!".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 8,
                },
            })
        }
    }

    #[tokio::test]
    async fn test_empty_response_after_tool_use_returns_fallback() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(EmptyAfterToolUseDriver::new());

        let result = run_agent_loop(
            &manifest,
            "Do something with tools",
            &mut session,
            &memory,
            driver,
            &[], // no tools registered — the tool call will fail, which is fine
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
        )
        .await
        .expect("Loop should complete without error");

        // The response MUST NOT be empty — it should contain our fallback text
        assert!(
            !result.response.trim().is_empty(),
            "Response should not be empty after tool use, got: {:?}",
            result.response
        );
        assert!(
            result.response.contains("Task completed"),
            "Expected fallback message, got: {:?}",
            result.response
        );
    }

    #[tokio::test]
    async fn test_empty_response_max_tokens_returns_fallback() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(EmptyMaxTokensDriver);

        let result = run_agent_loop(
            &manifest,
            "Tell me something long",
            &mut session,
            &memory,
            driver,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
        )
        .await
        .expect("Loop should complete without error");

        // Should hit MAX_CONTINUATIONS and return fallback instead of empty
        assert!(
            !result.response.trim().is_empty(),
            "Response should not be empty on max tokens, got: {:?}",
            result.response
        );
        assert!(
            result.response.contains("token limit"),
            "Expected max-tokens fallback message, got: {:?}",
            result.response
        );
    }

    #[tokio::test]
    async fn test_normal_response_not_replaced_by_fallback() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(NormalDriver);

        let result = run_agent_loop(
            &manifest,
            "Say hello",
            &mut session,
            &memory,
            driver,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
        )
        .await
        .expect("Loop should complete without error");

        // Normal response should pass through unchanged
        assert_eq!(result.response, "Hello from the agent!");
    }

    #[tokio::test]
    async fn test_streaming_empty_response_after_tool_use_returns_fallback() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(EmptyAfterToolUseDriver::new());
        let (tx, _rx) = mpsc::channel(64);

        let result = run_agent_loop_streaming(
            &manifest,
            "Do something with tools",
            &mut session,
            &memory,
            driver,
            &[],
            None,
            tx,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
        )
        .await
        .expect("Streaming loop should complete without error");

        assert!(
            !result.response.trim().is_empty(),
            "Streaming response should not be empty after tool use, got: {:?}",
            result.response
        );
        assert!(
            result.response.contains("Task completed"),
            "Expected fallback message in streaming, got: {:?}",
            result.response
        );
    }

    /// Mock driver that returns empty text on first call (EndTurn), then normal text on second.
    /// This tests the one-shot retry logic for iteration 0 empty responses.
    struct EmptyThenNormalDriver {
        call_count: AtomicU32,
    }

    impl EmptyThenNormalDriver {
        fn new() -> Self {
            Self {
                call_count: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmDriver for EmptyThenNormalDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let call = self.call_count.fetch_add(1, Ordering::Relaxed);
            if call == 0 {
                // First call: empty EndTurn (triggers retry)
                Ok(CompletionResponse {
                    content: vec![],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 10,
                        output_tokens: 0,
                    },
                })
            } else {
                // Second call (retry): normal response
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: "Recovered after retry!".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 15,
                        output_tokens: 8,
                    },
                })
            }
        }
    }

    /// Mock driver that always returns empty EndTurn (no recovery on retry).
    /// Tests that the fallback message appears when retry also fails.
    struct AlwaysEmptyDriver;

    #[async_trait]
    impl LlmDriver for AlwaysEmptyDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 0,
                },
            })
        }
    }

    #[tokio::test]
    async fn test_empty_first_response_retries_and_recovers() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(EmptyThenNormalDriver::new());

        let result = run_agent_loop(
            &manifest,
            "Hello",
            &mut session,
            &memory,
            driver,
            &[],
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
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
        )
        .await
        .expect("Loop should recover via retry");

        assert_eq!(result.response, "Recovered after retry!");
        assert_eq!(
            result.iterations, 2,
            "Should have taken 2 iterations (retry)"
        );
    }

    #[tokio::test]
    async fn test_empty_first_response_fallback_when_retry_also_empty() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(AlwaysEmptyDriver);

        let result = run_agent_loop(
            &manifest,
            "Hello",
            &mut session,
            &memory,
            driver,
            &[],
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
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
        )
        .await
        .expect("Loop should complete with fallback");

        // No tools were executed, so should get the empty response message
        assert!(
            result.response.contains("empty response"),
            "Expected empty response fallback (no tools executed), got: {:?}",
            result.response
        );
    }

    #[tokio::test]
    async fn test_max_history_messages_constant() {
        assert_eq!(MAX_HISTORY_MESSAGES, 20);
    }

    #[tokio::test]
    async fn test_streaming_empty_response_max_tokens_returns_fallback() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(EmptyMaxTokensDriver);
        let (tx, _rx) = mpsc::channel(64);

        let result = run_agent_loop_streaming(
            &manifest,
            "Tell me something long",
            &mut session,
            &memory,
            driver,
            &[],
            None,
            tx,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
        )
        .await
        .expect("Streaming loop should complete without error");

        assert!(
            !result.response.trim().is_empty(),
            "Streaming response should not be empty on max tokens, got: {:?}",
            result.response
        );
        assert!(
            result.response.contains("token limit"),
            "Expected max-tokens fallback in streaming, got: {:?}",
            result.response
        );
    }

    #[test]
    fn test_recover_text_tool_calls_basic() {
        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search the web".into(),
            input_schema: serde_json::json!({}),
        }];
        let text =
            r#"Let me search for that. <function=web_search>{"query":"rust async"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].input["query"], "rust async");
        assert!(calls[0].id.starts_with("recovered_"));
    }

    #[test]
    fn test_recover_text_tool_calls_unknown_tool() {
        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search the web".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function=hack_system>{"cmd":"rm -rf /"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty(), "Unknown tools should be rejected");
    }

    #[test]
    fn test_recover_text_tool_calls_invalid_json() {
        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search the web".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function=web_search>not valid json</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty(), "Invalid JSON should be skipped");
    }

    #[test]
    fn test_recover_text_tool_calls_multiple() {
        let tools = vec![
            tool_definition! {
                name: "web_search".into(),
                description: "Search".into(),
                input_schema: serde_json::json!({}),
            },
            tool_definition! {
                name: "read_file".into(),
                description: "Read a file".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        let text = r#"<function=web_search>{"query":"hello"}</function> then <function=read_file>{"path":"a.txt"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[1].name, "read_file");
    }

    #[test]
    fn test_recover_text_tool_calls_no_pattern() {
        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "Just a normal response with no tool calls.";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_text_tool_calls_empty_tools() {
        let text = r#"<function=web_search>{"query":"hello"}</function>"#;
        let calls = recover_text_tool_calls(text, &[]);
        assert!(calls.is_empty(), "No tools = no recovery");
    }

    // --- Deep edge-case tests for text-to-tool recovery ---

    #[test]
    fn test_recover_text_tool_calls_nested_json() {
        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function=web_search>{"query":"rust","filters":{"lang":"en","year":2024}}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].input["filters"]["lang"], "en");
    }

    #[test]
    fn test_recover_text_tool_calls_with_surrounding_text() {
        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "Sure, let me search that for you.\n\n<function=web_search>{\"query\":\"rust async programming\"}</function>\n\nI'll get back to you with results.";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].input["query"], "rust async programming");
    }

    #[test]
    fn test_recover_text_tool_calls_whitespace_in_json() {
        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        // Some models emit pretty-printed JSON
        let text = "<function=web_search>\n  {\"query\": \"hello world\"}\n</function>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].input["query"], "hello world");
    }

    #[test]
    fn test_recover_text_tool_calls_unclosed_tag() {
        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        // Missing </function> — should gracefully skip
        let text = r#"<function=web_search>{"query":"test"}"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty(), "Unclosed tag should be skipped");
    }

    #[test]
    fn test_recover_text_tool_calls_missing_closing_bracket() {
        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        // Missing > after tool name
        let text = r#"<function=web_search{"query":"test"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        // The parser finds > inside JSON, will likely produce invalid tool name
        // or invalid JSON — either way, should not panic
        // (just verifying no panic / no bad behavior)
        let _ = calls;
    }

    #[test]
    fn test_recover_text_tool_calls_empty_json_object() {
        let tools = vec![tool_definition! {
            name: "list_files".into(),
            description: "List".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function=list_files>{}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "list_files");
        assert_eq!(calls[0].input, serde_json::json!({}));
    }

    #[test]
    fn test_recover_text_tool_calls_mixed_valid_invalid() {
        let tools = vec![
            tool_definition! {
                name: "web_search".into(),
                description: "Search".into(),
                input_schema: serde_json::json!({}),
            },
            tool_definition! {
                name: "read_file".into(),
                description: "Read".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        // First: valid, second: unknown tool, third: valid
        let text = r#"<function=web_search>{"q":"a"}</function> <function=unknown>{"x":1}</function> <function=read_file>{"path":"b"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 2, "Should recover 2 valid, skip 1 unknown");
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[1].name, "read_file");
    }

    // --- Variant 2 pattern tests: <function>NAME{JSON}</function> ---

    #[test]
    fn test_recover_variant2_basic() {
        let tools = vec![tool_definition! {
            name: "web_fetch".into(),
            description: "Fetch".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function>web_fetch{"url":"https://example.com"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_fetch");
        assert_eq!(calls[0].input["url"], "https://example.com");
    }

    #[test]
    fn test_recover_variant2_unknown_tool() {
        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function>unknown_tool{"q":"test"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 0);
    }

    #[test]
    fn test_recover_variant2_with_surrounding_text() {
        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"Let me search for that. <function>web_search{"query":"rust lang"}</function> I'll find the answer."#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
    }

    #[test]
    fn test_recover_both_variants_mixed() {
        let tools = vec![
            tool_definition! {
                name: "web_search".into(),
                description: "Search".into(),
                input_schema: serde_json::json!({}),
            },
            tool_definition! {
                name: "web_fetch".into(),
                description: "Fetch".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        // Mix of variant 1 and variant 2
        let text = r#"<function=web_search>{"q":"a"}</function> <function>web_fetch{"url":"https://x.com"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[1].name, "web_fetch");
    }

    #[test]
    fn test_recover_tool_tag_variant() {
        let tools = vec![tool_definition! {
            name: "exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"I'll run that for you. <tool>exec{"command":"ls -la"}</tool>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(calls[0].input["command"], "ls -la");
    }

    #[test]
    fn test_recover_markdown_code_block() {
        let tools = vec![tool_definition! {
            name: "exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "I'll execute that command:\n```\nexec {\"command\": \"ls -la\"}\n```";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(calls[0].input["command"], "ls -la");
    }

    #[test]
    fn test_recover_markdown_code_block_with_lang() {
        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "```json\nweb_search {\"query\": \"rust\"}\n```";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
    }

    #[test]
    fn test_recover_backtick_wrapped() {
        let tools = vec![tool_definition! {
            name: "exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"Let me run `exec {"command":"pwd"}` for you."#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(calls[0].input["command"], "pwd");
    }

    #[test]
    fn test_recover_backtick_ignores_unknown_tool() {
        let tools = vec![tool_definition! {
            name: "exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"Try `unknown_tool {"key":"val"}` instead."#;
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_no_duplicates_across_patterns() {
        let tools = vec![tool_definition! {
            name: "exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        // Same call in both function tag and tool tag — should only appear once
        let text =
            r#"<function=exec>{"command":"ls"}</function> <tool>exec{"command":"ls"}</tool>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
    }

    // --- Pattern 6: [TOOL_CALL]...[/TOOL_CALL] tests (issue #354) ---

    #[test]
    fn test_recover_tool_call_block_json() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute shell command".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "[TOOL_CALL]\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls -la\"}}\n[/TOOL_CALL]";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].input["command"], "ls -la");
    }

    #[test]
    fn test_recover_tool_call_block_arrow_syntax() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute shell command".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        // Exact format from issue #354
        let text = "[TOOL_CALL]\n{tool => \"shell_exec\", args => {\n--command \"ls -F /\"\n}}\n[/TOOL_CALL]";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].input["command"], "ls -F /");
    }

    #[test]
    fn test_recover_tool_call_block_unknown_tool() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "[TOOL_CALL]\n{\"name\": \"hack_system\", \"arguments\": {\"cmd\": \"rm -rf /\"}}\n[/TOOL_CALL]";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_tool_call_block_multiple() {
        let tools = vec![
            ToolDefinition {
                name: "shell_exec".into(),
                description: "Execute".into(),
                input_schema: serde_json::json!({}),
                defer_loading: false,
            },
            ToolDefinition {
                name: "file_read".into(),
                description: "Read".into(),
                input_schema: serde_json::json!({}),
                defer_loading: false,
            },
        ];
        let text = "[TOOL_CALL]\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}\n[/TOOL_CALL]\nSome text.\n[TOOL_CALL]\n{\"name\": \"file_read\", \"arguments\": {\"path\": \"/tmp/test.txt\"}}\n[/TOOL_CALL]";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[1].name, "file_read");
    }

    #[test]
    fn test_recover_tool_call_block_unclosed() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        // Unclosed [TOOL_CALL] — pattern 6 skips it, but pattern 8 (bare JSON)
        // still finds the valid JSON tool call object.
        let text = "[TOOL_CALL]\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1, "Bare JSON fallback should recover this");
        assert_eq!(calls[0].name, "shell_exec");
    }

    // --- Pattern 7: <tool_call>JSON</tool_call> tests (Qwen3, issue #332) ---

    #[test]
    fn test_recover_tool_call_xml_basic() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "<tool_call>\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls -la\"}}\n</tool_call>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].input["command"], "ls -la");
    }

    #[test]
    fn test_recover_tool_call_xml_with_surrounding_text() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "I'll search for that.\n\n<tool_call>\n{\"name\": \"web_search\", \"arguments\": {\"query\": \"rust async\"}}\n</tool_call>\n\nLet me get results.";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].input["query"], "rust async");
    }

    #[test]
    fn test_recover_tool_call_xml_function_field() {
        let tools = vec![ToolDefinition {
            name: "file_read".into(),
            description: "Read".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "<tool_call>{\"function\": \"file_read\", \"arguments\": {\"path\": \"/etc/hosts\"}}</tool_call>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "file_read");
    }

    #[test]
    fn test_recover_tool_call_xml_parameters_field() {
        let tools = vec![ToolDefinition {
            name: "web_fetch".into(),
            description: "Fetch".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "<tool_call>{\"name\": \"web_fetch\", \"parameters\": {\"url\": \"https://example.com\"}}</tool_call>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_fetch");
        assert_eq!(calls[0].input["url"], "https://example.com");
    }

    #[test]
    fn test_recover_tool_call_xml_stringified_args() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "<tool_call>{\"name\": \"shell_exec\", \"arguments\": \"{\\\"command\\\": \\\"pwd\\\"}\"}</tool_call>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].input["command"], "pwd");
    }

    #[test]
    fn test_recover_tool_call_xml_unknown_tool() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "<tool_call>{\"name\": \"hack_system\", \"arguments\": {\"cmd\": \"rm -rf /\"}}</tool_call>";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_tool_call_xml_multiple() {
        let tools = vec![
            ToolDefinition {
                name: "shell_exec".into(),
                description: "Execute".into(),
                input_schema: serde_json::json!({}),
                defer_loading: false,
            },
            ToolDefinition {
                name: "web_search".into(),
                description: "Search".into(),
                input_schema: serde_json::json!({}),
                defer_loading: false,
            },
        ];
        let text = "<tool_call>{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}</tool_call>\n<tool_call>{\"name\": \"web_search\", \"arguments\": {\"query\": \"rust\"}}</tool_call>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[1].name, "web_search");
    }

    // --- Pattern 8: Bare JSON tool call object tests ---

    #[test]
    fn test_recover_bare_json_tool_call() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text =
            "I'll run that: {\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls -la\"}}";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].input["command"], "ls -la");
    }

    #[test]
    fn test_recover_bare_json_no_false_positive() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "The config looks like {\"debug\": true, \"level\": \"info\"}";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_bare_json_skipped_when_tags_found() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "<function=shell_exec>{\"command\":\"ls\"}</function> {\"name\": \"shell_exec\", \"arguments\": {\"command\": \"pwd\"}}";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].input["command"], "ls");
    }

    // --- Pattern 9: XML-attribute style <function name="..." parameters="..." /> ---

    #[test]
    fn test_recover_xml_attribute_basic() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = r#"<function name="web_search" parameters="{&quot;query&quot;: &quot;best crypto 2024&quot;}" />"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].input["query"], "best crypto 2024");
    }

    #[test]
    fn test_recover_xml_attribute_unknown_tool() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = r#"<function name="unknown_tool" parameters="{&quot;x&quot;: 1}" />"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_xml_attribute_non_selfclosing() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = r#"<function name="shell_exec" parameters="{&quot;command&quot;: &quot;ls&quot;}"></function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
    }

    // --- Pattern 10: <|plugin|>...<|endofblock|> tests ---

    #[test]
    fn test_recover_plugin_block() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "<|plugin|>\n{\"name\": \"web_search\", \"arguments\": {\"query\": \"rust\"}}\n<|endofblock|>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].input["query"], "rust");
    }

    #[test]
    fn test_recover_plugin_block_unknown_tool() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text =
            "<|plugin|>\n{\"name\": \"hack\", \"arguments\": {\"cmd\": \"rm\"}}\n<|endofblock|>";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    // --- Pattern 11: Action/Action Input tests ---

    #[test]
    fn test_recover_action_input() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "Action: web_search\nAction Input: {\"query\": \"rust programming\"}";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].input["query"], "rust programming");
    }

    #[test]
    fn test_recover_action_input_unknown_tool() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "Action: unknown_tool\nAction Input: {\"key\": \"value\"}";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    // --- Pattern 12: name + JSON on next line tests ---

    #[test]
    fn test_recover_name_json_nextline() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "shell_exec\n{\"command\": \"ls -la\"}";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].input["command"], "ls -la");
    }

    #[test]
    fn test_recover_name_json_nextline_unknown() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "unknown_tool\n{\"command\": \"ls\"}";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    // --- Pattern 13: <tool_use> tests ---

    #[test]
    fn test_recover_tool_use_block() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text =
            "<tool_use>{\"name\": \"web_search\", \"arguments\": {\"query\": \"test\"}}</tool_use>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
    }

    #[test]
    fn test_recover_tool_use_block_unknown() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
            defer_loading: false,
        }];
        let text = "<tool_use>{\"name\": \"hack\", \"arguments\": {\"cmd\": \"rm\"}}</tool_use>";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    // --- Helper function tests ---

    #[test]
    fn test_parse_dash_dash_args_basic() {
        let result = parse_dash_dash_args("{--command \"ls -F /\"}");
        assert_eq!(result["command"], "ls -F /");
    }

    #[test]
    fn test_parse_dash_dash_args_multiple() {
        let result = parse_dash_dash_args("{--file \"test.txt\", --verbose}");
        assert_eq!(result["file"], "test.txt");
        assert_eq!(result["verbose"], true);
    }

    #[test]
    fn test_parse_dash_dash_args_unquoted_value() {
        let result = parse_dash_dash_args("{--count 5}");
        assert_eq!(result["count"], "5");
    }

    #[test]
    fn test_parse_json_tool_call_object_standard() {
        let tool_names = vec!["shell_exec"];
        let result = parse_json_tool_call_object(
            "{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}",
            &tool_names,
        );
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "shell_exec");
        assert_eq!(args["command"], "ls");
    }

    #[test]
    fn test_parse_json_tool_call_object_function_field() {
        let tool_names = vec!["web_fetch"];
        let result = parse_json_tool_call_object(
            "{\"function\": \"web_fetch\", \"parameters\": {\"url\": \"https://x.com\"}}",
            &tool_names,
        );
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "web_fetch");
        assert_eq!(args["url"], "https://x.com");
    }

    #[test]
    fn test_parse_json_tool_call_object_unknown_tool() {
        let tool_names = vec!["shell_exec"];
        let result =
            parse_json_tool_call_object("{\"name\": \"unknown\", \"arguments\": {}}", &tool_names);
        assert!(result.is_none());
    }

    // --- End-to-end integration test: text-as-tool-call recovery through agent loop ---

    /// Mock driver that simulates a Groq/Llama model outputting tool calls as text.
    /// Call 1: Returns text with `<function=web_search>...</function>` (EndTurn, no tool_calls)
    /// Call 2: Returns a normal text response (after tool result is provided)
    struct TextToolCallDriver {
        call_count: AtomicU32,
    }

    impl TextToolCallDriver {
        fn new() -> Self {
            Self {
                call_count: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmDriver for TextToolCallDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let call = self.call_count.fetch_add(1, Ordering::Relaxed);
            if call == 0 {
                // Simulate Groq/Llama: tool call as text, not in tool_calls field
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: r#"Let me search for that. <function=web_search>{"query":"rust async"}</function>"#.to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![], // BUG: no tool_calls!
                    usage: TokenUsage {
                        input_tokens: 20,
                        output_tokens: 15,
                    },
                })
            } else {
                // After tool result, return normal response
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: "Based on the search results, Rust async is great!".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 30,
                        output_tokens: 12,
                    },
                })
            }
        }
    }

    #[tokio::test]
    async fn test_text_tool_call_recovery_e2e() {
        // This is THE critical test: a model outputs a tool call as text,
        // the recovery code detects it, promotes it to ToolUse, executes the tool,
        // and the agent loop continues to produce a final response.
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(TextToolCallDriver::new());

        // Provide web_search as an available tool so recovery can match it
        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search the web".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                }
            }),
        }];

        let result = run_agent_loop(
            &manifest,
            "Search for rust async programming",
            &mut session,
            &memory,
            driver,
            &tools,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
        )
        .await
        .expect("Agent loop should complete");

        // The response should contain the second call's output, NOT the raw function tag
        assert!(
            !result.response.contains("<function="),
            "Response should not contain raw function tags, got: {:?}",
            result.response
        );
        assert!(
            result.iterations >= 2,
            "Should have at least 2 iterations (tool call + final response), got: {}",
            result.iterations
        );
        // Verify the final text response came through
        assert!(
            result.response.contains("search results") || result.response.contains("Rust async"),
            "Expected final response text, got: {:?}",
            result.response
        );
    }

    /// Mock driver that returns NO text-based tool calls — just normal text.
    /// Verifies recovery does NOT interfere with normal flow.
    #[tokio::test]
    async fn test_normal_flow_unaffected_by_recovery() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(NormalDriver);

        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search the web".into(),
            input_schema: serde_json::json!({}),
        }];

        let result = run_agent_loop(
            &manifest,
            "Say hello",
            &mut session,
            &memory,
            driver,
            &tools, // tools available but not used
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
            None,
            None, // user_content_blocks
        )
        .await
        .expect("Normal loop should complete");

        assert_eq!(result.response, "Hello from the agent!");
        assert_eq!(
            result.iterations, 1,
            "Normal response should complete in 1 iteration"
        );
    }

    // --- Streaming path: text-as-tool-call recovery ---

    #[tokio::test]
    async fn test_text_tool_call_recovery_streaming_e2e() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(TextToolCallDriver::new());

        let tools = vec![tool_definition! {
            name: "web_search".into(),
            description: "Search the web".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                }
            }),
        }];

        let (tx, mut rx) = mpsc::channel(64);

        let result = run_agent_loop_streaming(
            &manifest,
            "Search for rust async programming",
            &mut session,
            &memory,
            driver,
            &tools,
            None,
            tx,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
        )
        .await
        .expect("Streaming loop should complete");

        // Same assertions as non-streaming
        assert!(
            !result.response.contains("<function="),
            "Streaming: response should not contain raw function tags, got: {:?}",
            result.response
        );
        assert!(
            result.iterations >= 2,
            "Streaming: should have at least 2 iterations, got: {}",
            result.iterations
        );

        // Drain the stream channel to verify events were sent
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        assert!(!events.is_empty(), "Should have received stream events");
    }
}
