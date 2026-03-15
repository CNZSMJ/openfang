//! Memory substrate types: fragments, sources, filters, and the unified Memory trait.

use crate::agent::AgentId;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use uuid::Uuid;

/// Default namespace for user-managed structured memory keys.
pub const DEFAULT_USER_MEMORY_NAMESPACE: &str = "general";
/// Internal sidecar prefix used to store governance metadata for user memory keys.
pub const MEMORY_METADATA_PREFIX: &str = "__openfang_memory_meta.";
/// Current schema version for governed structured memory metadata.
pub const MEMORY_METADATA_SCHEMA_VERSION: u32 = 1;
/// Source label used when cleanup backfills governed metadata.
pub const MEMORY_CLEANUP_SOURCE: &str = "memory_cleanup_api";

/// Unique identifier for a memory fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoryId(pub Uuid);

impl MemoryId {
    /// Create a new random MemoryId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for MemoryId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MemoryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where a memory came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    /// From a conversation/interaction.
    Conversation,
    /// From a document that was processed.
    Document,
    /// From an observation (tool output, web page, etc.).
    Observation,
    /// Inferred by the agent from existing knowledge.
    Inference,
    /// Explicitly provided by the user.
    UserProvided,
    /// From a system event.
    System,
}

/// A single unit of memory stored in the semantic store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFragment {
    /// Unique ID.
    pub id: MemoryId,
    /// Which agent owns this memory.
    pub agent_id: AgentId,
    /// The textual content of this memory.
    pub content: String,
    /// Vector embedding (populated by the semantic store).
    pub embedding: Option<Vec<f32>>,
    /// Arbitrary metadata.
    pub metadata: HashMap<String, serde_json::Value>,
    /// How this memory was created.
    pub source: MemorySource,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f32,
    /// When this memory was created.
    pub created_at: DateTime<Utc>,
    /// When this memory was last accessed.
    pub accessed_at: DateTime<Utc>,
    /// How many times this memory has been accessed.
    pub access_count: u64,
    /// Memory scope/collection name.
    pub scope: String,
}

/// Filter criteria for memory recall.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryFilter {
    /// Filter by agent ID.
    pub agent_id: Option<AgentId>,
    /// Filter by source type.
    pub source: Option<MemorySource>,
    /// Filter by scope.
    pub scope: Option<String>,
    /// Minimum confidence threshold.
    pub min_confidence: Option<f32>,
    /// Only memories created after this time.
    pub after: Option<DateTime<Utc>>,
    /// Only memories created before this time.
    pub before: Option<DateTime<Utc>>,
    /// Metadata key-value filters.
    pub metadata: HashMap<String, serde_json::Value>,
}

impl MemoryFilter {
    /// Create a filter for a specific agent.
    pub fn agent(agent_id: AgentId) -> Self {
        Self {
            agent_id: Some(agent_id),
            ..Default::default()
        }
    }

    /// Create a filter for a specific scope.
    pub fn scope(scope: impl Into<String>) -> Self {
        Self {
            scope: Some(scope.into()),
            ..Default::default()
        }
    }
}

/// True when the key belongs to an OpenFang-reserved internal namespace.
pub fn is_internal_memory_key(key: &str) -> bool {
    key.starts_with("session_") || key.starts_with("__openfang_")
}

/// True when the key is an internal sidecar metadata key.
pub fn is_memory_metadata_key(key: &str) -> bool {
    key.starts_with(MEMORY_METADATA_PREFIX)
}

/// True when the key is a user-managed legacy bare key that predates namespacing.
pub fn is_legacy_user_memory_key(key: &str) -> bool {
    !is_internal_memory_key(key) && !is_memory_metadata_key(key) && !key.contains('.')
}

fn is_valid_memory_key_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')
}

fn is_valid_memory_namespace(namespace: &str) -> bool {
    !namespace.is_empty()
        && namespace
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

fn is_valid_memory_key_shape(key: &str) -> bool {
    if key.is_empty() || key.starts_with('.') || key.ends_with('.') || key.contains("..") {
        return false;
    }

    key.chars().all(is_valid_memory_key_char)
}

fn is_valid_memory_prefix_shape(prefix: &str) -> bool {
    if prefix.is_empty() || prefix.starts_with('.') || prefix.contains("..") {
        return false;
    }

    prefix.chars().all(is_valid_memory_key_char)
}

/// Normalize a user-facing memory namespace.
pub fn canonicalize_memory_namespace(namespace: &str) -> Result<String, String> {
    let trimmed = namespace.trim();
    if !is_valid_memory_namespace(trimmed) {
        return Err(format!(
            "Invalid memory namespace '{trimmed}'. Use letters, numbers, '_' or '-'."
        ));
    }
    Ok(trimmed.to_string())
}

/// Normalize a user-facing memory key into a namespaced form.
///
/// Internal keys such as `session_*` and `__openfang_*` remain unchanged.
/// Bare user keys are promoted into the `general.` namespace.
pub fn canonicalize_user_memory_key(key: &str) -> Result<String, String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("Memory key cannot be empty.".to_string());
    }
    if !is_valid_memory_key_shape(trimmed) {
        return Err(format!(
            "Invalid memory key '{trimmed}'. Use letters, numbers, '.', '_' or '-'."
        ));
    }

    if is_internal_memory_key(trimmed) {
        return Ok(trimmed.to_string());
    }

    if trimmed.contains('.') {
        return Ok(trimmed.to_string());
    }

    Ok(format!("{DEFAULT_USER_MEMORY_NAMESPACE}.{trimmed}"))
}

/// Return the effective namespace for a memory key.
pub fn memory_key_namespace(key: &str) -> Option<String> {
    let canonical = canonicalize_user_memory_key(key).ok()?;
    if is_internal_memory_key(&canonical) {
        return None;
    }
    canonical
        .split_once('.')
        .map(|(namespace, _)| namespace.to_string())
}

/// Build lookup candidates for a user-facing key, preserving backward compatibility.
pub fn memory_lookup_candidates(key: &str) -> Result<Vec<String>, String> {
    let trimmed = key.trim();
    let canonical = canonicalize_user_memory_key(trimmed)?;
    if canonical == trimmed {
        Ok(vec![canonical])
    } else {
        Ok(vec![canonical, trimmed.to_string()])
    }
}

/// Match a stored key against a user-facing prefix.
pub fn memory_key_matches_prefix(key: &str, prefix: &str) -> Result<bool, String> {
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        return Ok(true);
    }

    if !is_valid_memory_prefix_shape(trimmed) {
        return Err(format!(
            "Invalid memory prefix '{trimmed}'. Use letters, numbers, '.', '_' or '-'."
        ));
    }

    if key.starts_with(trimmed) {
        return Ok(true);
    }

    if is_internal_memory_key(trimmed) || trimmed.contains('.') {
        return Ok(false);
    }

    Ok(key.starts_with(&format!("{DEFAULT_USER_MEMORY_NAMESPACE}.{trimmed}")))
}

/// Freshness class for governed structured memory.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryFreshness {
    Rolling,
    #[default]
    Durable,
    Archival,
}

/// Conflict policy used by governed writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryConflictPolicy {
    #[default]
    Overwrite,
    SkipIfExists,
}

/// Governance metadata stored alongside a user-managed KV entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRecordMetadata {
    pub schema_version: u32,
    pub key: String,
    pub namespace: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub freshness: MemoryFreshness,
    pub source: String,
    pub updated_at: DateTime<Utc>,
}

/// Derived lifecycle state for a governed memory record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLifecycleState {
    Active,
    Stale,
    Expired,
}

/// Computed lifecycle snapshot used by API/tool responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryLifecycleSnapshot {
    pub state: MemoryLifecycleState,
    pub review_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub promotion_candidate: bool,
}

/// Governed structured memory candidate selected for prompt-level retrieval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedMemoryPromptCandidate {
    pub key: String,
    pub value: serde_json::Value,
    pub metadata: MemoryRecordMetadata,
    pub lifecycle: MemoryLifecycleSnapshot,
}

/// Action-oriented governance signal used for prompt orchestration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedMemoryOrchestrationSignal {
    pub key: String,
    pub metadata: MemoryRecordMetadata,
    pub lifecycle: MemoryLifecycleSnapshot,
}

/// Governance snapshot summarizing review and promotion actions for prompts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GovernedMemoryOrchestrationSnapshot {
    pub stale_review: Vec<GovernedMemoryOrchestrationSignal>,
    pub promotion_candidates: Vec<GovernedMemoryOrchestrationSignal>,
}

/// Source types that can participate in prompt-time memory fusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryContextSource {
    Semantic,
    Shared,
}

impl MemoryContextSource {
    /// Stable string label used for logging and prompt trace output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Shared => "shared",
        }
    }
}

/// Ranked prompt-time recall candidate before cross-source fusion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankedMemoryContextCandidate {
    pub rendered: String,
    pub source: MemoryContextSource,
    pub source_rank: usize,
    pub source_weight: f32,
    pub tie_break_priority: u8,
}

/// Ranked prompt-time recall candidate after cross-source fusion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FusedMemoryContextCandidate {
    pub rendered: String,
    pub source: MemoryContextSource,
    pub source_rank: usize,
    pub source_weight: f32,
    pub fused_score: f32,
    pub tie_break_priority: u8,
}

/// Combined prompt-time fusion output used by runtime recall injection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MemoryContextFusionResult {
    pub semantic_candidates: usize,
    pub shared_candidates: usize,
    pub fused_candidates: Vec<FusedMemoryContextCandidate>,
}

/// Which recall mode produced the semantic branch of prompt-time memory context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryContextRecallMode {
    Hybrid,
    TextOnly,
}

impl MemoryContextRecallMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            Self::TextOnly => "text_only",
        }
    }
}

/// Tunable limits for prompt-time memory context assembly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptMemoryContextBuildOptions {
    pub semantic_limit: usize,
    pub shared_limit: usize,
    pub fused_limit: usize,
    pub maintenance_signal_limit: usize,
    pub attention_signal_limit: usize,
    pub session_summary_limit: usize,
}

impl Default for PromptMemoryContextBuildOptions {
    fn default() -> Self {
        Self {
            semantic_limit: 5,
            shared_limit: 10,
            fused_limit: 5,
            maintenance_signal_limit: 2,
            attention_signal_limit: 2,
            session_summary_limit: 3,
        }
    }
}

/// Structured trace payload for prompt-time memory context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptMemoryContextTrace {
    pub semantic_mode: MemoryContextRecallMode,
    pub semantic_candidates: usize,
    pub shared_candidates: usize,
    pub fused_candidates: Vec<FusedMemoryContextCandidate>,
    pub maintenance_signals: usize,
    pub attention_signals: usize,
    pub session_summaries: usize,
}

/// Shared prompt-time memory context payload consumed by runtime prompt injection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptMemoryContext {
    pub recalled_memories: Vec<String>,
    pub cleanup_maintenance_signals: Vec<String>,
    pub governance_attention_signals: Vec<String>,
    pub recent_session_summaries: Vec<String>,
    pub trace: PromptMemoryContextTrace,
}

/// Governed prompt candidate plus fusion metadata for cross-source reranking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GovernedMemoryPromptSelection {
    pub candidate: GovernedMemoryPromptCandidate,
    pub query_match_score: usize,
    pub source_weight: f32,
    pub tie_break_priority: u8,
}

/// Base RRF constant used when fusing prompt-time recall candidates.
pub const MEMORY_CONTEXT_RRF_K: f32 = 60.0;

fn is_valid_memory_meta_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

/// Normalize a memory kind token.
pub fn canonicalize_memory_kind(kind: &str) -> Result<String, String> {
    let trimmed = kind.trim().to_lowercase();
    if !is_valid_memory_meta_token(&trimmed) {
        return Err(format!(
            "Invalid memory kind '{kind}'. Use letters, numbers, '_' or '-'."
        ));
    }
    Ok(trimmed)
}

/// Normalize, lowercase, deduplicate, and validate memory tags.
pub fn canonicalize_memory_tags(tags: &[String]) -> Result<Vec<String>, String> {
    if tags.len() > 8 {
        return Err("Too many memory tags. Maximum is 8.".to_string());
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for tag in tags {
        let normalized = tag.trim().to_lowercase();
        if !is_valid_memory_meta_token(&normalized) {
            return Err(format!(
                "Invalid memory tag '{tag}'. Use letters, numbers, '_' or '-'."
            ));
        }
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    Ok(out)
}

/// Normalize tag filters from repeated query params or comma-delimited input.
pub fn canonicalize_memory_tag_filters(tags: &[String]) -> Result<Vec<String>, String> {
    let flattened: Vec<String> = tags
        .iter()
        .flat_map(|tag| tag.split(','))
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    canonicalize_memory_tags(&flattened)
}

/// True when an entry contains every requested normalized tag.
pub fn memory_tags_match(entry_tags: &[String], filter_tags: &[String]) -> bool {
    filter_tags
        .iter()
        .all(|filter_tag| entry_tags.iter().any(|entry_tag| entry_tag == filter_tag))
}

/// Proposed governance cleanup action for a memory entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCleanupAction {
    MigrateLegacyKey,
    DeleteLegacyKey,
    DeleteOrphanMetadata,
    BackfillMetadata,
}

/// A single governance cleanup finding derived from a raw KV snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryCleanupFinding {
    pub action: MemoryCleanupAction,
    pub key: String,
    pub canonical_key: Option<String>,
    pub metadata_key: Option<String>,
}

/// Cleanup plan derived from a raw KV snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MemoryCleanupPlan {
    pub findings: Vec<MemoryCleanupFinding>,
}

/// Cleanup finding surfaced as a prompt-time governance maintenance signal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryCleanupOrchestrationSignal {
    pub action: MemoryCleanupAction,
    pub key: String,
    pub canonical_key: Option<String>,
    pub metadata_key: Option<String>,
}

/// Cleanup maintenance snapshot grouped for prompt orchestration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MemoryCleanupOrchestrationSnapshot {
    pub legacy_repairs: Vec<MemoryCleanupOrchestrationSignal>,
    pub metadata_repairs: Vec<MemoryCleanupOrchestrationSignal>,
    pub orphan_metadata: Vec<MemoryCleanupOrchestrationSignal>,
}

/// Build a governance cleanup plan for legacy bare keys and sidecar inconsistencies.
pub fn plan_memory_cleanup(entries: &[(String, serde_json::Value)]) -> MemoryCleanupPlan {
    let metadata_map = collect_memory_metadata(entries);
    let primary_keys: HashSet<String> = entries
        .iter()
        .map(|(key, _)| key)
        .filter(|key| !is_memory_metadata_key(key))
        .cloned()
        .collect();

    let mut findings = Vec::new();

    for key in &primary_keys {
        if is_internal_memory_key(key) {
            continue;
        }

        if is_legacy_user_memory_key(key) {
            let canonical_key = canonicalize_user_memory_key(key).ok();
            let action = match canonical_key.as_deref() {
                Some(canonical_key) if primary_keys.contains(canonical_key) => {
                    MemoryCleanupAction::DeleteLegacyKey
                }
                Some(_) => MemoryCleanupAction::MigrateLegacyKey,
                None => continue,
            };
            findings.push(MemoryCleanupFinding {
                action,
                key: key.clone(),
                canonical_key,
                metadata_key: None,
            });
            continue;
        }

        if !metadata_map.contains_key(key) {
            findings.push(MemoryCleanupFinding {
                action: MemoryCleanupAction::BackfillMetadata,
                key: key.clone(),
                canonical_key: Some(key.clone()),
                metadata_key: memory_metadata_key(key).ok(),
            });
        }
    }

    for (key, _) in entries {
        if !is_memory_metadata_key(key) {
            continue;
        }
        let Some(primary_key) = memory_key_from_metadata_key(key) else {
            continue;
        };
        if !primary_keys.contains(&primary_key) {
            findings.push(MemoryCleanupFinding {
                action: MemoryCleanupAction::DeleteOrphanMetadata,
                key: primary_key,
                canonical_key: None,
                metadata_key: Some(key.clone()),
            });
        }
    }

    findings.sort_by(|a, b| {
        a.key.cmp(&b.key).then_with(|| {
            let a_meta = a.metadata_key.as_deref().unwrap_or("");
            let b_meta = b.metadata_key.as_deref().unwrap_or("");
            a_meta.cmp(b_meta)
        })
    });

    MemoryCleanupPlan { findings }
}

fn memory_cleanup_action_sort_rank(action: &MemoryCleanupAction) -> u8 {
    match action {
        MemoryCleanupAction::MigrateLegacyKey => 0,
        MemoryCleanupAction::DeleteLegacyKey => 1,
        MemoryCleanupAction::BackfillMetadata => 2,
        MemoryCleanupAction::DeleteOrphanMetadata => 3,
    }
}

/// Summarize cleanup findings into prompt-time maintenance buckets.
pub fn summarize_memory_cleanup_for_orchestration(
    entries: &[(String, serde_json::Value)],
    limit_per_bucket: usize,
) -> MemoryCleanupOrchestrationSnapshot {
    if limit_per_bucket == 0 {
        return MemoryCleanupOrchestrationSnapshot::default();
    }

    let mut legacy_repairs = Vec::new();
    let mut metadata_repairs = Vec::new();
    let mut orphan_metadata = Vec::new();

    let mut findings = plan_memory_cleanup(entries).findings;
    findings.sort_by(|a, b| {
        memory_cleanup_action_sort_rank(&a.action)
            .cmp(&memory_cleanup_action_sort_rank(&b.action))
            .then_with(|| a.key.cmp(&b.key))
            .then_with(|| a.canonical_key.cmp(&b.canonical_key))
            .then_with(|| a.metadata_key.cmp(&b.metadata_key))
    });

    for finding in findings {
        let signal = MemoryCleanupOrchestrationSignal {
            action: finding.action.clone(),
            key: finding.key.clone(),
            canonical_key: finding.canonical_key.clone(),
            metadata_key: finding.metadata_key.clone(),
        };

        match finding.action {
            MemoryCleanupAction::MigrateLegacyKey | MemoryCleanupAction::DeleteLegacyKey => {
                if legacy_repairs.len() < limit_per_bucket {
                    legacy_repairs.push(signal);
                }
            }
            MemoryCleanupAction::BackfillMetadata => {
                if metadata_repairs.len() < limit_per_bucket {
                    metadata_repairs.push(signal);
                }
            }
            MemoryCleanupAction::DeleteOrphanMetadata => {
                if orphan_metadata.len() < limit_per_bucket {
                    orphan_metadata.push(signal);
                }
            }
        }
    }

    MemoryCleanupOrchestrationSnapshot {
        legacy_repairs,
        metadata_repairs,
        orphan_metadata,
    }
}

/// Build the internal sidecar key that stores governance metadata.
pub fn memory_metadata_key(key: &str) -> Result<String, String> {
    let canonical = canonicalize_user_memory_key(key)?;
    if is_internal_memory_key(&canonical) {
        return Err("Internal memory keys do not use governed metadata sidecars.".to_string());
    }
    Ok(format!("{MEMORY_METADATA_PREFIX}{canonical}"))
}

/// Parse the governed user key represented by a metadata sidecar key.
pub fn memory_key_from_metadata_key(metadata_key: &str) -> Option<String> {
    metadata_key
        .strip_prefix(MEMORY_METADATA_PREFIX)
        .map(|key| key.to_string())
}

/// Build metadata for a governed structured memory entry.
pub fn build_memory_record_metadata(
    key: &str,
    kind: Option<&str>,
    tags: &[String],
    freshness: Option<MemoryFreshness>,
    source: &str,
) -> Result<MemoryRecordMetadata, String> {
    let canonical_key = canonicalize_user_memory_key(key)?;
    if is_internal_memory_key(&canonical_key) {
        return Err("Internal memory keys cannot be wrapped in governed metadata.".to_string());
    }

    let namespace = memory_key_namespace(&canonical_key)
        .ok_or("Governed memory keys must have a namespace.".to_string())?;
    let kind = canonicalize_memory_kind(kind.unwrap_or("fact"))?;
    let tags = canonicalize_memory_tags(tags)?;
    let source = canonicalize_memory_kind(source)?;

    Ok(MemoryRecordMetadata {
        schema_version: MEMORY_METADATA_SCHEMA_VERSION,
        key: canonical_key,
        namespace,
        kind,
        tags,
        freshness: freshness.unwrap_or_default(),
        source,
        updated_at: Utc::now(),
    })
}

/// Extract all governed metadata sidecars from a raw KV list.
pub fn collect_memory_metadata(
    entries: &[(String, serde_json::Value)],
) -> HashMap<String, MemoryRecordMetadata> {
    let mut out = HashMap::new();
    for (key, value) in entries {
        let Some(primary_key) = memory_key_from_metadata_key(key) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_value::<MemoryRecordMetadata>(value.clone()) else {
            continue;
        };
        out.insert(primary_key, metadata);
    }
    out
}

fn lifecycle_review_window(freshness: &MemoryFreshness) -> Duration {
    match freshness {
        MemoryFreshness::Rolling => Duration::days(7),
        MemoryFreshness::Durable => Duration::days(30),
        MemoryFreshness::Archival => Duration::days(180),
    }
}

fn lifecycle_expiry_window(freshness: &MemoryFreshness) -> Option<Duration> {
    match freshness {
        MemoryFreshness::Rolling => Some(Duration::days(30)),
        MemoryFreshness::Durable | MemoryFreshness::Archival => None,
    }
}

/// Determine whether a governed memory record should be considered for promotion into MEMORY.md.
pub fn is_memory_promotion_candidate(metadata: &MemoryRecordMetadata) -> bool {
    matches!(metadata.freshness, MemoryFreshness::Durable)
        && matches!(
            metadata.kind.as_str(),
            "preference" | "decision" | "constraint" | "profile" | "project_state"
        )
}

/// Compute lifecycle fields for a governed memory record.
pub fn memory_lifecycle_snapshot(
    metadata: &MemoryRecordMetadata,
    now: DateTime<Utc>,
) -> MemoryLifecycleSnapshot {
    let review_at = metadata.updated_at + lifecycle_review_window(&metadata.freshness);
    let expires_at =
        lifecycle_expiry_window(&metadata.freshness).map(|duration| metadata.updated_at + duration);
    let state = match expires_at {
        Some(expires_at) if now >= expires_at => MemoryLifecycleState::Expired,
        _ if now >= review_at => MemoryLifecycleState::Stale,
        _ => MemoryLifecycleState::Active,
    };

    MemoryLifecycleSnapshot {
        state,
        review_at,
        expires_at,
        promotion_candidate: is_memory_promotion_candidate(metadata),
    }
}

fn is_priority_memory_kind(kind: &str) -> bool {
    matches!(
        kind,
        "preference" | "decision" | "constraint" | "profile" | "project_state"
    )
}

fn memory_lifecycle_sort_rank(state: MemoryLifecycleState) -> u8 {
    match state {
        MemoryLifecycleState::Active => 0,
        MemoryLifecycleState::Stale => 1,
        MemoryLifecycleState::Expired => 2,
    }
}

fn memory_freshness_sort_rank(freshness: &MemoryFreshness) -> u8 {
    match freshness {
        MemoryFreshness::Durable => 0,
        MemoryFreshness::Rolling => 1,
        MemoryFreshness::Archival => 2,
    }
}

/// Render a governed freshness enum as a stable lowercase label.
pub fn render_memory_freshness(freshness: &MemoryFreshness) -> &'static str {
    match freshness {
        MemoryFreshness::Rolling => "rolling",
        MemoryFreshness::Durable => "durable",
        MemoryFreshness::Archival => "archival",
    }
}

/// Render a governed lifecycle state as a stable lowercase label.
pub fn render_memory_lifecycle_state(state: MemoryLifecycleState) -> &'static str {
    match state {
        MemoryLifecycleState::Active => "active",
        MemoryLifecycleState::Stale => "stale",
        MemoryLifecycleState::Expired => "expired",
    }
}

#[derive(Debug, Clone, Default)]
struct MemoryQueryProfile {
    terms: Vec<String>,
    phrases: Vec<String>,
    namespace_hints: Vec<String>,
    kind_hints: Vec<String>,
}

impl MemoryQueryProfile {
    fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.phrases.is_empty()
            && self.namespace_hints.is_empty()
            && self.kind_hints.is_empty()
    }
}

fn is_memory_query_stopword(term: &str) -> bool {
    matches!(
        term,
        "a" | "an"
            | "and"
            | "are"
            | "at"
            | "be"
            | "before"
            | "but"
            | "by"
            | "can"
            | "did"
            | "do"
            | "does"
            | "for"
            | "from"
            | "how"
            | "i"
            | "if"
            | "in"
            | "into"
            | "is"
            | "it"
            | "me"
            | "my"
            | "now"
            | "of"
            | "on"
            | "or"
            | "our"
            | "right"
            | "should"
            | "that"
            | "the"
            | "their"
            | "them"
            | "there"
            | "these"
            | "they"
            | "this"
            | "to"
            | "up"
            | "use"
            | "we"
            | "what"
            | "when"
            | "where"
            | "which"
            | "who"
            | "why"
            | "you"
            | "your"
    )
}

fn normalize_memory_query_text(input: &str) -> String {
    let mut normalized = String::with_capacity(input.len());
    let mut previous_was_space = false;

    for ch in input.chars() {
        let normalized_char = if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
            ch.to_ascii_lowercase()
        } else {
            ' '
        };

        if normalized_char == ' ' {
            if !previous_was_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            previous_was_space = true;
        } else {
            normalized.push(normalized_char);
            previous_was_space = false;
        }
    }

    normalized.trim().to_string()
}

fn memory_query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut seen = HashSet::new();

    for term in normalize_memory_query_text(query)
        .split_whitespace()
        .filter(|term| term.len() >= 2)
        .filter(|term| !is_memory_query_stopword(term))
    {
        let term = term.to_string();
        if seen.insert(term.clone()) {
            terms.push(term);
        }
    }

    terms
}

fn memory_query_phrases(terms: &[String]) -> Vec<String> {
    let mut phrases = Vec::new();
    let mut seen = HashSet::new();

    for width in [3, 2] {
        for window in terms.windows(width) {
            let phrase = window.join(" ");
            if seen.insert(phrase.clone()) {
                phrases.push(phrase);
            }
        }
    }

    phrases
}

fn push_unique_hint(hints: &mut Vec<String>, hint: &str) {
    if !hints.iter().any(|existing| existing == hint) {
        hints.push(hint.to_string());
    }
}

fn memory_query_profile(query: &str) -> MemoryQueryProfile {
    let terms = memory_query_terms(query);
    let phrases = memory_query_phrases(&terms);
    let mut namespace_hints = Vec::new();
    let mut kind_hints = Vec::new();

    for term in &terms {
        match term.as_str() {
            "blocked" | "blocker" | "launch" | "milestone" | "project" | "qa" | "release"
            | "roadmap" | "ship" | "shipping" | "status" => {
                push_unique_hint(&mut namespace_hints, "project");
                push_unique_hint(&mut kind_hints, "project_state");
            }
            "constraint" | "constraints" | "must" | "policy" | "requirement" | "required" => {
                push_unique_hint(&mut kind_hints, "constraint");
            }
            "decide" | "decided" | "decision" | "choice" | "chosen" => {
                push_unique_hint(&mut kind_hints, "decision");
            }
            "prefer" | "preference" | "preferences" | "preferred" | "style" | "theme" | "tone"
            | "format" | "formatting" | "ux" | "ui" => {
                push_unique_hint(&mut namespace_hints, "pref");
                push_unique_hint(&mut kind_hints, "preference");
            }
            "profile" | "background" | "bio" => {
                push_unique_hint(&mut kind_hints, "profile");
            }
            _ => {}
        }
    }

    MemoryQueryProfile {
        terms,
        phrases,
        namespace_hints,
        kind_hints,
    }
}

fn memory_query_match_score(
    candidate: &GovernedMemoryPromptCandidate,
    query_profile: &MemoryQueryProfile,
) -> usize {
    if query_profile.is_empty() {
        return 0;
    }

    let key_tokens = memory_query_terms(&candidate.key);
    let normalized_key = normalize_memory_query_text(&candidate.key);
    let namespace = candidate.metadata.namespace.to_lowercase();
    let kind = candidate.metadata.kind.to_lowercase();
    let lowered_tags: Vec<String> = candidate
        .metadata
        .tags
        .iter()
        .map(|tag| tag.to_lowercase())
        .collect();
    let normalized_value = match &candidate.value {
        serde_json::Value::String(text) => normalize_memory_query_text(text),
        other => normalize_memory_query_text(&serde_json::to_string(other).unwrap_or_default()),
    };

    let term_score: usize = query_profile
        .terms
        .iter()
        .map(|term| {
            let mut score = 0;

            if lowered_tags.iter().any(|tag| tag == term) {
                score += 8;
            }
            if key_tokens.iter().any(|token| token == term) {
                score += 7;
            } else if namespace == *term {
                score += 6;
            } else if normalized_key.contains(term) {
                score += 3;
            }
            if kind == *term {
                score += 6;
            }
            if normalized_value.contains(term) {
                score += 2;
            }

            score
        })
        .sum();

    let phrase_score: usize = query_profile
        .phrases
        .iter()
        .map(|phrase| {
            let mut score = 0;

            if normalized_key.contains(phrase) {
                score += 10;
            }
            if normalized_value.contains(phrase) {
                score += 5;
            }
            if lowered_tags
                .iter()
                .map(|tag| normalize_memory_query_text(tag))
                .any(|tag| tag == *phrase)
            {
                score += 6;
            }

            score
        })
        .sum();

    let namespace_hint_score = if query_profile
        .namespace_hints
        .iter()
        .any(|hint| hint == &namespace)
    {
        8
    } else {
        0
    };
    let kind_hint_score = if query_profile.kind_hints.iter().any(|hint| hint == &kind) {
        7
    } else {
        0
    };

    term_score + phrase_score + namespace_hint_score + kind_hint_score
}

fn collect_governed_memory_prompt_candidates(
    entries: &[(String, serde_json::Value)],
    now: DateTime<Utc>,
) -> Vec<GovernedMemoryPromptCandidate> {
    let metadata_map = collect_memory_metadata(entries);

    entries
        .iter()
        .filter_map(|(key, value)| {
            if is_internal_memory_key(key) || is_memory_metadata_key(key) {
                return None;
            }

            let metadata = metadata_map.get(key)?.clone();
            let lifecycle = memory_lifecycle_snapshot(&metadata, now);
            if lifecycle.state == MemoryLifecycleState::Expired {
                return None;
            }

            if !(is_priority_memory_kind(&metadata.kind)
                || lifecycle.promotion_candidate
                || !metadata.tags.is_empty())
            {
                return None;
            }

            Some(GovernedMemoryPromptCandidate {
                key: key.clone(),
                value: value.clone(),
                metadata,
                lifecycle,
            })
        })
        .collect()
}

#[derive(Debug, Clone)]
struct ScoredGovernedMemoryPromptCandidate {
    candidate: GovernedMemoryPromptCandidate,
    query_match_score: usize,
}

fn collect_scored_governed_memory_prompt_candidates(
    entries: &[(String, serde_json::Value)],
    now: DateTime<Utc>,
    query_profile: &MemoryQueryProfile,
) -> Vec<ScoredGovernedMemoryPromptCandidate> {
    collect_governed_memory_prompt_candidates(entries, now)
        .into_iter()
        .map(|candidate| ScoredGovernedMemoryPromptCandidate {
            query_match_score: memory_query_match_score(&candidate, query_profile),
            candidate,
        })
        .collect()
}

fn compare_scored_governed_memory_candidates(
    a: &ScoredGovernedMemoryPromptCandidate,
    b: &ScoredGovernedMemoryPromptCandidate,
) -> std::cmp::Ordering {
    b.query_match_score
        .cmp(&a.query_match_score)
        .then_with(|| {
            memory_lifecycle_sort_rank(a.candidate.lifecycle.state)
                .cmp(&memory_lifecycle_sort_rank(b.candidate.lifecycle.state))
        })
        .then_with(|| {
            (!a.candidate.lifecycle.promotion_candidate)
                .cmp(&(!b.candidate.lifecycle.promotion_candidate))
        })
        .then_with(|| {
            a.candidate
                .metadata
                .tags
                .is_empty()
                .cmp(&b.candidate.metadata.tags.is_empty())
        })
        .then_with(|| {
            is_priority_memory_kind(&a.candidate.metadata.kind)
                .cmp(&is_priority_memory_kind(&b.candidate.metadata.kind))
                .reverse()
        })
        .then_with(|| {
            memory_freshness_sort_rank(&a.candidate.metadata.freshness)
                .cmp(&memory_freshness_sort_rank(&b.candidate.metadata.freshness))
        })
        .then_with(|| b.candidate.metadata.updated_at.cmp(&a.candidate.metadata.updated_at))
        .then_with(|| a.candidate.key.cmp(&b.candidate.key))
}

fn governed_memory_source_weight(
    candidate: &GovernedMemoryPromptCandidate,
    query_match_score: usize,
) -> f32 {
    let query_weight = match query_match_score {
        24.. => 1.35,
        8..=23 => 1.20,
        1..=7 => 1.10,
        _ => 1.00,
    };
    let lifecycle_weight = match candidate.lifecycle.state {
        MemoryLifecycleState::Active => 1.10,
        MemoryLifecycleState::Stale => 1.00,
        MemoryLifecycleState::Expired => 0.90,
    };
    let promotion_weight = if candidate.lifecycle.promotion_candidate {
        1.05
    } else {
        1.00
    };

    query_weight * lifecycle_weight * promotion_weight
}

fn governed_memory_tie_break_priority(
    candidate: &GovernedMemoryPromptCandidate,
    query_match_score: usize,
) -> u8 {
    let query_bucket = if query_match_score > 0 { 0 } else { 1 };
    let lifecycle_bucket = match candidate.lifecycle.state {
        MemoryLifecycleState::Active => 0,
        MemoryLifecycleState::Stale => 1,
        MemoryLifecycleState::Expired => 2,
    };
    let promotion_bucket = if candidate.lifecycle.promotion_candidate {
        0
    } else {
        1
    };

    query_bucket * 6 + lifecycle_bucket * 2 + promotion_bucket
}

fn select_scored_governed_memory_prompt_candidates_for_query(
    entries: &[(String, serde_json::Value)],
    now: DateTime<Utc>,
    limit: usize,
    query: Option<&str>,
) -> Vec<ScoredGovernedMemoryPromptCandidate> {
    if limit == 0 {
        return Vec::new();
    }

    let query_profile = query.map(memory_query_profile).unwrap_or_default();
    let mut candidates =
        collect_scored_governed_memory_prompt_candidates(entries, now, &query_profile);
    candidates.sort_by(compare_scored_governed_memory_candidates);
    candidates.truncate(limit);
    candidates
}

/// Select governed user-memory candidates and expose fusion weights / tie-break metadata.
pub fn select_governed_memory_prompt_candidates_for_query_with_fusion(
    entries: &[(String, serde_json::Value)],
    now: DateTime<Utc>,
    limit: usize,
    query: Option<&str>,
) -> Vec<GovernedMemoryPromptSelection> {
    select_scored_governed_memory_prompt_candidates_for_query(entries, now, limit, query)
        .into_iter()
        .map(|selection| GovernedMemoryPromptSelection {
            source_weight: governed_memory_source_weight(
                &selection.candidate,
                selection.query_match_score,
            ),
            tie_break_priority: governed_memory_tie_break_priority(
                &selection.candidate,
                selection.query_match_score,
            ),
            query_match_score: selection.query_match_score,
            candidate: selection.candidate,
        })
        .collect()
}

/// Select governed user-memory candidates suitable for prompt-level retrieval.
///
/// The output is intentionally biased toward durable preferences, decisions,
/// constraints, profile, and project-state records, while still allowing tagged
/// governed records to surface. Expired lifecycle entries are excluded.
pub fn select_governed_memory_prompt_candidates(
    entries: &[(String, serde_json::Value)],
    now: DateTime<Utc>,
    limit: usize,
) -> Vec<GovernedMemoryPromptCandidate> {
    select_governed_memory_prompt_candidates_for_query(entries, now, limit, None)
}

/// Select governed user-memory candidates and rerank them against the current query.
pub fn select_governed_memory_prompt_candidates_for_query(
    entries: &[(String, serde_json::Value)],
    now: DateTime<Utc>,
    limit: usize,
    query: Option<&str>,
) -> Vec<GovernedMemoryPromptCandidate> {
    select_governed_memory_prompt_candidates_for_query_with_fusion(entries, now, limit, query)
        .into_iter()
        .map(|selection| selection.candidate)
        .collect()
}

/// Summarize governed memory review/promotion actions for higher-level prompt orchestration.
pub fn summarize_governed_memory_orchestration_for_query(
    entries: &[(String, serde_json::Value)],
    now: DateTime<Utc>,
    limit_per_bucket: usize,
    query: Option<&str>,
) -> GovernedMemoryOrchestrationSnapshot {
    if limit_per_bucket == 0 {
        return GovernedMemoryOrchestrationSnapshot::default();
    }

    let candidates = select_scored_governed_memory_prompt_candidates_for_query(
        entries,
        now,
        usize::MAX,
        query,
    );

    let mut stale_review: Vec<ScoredGovernedMemoryPromptCandidate> = candidates
        .iter()
        .filter(|candidate| candidate.candidate.lifecycle.state == MemoryLifecycleState::Stale)
        .cloned()
        .collect();
    stale_review.truncate(limit_per_bucket);

    let mut promotion_candidates: Vec<ScoredGovernedMemoryPromptCandidate> = candidates
        .iter()
        .filter(|candidate| candidate.candidate.lifecycle.promotion_candidate)
        .cloned()
        .collect();
    promotion_candidates.truncate(limit_per_bucket);

    GovernedMemoryOrchestrationSnapshot {
        stale_review: stale_review
            .into_iter()
            .map(|candidate| GovernedMemoryOrchestrationSignal {
                key: candidate.candidate.key,
                metadata: candidate.candidate.metadata,
                lifecycle: candidate.candidate.lifecycle,
            })
            .collect(),
        promotion_candidates: promotion_candidates
            .into_iter()
            .map(|candidate| GovernedMemoryOrchestrationSignal {
                key: candidate.candidate.key,
                metadata: candidate.candidate.metadata,
                lifecycle: candidate.candidate.lifecycle,
            })
            .collect(),
    }
}

/// Fuse ranked prompt-time memory candidates from multiple sources using weighted RRF.
pub fn fuse_ranked_memory_context_candidates(
    candidates: Vec<RankedMemoryContextCandidate>,
    limit: usize,
) -> Vec<FusedMemoryContextCandidate> {
    if limit == 0 {
        return Vec::new();
    }

    let mut fused: Vec<FusedMemoryContextCandidate> = candidates
        .into_iter()
        .map(|candidate| FusedMemoryContextCandidate {
            fused_score: candidate.source_weight
                * (1.0 / (MEMORY_CONTEXT_RRF_K + candidate.source_rank as f32 + 1.0)),
            rendered: candidate.rendered,
            source: candidate.source,
            source_rank: candidate.source_rank,
            source_weight: candidate.source_weight,
            tie_break_priority: candidate.tie_break_priority,
        })
        .collect();

    fused.sort_by(|a, b| {
        b.fused_score
            .partial_cmp(&a.fused_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.tie_break_priority.cmp(&b.tie_break_priority))
            .then_with(|| a.source_rank.cmp(&b.source_rank))
            .then_with(|| a.rendered.cmp(&b.rendered))
    });

    let mut seen = HashSet::new();
    fused.into_iter()
        .filter(|candidate| seen.insert(candidate.rendered.clone()))
        .take(limit)
        .collect()
}

/// Render semantic memory fragments into ranked prompt-time recall candidates.
pub fn rank_semantic_memory_context_candidates(
    memories: &[MemoryFragment],
    limit: usize,
) -> Vec<RankedMemoryContextCandidate> {
    if limit == 0 {
        return Vec::new();
    }

    let mut seen = HashSet::new();

    memories
        .iter()
        .filter_map(|memory| {
            let content = memory.content.trim();
            if content.is_empty() {
                return None;
            }

            let label = memory
                .metadata
                .get("key")
                .and_then(|value| value.as_str())
                .map(|key| format!("[{key}] "))
                .or_else(|| {
                    let scope = memory.scope.trim();
                    (!scope.is_empty()).then(|| format!("[{scope}] "))
                })
                .unwrap_or_default();

            let rendered = format!("Semantic memory {label}{}", crate::truncate_str(content, 320));
            if seen.insert(rendered.clone()) {
                Some(rendered)
            } else {
                None
            }
        })
        .take(limit)
        .enumerate()
        .map(|(rank, rendered)| RankedMemoryContextCandidate {
            rendered,
            source: MemoryContextSource::Semantic,
            source_rank: rank,
            source_weight: 1.0,
            tie_break_priority: 3,
        })
        .collect()
}

/// Render governed shared-memory prompt candidates into ranked prompt-time recall candidates.
pub fn rank_governed_memory_context_candidates_for_query(
    entries: &[(String, serde_json::Value)],
    now: DateTime<Utc>,
    limit: usize,
    query: Option<&str>,
) -> Vec<RankedMemoryContextCandidate> {
    if limit == 0 {
        return Vec::new();
    }

    let mut seen = HashSet::new();

    select_governed_memory_prompt_candidates_for_query_with_fusion(entries, now, limit, query)
        .into_iter()
        .enumerate()
        .filter_map(|(rank, selection)| {
            let rendered_value = match &selection.candidate.value {
                serde_json::Value::String(text) => text.clone(),
                other => serde_json::to_string(other).ok()?,
            };
            let rendered_value = rendered_value.trim();
            if rendered_value.is_empty() {
                return None;
            }

            let mut qualifiers = vec![
                format!("kind={}", selection.candidate.metadata.kind),
                format!(
                    "freshness={}",
                    render_memory_freshness(&selection.candidate.metadata.freshness)
                ),
                format!(
                    "lifecycle={}",
                    render_memory_lifecycle_state(selection.candidate.lifecycle.state)
                ),
            ];
            if !selection.candidate.metadata.tags.is_empty() {
                qualifiers.push(format!(
                    "tags={}",
                    selection.candidate.metadata.tags.join(",")
                ));
            }
            if selection.candidate.lifecycle.promotion_candidate {
                qualifiers.push("promotion_candidate".to_string());
            }

            let rendered = format!(
                "Shared memory [{}] ({}) {}",
                selection.candidate.key,
                qualifiers.join(", "),
                crate::truncate_str(rendered_value, 240)
            );

            if seen.insert(rendered.clone()) {
                Some(RankedMemoryContextCandidate {
                    rendered,
                    source: MemoryContextSource::Shared,
                    source_rank: rank,
                    source_weight: selection.source_weight,
                    tie_break_priority: selection.tie_break_priority,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Build the fused prompt-time memory context from semantic and governed shared memory.
pub fn build_fused_memory_context_for_query(
    query: &str,
    semantic_memories: &[MemoryFragment],
    governed_entries: &[(String, serde_json::Value)],
    semantic_limit: usize,
    shared_limit: usize,
    fused_limit: usize,
    now: DateTime<Utc>,
) -> MemoryContextFusionResult {
    if fused_limit == 0 {
        return MemoryContextFusionResult::default();
    }

    let semantic_candidates = rank_semantic_memory_context_candidates(semantic_memories, semantic_limit);
    let shared_candidates =
        rank_governed_memory_context_candidates_for_query(governed_entries, now, shared_limit, Some(query));
    let semantic_candidate_count = semantic_candidates.len();
    let shared_candidate_count = shared_candidates.len();

    let fused_candidates = fuse_ranked_memory_context_candidates(
        semantic_candidates
            .into_iter()
            .chain(shared_candidates)
            .collect(),
        fused_limit,
    );

    MemoryContextFusionResult {
        semantic_candidates: semantic_candidate_count,
        shared_candidates: shared_candidate_count,
        fused_candidates,
    }
}

fn cap_prompt_memory_context_text(s: &str, max_chars: usize) -> String {
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

/// Render recent `session_*` summaries for prompt-time memory context.
pub fn render_recent_session_summaries_from_entries(
    entries: &[(String, serde_json::Value)],
    limit: usize,
) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }

    let mut summaries: Vec<(String, String)> = entries
        .iter()
        .filter_map(|(key, value)| {
            if !key.starts_with("session_") {
                return None;
            }

            let rendered = match value {
                serde_json::Value::String(text) => text.clone(),
                other => serde_json::to_string(other).ok()?,
            };

            let rendered = rendered.trim();
            if rendered.is_empty() {
                return None;
            }

            Some((
                key.clone(),
                format!("{}: {}", key, crate::truncate_str(rendered, 320)),
            ))
        })
        .collect();

    summaries.sort_by(|a, b| b.0.cmp(&a.0));
    summaries
        .into_iter()
        .take(limit)
        .map(|(_, summary)| summary)
        .collect()
}

/// Render governed attention signals for stale review and promotion hints.
pub fn render_governed_memory_orchestration_signals_for_query(
    entries: &[(String, serde_json::Value)],
    now: DateTime<Utc>,
    limit_per_bucket: usize,
    query: Option<&str>,
) -> Vec<String> {
    let snapshot =
        summarize_governed_memory_orchestration_for_query(entries, now, limit_per_bucket, query);
    let mut rendered = Vec::new();

    for signal in snapshot.stale_review {
        let mut qualifiers = vec![
            format!("kind={}", signal.metadata.kind),
            format!("review_at={}", signal.lifecycle.review_at.to_rfc3339()),
        ];
        if let Some(expires_at) = signal.lifecycle.expires_at {
            qualifiers.push(format!("expires_at={}", expires_at.to_rfc3339()));
        }
        if !signal.metadata.tags.is_empty() {
            qualifiers.push(format!("tags={}", signal.metadata.tags.join(",")));
        }
        rendered.push(format!(
            "Review stale memory before reuse: [{}] ({})",
            signal.key,
            qualifiers.join(", ")
        ));
    }

    for signal in snapshot.promotion_candidates {
        let mut qualifiers = vec![
            format!("kind={}", signal.metadata.kind),
            format!(
                "freshness={}",
                render_memory_freshness(&signal.metadata.freshness)
            ),
            format!(
                "lifecycle={}",
                render_memory_lifecycle_state(signal.lifecycle.state)
            ),
        ];
        if !signal.metadata.tags.is_empty() {
            qualifiers.push(format!("tags={}", signal.metadata.tags.join(",")));
        }
        rendered.push(format!(
            "Consider promoting to MEMORY.md: [{}] ({})",
            signal.key,
            qualifiers.join(", ")
        ));
    }

    rendered
}

/// Render governance maintenance actions for prompt-time memory context.
pub fn render_memory_cleanup_orchestration_signals(
    entries: &[(String, serde_json::Value)],
    limit_per_bucket: usize,
) -> Vec<String> {
    let snapshot = summarize_memory_cleanup_for_orchestration(entries, limit_per_bucket);
    let mut rendered = Vec::new();

    for signal in snapshot.legacy_repairs {
        match signal.action {
            MemoryCleanupAction::MigrateLegacyKey => rendered.push(format!(
                "Run memory_cleanup before reuse: migrate legacy key [{}] to [{}]",
                signal.key,
                signal
                    .canonical_key
                    .unwrap_or_else(|| "unknown".to_string())
            )),
            MemoryCleanupAction::DeleteLegacyKey => rendered.push(format!(
                "Run memory_cleanup before reuse: delete duplicate legacy key [{}]",
                signal.key
            )),
            _ => {}
        }
    }

    for signal in snapshot.metadata_repairs {
        rendered.push(format!(
            "Run memory_cleanup to backfill governed metadata for [{}]",
            signal.key
        ));
    }

    for signal in snapshot.orphan_metadata {
        rendered.push(format!(
            "Run memory_cleanup to remove orphan metadata sidecar [{}]",
            signal.metadata_key.unwrap_or(signal.key)
        ));
    }

    rendered
}

/// Build a shared prompt-time memory context payload from semantic/shared/session sources.
pub fn build_prompt_memory_context(
    query: &str,
    semantic_mode: MemoryContextRecallMode,
    semantic_memories: &[MemoryFragment],
    governed_entries: &[(String, serde_json::Value)],
    structured_entries: &[(String, serde_json::Value)],
    options: &PromptMemoryContextBuildOptions,
    now: DateTime<Utc>,
) -> PromptMemoryContext {
    let fused_result = build_fused_memory_context_for_query(
        query,
        semantic_memories,
        governed_entries,
        options.semantic_limit,
        options.shared_limit,
        options.fused_limit,
        now,
    );
    let cleanup_maintenance_signals =
        render_memory_cleanup_orchestration_signals(governed_entries, options.maintenance_signal_limit);
    let governance_attention_signals = render_governed_memory_orchestration_signals_for_query(
        governed_entries,
        now,
        options.attention_signal_limit,
        Some(query),
    );
    let recent_session_summaries =
        render_recent_session_summaries_from_entries(structured_entries, options.session_summary_limit);
    let maintenance_signal_count = cleanup_maintenance_signals.len();
    let attention_signal_count = governance_attention_signals.len();
    let session_summary_count = recent_session_summaries.len();
    let recalled_memories = fused_result
        .fused_candidates
        .iter()
        .map(|candidate| candidate.rendered.clone())
        .collect();

    PromptMemoryContext {
        recalled_memories,
        cleanup_maintenance_signals,
        governance_attention_signals,
        recent_session_summaries,
        trace: PromptMemoryContextTrace {
            semantic_mode,
            semantic_candidates: fused_result.semantic_candidates,
            shared_candidates: fused_result.shared_candidates,
            fused_candidates: fused_result.fused_candidates,
            maintenance_signals: maintenance_signal_count,
            attention_signals: attention_signal_count,
            session_summaries: session_summary_count,
        },
    }
}

/// Render prompt-time memory context into the standalone user-message block used by runtime.
pub fn render_prompt_memory_context_message(context: &PromptMemoryContext) -> Option<String> {
    if context.recalled_memories.is_empty()
        && context.cleanup_maintenance_signals.is_empty()
        && context.governance_attention_signals.is_empty()
        && context.recent_session_summaries.is_empty()
    {
        return None;
    }

    let mut out = String::from("[Memory context]\n");

    if !context.recalled_memories.is_empty() {
        out.push_str("Relevant recalled memories:\n");
        for memory in context.recalled_memories.iter().take(5) {
            out.push_str(&format!(
                "- {}\n",
                cap_prompt_memory_context_text(memory, 320)
            ));
        }
    }

    if !context.cleanup_maintenance_signals.is_empty() {
        if !context.recalled_memories.is_empty() {
            out.push('\n');
        }
        out.push_str("Governance maintenance signals:\n");
        for signal in context.cleanup_maintenance_signals.iter().take(4) {
            out.push_str(&format!(
                "- {}\n",
                cap_prompt_memory_context_text(signal, 320)
            ));
        }
    }

    if !context.governance_attention_signals.is_empty() {
        if !context.recalled_memories.is_empty() || !context.cleanup_maintenance_signals.is_empty()
        {
            out.push('\n');
        }
        out.push_str("Governance attention signals:\n");
        for signal in context.governance_attention_signals.iter().take(4) {
            out.push_str(&format!(
                "- {}\n",
                cap_prompt_memory_context_text(signal, 320)
            ));
        }
    }

    if !context.recent_session_summaries.is_empty() {
        if !context.recalled_memories.is_empty()
            || !context.cleanup_maintenance_signals.is_empty()
            || !context.governance_attention_signals.is_empty()
        {
            out.push('\n');
        }
        out.push_str("Recent session summaries:\n");
        for summary in context.recent_session_summaries.iter().take(3) {
            out.push_str(&format!(
                "- {}\n",
                cap_prompt_memory_context_text(summary, 320)
            ));
        }
    }

    Some(out.trim_end().to_string())
}

/// Render the shared prompt-time memory trace payload used by `llm.log`.
pub fn render_prompt_memory_context_trace(trace: &PromptMemoryContextTrace) -> Option<String> {
    if trace.semantic_candidates == 0
        && trace.shared_candidates == 0
        && trace.maintenance_signals == 0
        && trace.attention_signals == 0
        && trace.session_summaries == 0
    {
        return None;
    }

    let mut out = String::new();
    out.push_str(&format!(
        "semantic_mode={}\nsemantic_candidates={}\nshared_candidates={}\nmaintenance_signals={}\nattention_signals={}\nsession_summaries={}\n",
        trace.semantic_mode.as_str(),
        trace.semantic_candidates,
        trace.shared_candidates,
        trace.maintenance_signals,
        trace.attention_signals,
        trace.session_summaries
    ));

    if !trace.fused_candidates.is_empty() {
        out.push_str("selected_fused_recall:\n");
        for (rank, candidate) in trace.fused_candidates.iter().enumerate() {
            out.push_str(&format!(
                "{}. source={} source_rank={} source_weight={:.3} tie_break_priority={} fused_score={:.5} {}\n",
                rank + 1,
                candidate.source.as_str(),
                candidate.source_rank + 1,
                candidate.source_weight,
                candidate.tie_break_priority,
                candidate.fused_score,
                crate::truncate_str(&candidate.rendered, 280)
            ));
        }
    }

    Some(out.trim_end().to_string())
}

/// An entity in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Unique entity ID.
    pub id: String,
    /// Entity type (Person, Organization, Project, etc.).
    pub entity_type: EntityType,
    /// Display name.
    pub name: String,
    /// Arbitrary properties.
    pub properties: HashMap<String, serde_json::Value>,
    /// When this entity was created.
    pub created_at: DateTime<Utc>,
    /// When this entity was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Types of entities in the knowledge graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    /// A person.
    Person,
    /// An organization.
    Organization,
    /// A project.
    Project,
    /// A concept or idea.
    Concept,
    /// An event.
    Event,
    /// A location.
    Location,
    /// A document.
    Document,
    /// A tool.
    Tool,
    /// A custom type.
    Custom(String),
}

/// A relation between two entities in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    /// Source entity ID.
    pub source: String,
    /// Relation type.
    pub relation: RelationType,
    /// Target entity ID.
    pub target: String,
    /// Arbitrary properties on the relation.
    pub properties: HashMap<String, serde_json::Value>,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f32,
    /// When this relation was created.
    pub created_at: DateTime<Utc>,
}

/// Types of relations in the knowledge graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    /// Entity works at an organization.
    WorksAt,
    /// Entity knows about a concept.
    KnowsAbout,
    /// Entities are related.
    RelatedTo,
    /// Entity depends on another.
    DependsOn,
    /// Entity is owned by another.
    OwnedBy,
    /// Entity was created by another.
    CreatedBy,
    /// Entity is located in another.
    LocatedIn,
    /// Entity is part of another.
    PartOf,
    /// Entity uses another.
    Uses,
    /// Entity produces another.
    Produces,
    /// A custom relation type.
    Custom(String),
}

/// A pattern for querying the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPattern {
    /// Optional source entity filter.
    pub source: Option<String>,
    /// Optional relation type filter.
    pub relation: Option<RelationType>,
    /// Optional target entity filter.
    pub target: Option<String>,
    /// Maximum traversal depth.
    pub max_depth: u32,
}

/// A result from a graph query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMatch {
    /// The source entity.
    pub source: Entity,
    /// The relation.
    pub relation: Relation,
    /// The target entity.
    pub target: Entity,
}

/// Report from memory consolidation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationReport {
    /// Number of memories merged.
    pub memories_merged: u64,
    /// Number of memories whose confidence decayed.
    pub memories_decayed: u64,
    /// How long the consolidation took.
    pub duration_ms: u64,
}

/// Format for memory export/import.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ExportFormat {
    /// JSON format.
    Json,
    /// MessagePack binary format.
    MessagePack,
}

/// Report from memory import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportReport {
    /// Number of entities imported.
    pub entities_imported: u64,
    /// Number of relations imported.
    pub relations_imported: u64,
    /// Number of memories imported.
    pub memories_imported: u64,
    /// Errors encountered during import.
    pub errors: Vec<String>,
}

/// The unified Memory trait that agents interact with.
///
/// This abstracts over the structured store (SQLite), semantic store,
/// and knowledge graph, presenting a single coherent API.
#[async_trait]
pub trait Memory: Send + Sync {
    // -- Key-value operations (structured store) --

    /// Get a value by key for a specific agent.
    async fn get(
        &self,
        agent_id: AgentId,
        key: &str,
    ) -> crate::error::OpenFangResult<Option<serde_json::Value>>;

    /// Set a key-value pair for a specific agent.
    async fn set(
        &self,
        agent_id: AgentId,
        key: &str,
        value: serde_json::Value,
    ) -> crate::error::OpenFangResult<()>;

    /// Delete a key-value pair for a specific agent.
    async fn delete(&self, agent_id: AgentId, key: &str) -> crate::error::OpenFangResult<()>;

    // -- Semantic operations --

    /// Store a new memory fragment.
    async fn remember(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
    ) -> crate::error::OpenFangResult<MemoryId>;

    /// Semantic search for relevant memories.
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
    ) -> crate::error::OpenFangResult<Vec<MemoryFragment>>;

    /// Soft-delete a memory fragment.
    async fn forget(&self, id: MemoryId) -> crate::error::OpenFangResult<()>;

    // -- Knowledge graph operations --

    /// Add an entity to the knowledge graph.
    async fn add_entity(&self, entity: Entity) -> crate::error::OpenFangResult<String>;

    /// Add a relation between entities.
    async fn add_relation(&self, relation: Relation) -> crate::error::OpenFangResult<String>;

    /// Query the knowledge graph.
    async fn query_graph(
        &self,
        pattern: GraphPattern,
    ) -> crate::error::OpenFangResult<Vec<GraphMatch>>;

    // -- Maintenance --

    /// Consolidate and optimize memory.
    async fn consolidate(&self) -> crate::error::OpenFangResult<ConsolidationReport>;

    /// Export all memory data.
    async fn export(&self, format: ExportFormat) -> crate::error::OpenFangResult<Vec<u8>>;

    /// Import memory data.
    async fn import(
        &self,
        data: &[u8],
        format: ExportFormat,
    ) -> crate::error::OpenFangResult<ImportReport>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_filter_agent() {
        let id = AgentId::new();
        let filter = MemoryFilter::agent(id);
        assert_eq!(filter.agent_id, Some(id));
        assert!(filter.source.is_none());
    }

    #[test]
    fn test_memory_fragment_serialization() {
        let fragment = MemoryFragment {
            id: MemoryId::new(),
            agent_id: AgentId::new(),
            content: "Test memory".to_string(),
            embedding: None,
            metadata: HashMap::new(),
            source: MemorySource::Conversation,
            confidence: 0.95,
            created_at: Utc::now(),
            accessed_at: Utc::now(),
            access_count: 0,
            scope: "episodic".to_string(),
        };
        let json = serde_json::to_string(&fragment).unwrap();
        let deserialized: MemoryFragment = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.content, "Test memory");
    }

    #[test]
    fn test_canonicalize_user_memory_key_adds_default_namespace() {
        assert_eq!(
            canonicalize_user_memory_key("user_name").unwrap(),
            "general.user_name"
        );
        assert_eq!(
            canonicalize_user_memory_key("project.alpha.status").unwrap(),
            "project.alpha.status"
        );
    }

    #[test]
    fn test_internal_memory_key_bypasses_namespace() {
        assert_eq!(
            canonicalize_user_memory_key("session_2026-03-13_summary").unwrap(),
            "session_2026-03-13_summary"
        );
        assert!(is_internal_memory_key("__openfang_schedules"));
        assert!(is_legacy_user_memory_key("user_name"));
        assert!(!is_legacy_user_memory_key("general.user_name"));
    }

    #[test]
    fn test_memory_lookup_candidates_preserve_legacy_key_and_canonical_key() {
        assert_eq!(
            memory_lookup_candidates("user_name").unwrap(),
            vec!["general.user_name".to_string(), "user_name".to_string()]
        );
        assert_eq!(
            memory_lookup_candidates("project.alpha.status").unwrap(),
            vec!["project.alpha.status".to_string()]
        );
    }

    #[test]
    fn test_memory_key_matches_prefix_handles_default_namespace() {
        assert!(memory_key_matches_prefix("general.user_name", "user").unwrap());
        assert!(memory_key_matches_prefix("project.alpha.status", "project.").unwrap());
        assert!(!memory_key_matches_prefix("__openfang_schedules", "user").unwrap());
    }

    #[test]
    fn test_memory_metadata_key_roundtrip() {
        let key = memory_metadata_key("user_name").unwrap();
        assert_eq!(key, "__openfang_memory_meta.general.user_name");
        assert_eq!(
            memory_key_from_metadata_key(&key).unwrap(),
            "general.user_name"
        );
    }

    #[test]
    fn test_build_memory_record_metadata_normalizes_fields() {
        let metadata = build_memory_record_metadata(
            "user_name",
            Some("Preference"),
            &[
                "Name".to_string(),
                "Name".to_string(),
                "Profile".to_string(),
            ],
            Some(MemoryFreshness::Rolling),
            "memory_store_tool",
        )
        .unwrap();

        assert_eq!(metadata.key, "general.user_name");
        assert_eq!(metadata.namespace, "general");
        assert_eq!(metadata.kind, "preference");
        assert_eq!(
            metadata.tags,
            vec!["name".to_string(), "profile".to_string()]
        );
        assert_eq!(metadata.freshness, MemoryFreshness::Rolling);
    }

    #[test]
    fn test_canonicalize_memory_tag_filters_flattens_csv_and_deduplicates() {
        let filters = canonicalize_memory_tag_filters(&[
            "Profile, project".to_string(),
            "profile".to_string(),
        ])
        .unwrap();

        assert_eq!(filters, vec!["profile".to_string(), "project".to_string()]);
    }

    #[test]
    fn test_memory_tags_match_requires_all_requested_tags() {
        let entry_tags = vec![
            "profile".to_string(),
            "project".to_string(),
            "alpha".to_string(),
        ];

        assert!(memory_tags_match(
            &entry_tags,
            &["profile".to_string(), "alpha".to_string()]
        ));
        assert!(!memory_tags_match(
            &entry_tags,
            &["profile".to_string(), "missing".to_string()]
        ));
    }

    #[test]
    fn test_collect_memory_metadata_ignores_non_metadata_entries() {
        let metadata =
            build_memory_record_metadata("user_name", Some("fact"), &[], None, "memory_store_tool")
                .unwrap();
        let entries = vec![
            ("general.user_name".to_string(), serde_json::json!("Alice")),
            (
                "__openfang_memory_meta.general.user_name".to_string(),
                serde_json::to_value(&metadata).unwrap(),
            ),
        ];

        let collected = collect_memory_metadata(&entries);
        assert_eq!(collected["general.user_name"].kind, "fact");
    }

    #[test]
    fn test_plan_memory_cleanup_detects_legacy_orphan_and_missing_metadata() {
        let governed_metadata = build_memory_record_metadata(
            "general.user_name",
            Some("fact"),
            &[],
            None,
            "memory_store_tool",
        )
        .unwrap();
        let entries = vec![
            ("user_name".to_string(), serde_json::json!("Alice")),
            (
                "general.user_name".to_string(),
                serde_json::json!("Alice v2"),
            ),
            (
                memory_metadata_key("general.user_name").unwrap(),
                serde_json::to_value(governed_metadata).unwrap(),
            ),
            (
                "project.alpha.status".to_string(),
                serde_json::json!("green"),
            ),
            (
                "__openfang_memory_meta.pref.theme".to_string(),
                serde_json::json!({
                    "schema_version": MEMORY_METADATA_SCHEMA_VERSION,
                    "key": "pref.theme",
                    "namespace": "pref",
                    "kind": "preference",
                    "tags": [],
                    "freshness": "durable",
                    "source": "memory_store_tool",
                    "updated_at": Utc::now().to_rfc3339(),
                }),
            ),
        ];

        let plan = plan_memory_cleanup(&entries);

        assert!(plan.findings.iter().any(|finding| {
            finding.action == MemoryCleanupAction::DeleteLegacyKey
                && finding.key == "user_name"
                && finding.canonical_key.as_deref() == Some("general.user_name")
        }));
        assert!(plan.findings.iter().any(|finding| {
            finding.action == MemoryCleanupAction::BackfillMetadata
                && finding.key == "project.alpha.status"
                && finding.metadata_key.as_deref()
                    == Some("__openfang_memory_meta.project.alpha.status")
        }));
        assert!(plan.findings.iter().any(|finding| {
            finding.action == MemoryCleanupAction::DeleteOrphanMetadata
                && finding.key == "pref.theme"
                && finding.metadata_key.as_deref() == Some("__openfang_memory_meta.pref.theme")
        }));
    }

    #[test]
    fn test_summarize_memory_cleanup_for_orchestration_groups_findings() {
        let entries = vec![
            ("legacy_theme".to_string(), serde_json::json!("solarized")),
            (
                "project.alpha.note".to_string(),
                serde_json::json!("Alpha note"),
            ),
            (
                memory_metadata_key("pref.orphan").unwrap(),
                serde_json::json!({
                    "schema_version": MEMORY_METADATA_SCHEMA_VERSION,
                    "key": "pref.orphan",
                    "namespace": "pref",
                    "kind": "preference",
                    "tags": ["profile"],
                    "freshness": "durable",
                    "source": "memory_store_tool",
                    "updated_at": Utc::now().to_rfc3339(),
                }),
            ),
        ];

        let snapshot = summarize_memory_cleanup_for_orchestration(&entries, 2);

        assert_eq!(snapshot.legacy_repairs.len(), 1);
        assert_eq!(
            snapshot.legacy_repairs[0].action,
            MemoryCleanupAction::MigrateLegacyKey
        );
        assert_eq!(snapshot.legacy_repairs[0].key, "legacy_theme");
        assert_eq!(
            snapshot.legacy_repairs[0].canonical_key.as_deref(),
            Some("general.legacy_theme")
        );
        assert_eq!(snapshot.metadata_repairs.len(), 1);
        assert_eq!(
            snapshot.metadata_repairs[0].action,
            MemoryCleanupAction::BackfillMetadata
        );
        assert_eq!(snapshot.metadata_repairs[0].key, "project.alpha.note");
        assert_eq!(snapshot.orphan_metadata.len(), 1);
        assert_eq!(
            snapshot.orphan_metadata[0].action,
            MemoryCleanupAction::DeleteOrphanMetadata
        );
        assert_eq!(
            snapshot.orphan_metadata[0].metadata_key.as_deref(),
            Some("__openfang_memory_meta.pref.orphan")
        );
    }

    #[test]
    fn test_memory_lifecycle_snapshot_marks_rolling_records_expired() {
        let metadata = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
            key: "general.user_name".to_string(),
            namespace: "general".to_string(),
            kind: "fact".to_string(),
            tags: vec![],
            freshness: MemoryFreshness::Rolling,
            source: "memory_store_tool".to_string(),
            updated_at: Utc::now() - Duration::days(45),
        };

        let snapshot = memory_lifecycle_snapshot(&metadata, Utc::now());
        assert_eq!(snapshot.state, MemoryLifecycleState::Expired);
        assert!(snapshot.expires_at.is_some());
        assert!(!snapshot.promotion_candidate);
    }

    #[test]
    fn test_memory_lifecycle_snapshot_marks_durable_records_as_promotion_candidates() {
        let metadata = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
            key: "general.user_name".to_string(),
            namespace: "general".to_string(),
            kind: "preference".to_string(),
            tags: vec!["profile".to_string()],
            freshness: MemoryFreshness::Durable,
            source: "memory_store_tool".to_string(),
            updated_at: Utc::now(),
        };

        let snapshot = memory_lifecycle_snapshot(&metadata, Utc::now());
        assert_eq!(snapshot.state, MemoryLifecycleState::Active);
        assert!(snapshot.expires_at.is_none());
        assert!(snapshot.promotion_candidate);
    }

    #[test]
    fn test_select_governed_memory_prompt_candidates_prioritizes_active_tagged_records() {
        let now = Utc::now();
        let preference_metadata = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
            key: "pref.editor.theme".to_string(),
            namespace: "pref".to_string(),
            kind: "preference".to_string(),
            tags: vec!["profile".to_string(), "ui".to_string()],
            freshness: MemoryFreshness::Durable,
            source: "memory_store_tool".to_string(),
            updated_at: now - Duration::days(1),
        };
        let project_metadata = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
            key: "project.alpha.status".to_string(),
            namespace: "project".to_string(),
            kind: "project_state".to_string(),
            tags: vec!["project".to_string(), "alpha".to_string()],
            freshness: MemoryFreshness::Rolling,
            source: "memory_store_tool".to_string(),
            updated_at: now - Duration::days(10),
        };
        let fact_metadata = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
            key: "general.fact_note".to_string(),
            namespace: "general".to_string(),
            kind: "fact".to_string(),
            tags: vec![],
            freshness: MemoryFreshness::Durable,
            source: "memory_store_tool".to_string(),
            updated_at: now,
        };
        let expired_metadata = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
            key: "project.legacy.note".to_string(),
            namespace: "project".to_string(),
            kind: "project_state".to_string(),
            tags: vec!["project".to_string()],
            freshness: MemoryFreshness::Rolling,
            source: "memory_store_tool".to_string(),
            updated_at: now - Duration::days(45),
        };
        let entries = vec![
            (
                "pref.editor.theme".to_string(),
                serde_json::json!("solarized dark"),
            ),
            (
                memory_metadata_key("pref.editor.theme").unwrap(),
                serde_json::to_value(&preference_metadata).unwrap(),
            ),
            (
                "project.alpha.status".to_string(),
                serde_json::json!("green"),
            ),
            (
                memory_metadata_key("project.alpha.status").unwrap(),
                serde_json::to_value(&project_metadata).unwrap(),
            ),
            (
                "general.fact_note".to_string(),
                serde_json::json!("ungrouped fact"),
            ),
            (
                memory_metadata_key("general.fact_note").unwrap(),
                serde_json::to_value(&fact_metadata).unwrap(),
            ),
            (
                "project.legacy.note".to_string(),
                serde_json::json!("expired"),
            ),
            (
                memory_metadata_key("project.legacy.note").unwrap(),
                serde_json::to_value(&expired_metadata).unwrap(),
            ),
        ];

        let candidates = select_governed_memory_prompt_candidates(&entries, now, 5);

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].key, "pref.editor.theme");
        assert_eq!(candidates[0].lifecycle.state, MemoryLifecycleState::Active);
        assert_eq!(candidates[1].key, "project.alpha.status");
        assert_eq!(candidates[1].lifecycle.state, MemoryLifecycleState::Stale);
    }

    #[test]
    fn test_select_governed_memory_prompt_candidates_for_query_prioritizes_matching_tags_and_keys()
    {
        let now = Utc::now();
        let preference_metadata = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
            key: "pref.editor.theme".to_string(),
            namespace: "pref".to_string(),
            kind: "preference".to_string(),
            tags: vec!["profile".to_string(), "ui".to_string()],
            freshness: MemoryFreshness::Durable,
            source: "memory_store_tool".to_string(),
            updated_at: now - Duration::days(2),
        };
        let project_metadata = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
            key: "project.alpha.status".to_string(),
            namespace: "project".to_string(),
            kind: "project_state".to_string(),
            tags: vec!["project".to_string(), "alpha".to_string()],
            freshness: MemoryFreshness::Rolling,
            source: "memory_store_tool".to_string(),
            updated_at: now - Duration::hours(1),
        };
        let entries = vec![
            (
                "pref.editor.theme".to_string(),
                serde_json::json!("Use compact bullet points."),
            ),
            (
                memory_metadata_key("pref.editor.theme").unwrap(),
                serde_json::to_value(&preference_metadata).unwrap(),
            ),
            (
                "project.alpha.status".to_string(),
                serde_json::json!("Alpha project is in progress."),
            ),
            (
                memory_metadata_key("project.alpha.status").unwrap(),
                serde_json::to_value(&project_metadata).unwrap(),
            ),
        ];

        let candidates = select_governed_memory_prompt_candidates_for_query(
            &entries,
            now,
            5,
            Some("What is the alpha project status right now?"),
        );

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].key, "project.alpha.status");
        assert_eq!(candidates[1].key, "pref.editor.theme");
    }

    #[test]
    fn test_memory_query_terms_drop_stopwords_and_build_phrases() {
        let profile = memory_query_profile("What is the alpha project status right now?");

        assert_eq!(profile.terms, vec!["alpha", "project", "status"]);
        assert_eq!(
            profile.phrases,
            vec!["alpha project status", "alpha project", "project status"]
        );
        assert_eq!(profile.namespace_hints, vec!["project"]);
        assert_eq!(profile.kind_hints, vec!["project_state"]);
    }

    #[test]
    fn test_select_governed_memory_prompt_candidates_for_query_uses_preference_hints() {
        let now = Utc::now();
        let preference_metadata = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
            key: "pref.reply.style".to_string(),
            namespace: "pref".to_string(),
            kind: "preference".to_string(),
            tags: vec!["profile".to_string()],
            freshness: MemoryFreshness::Durable,
            source: "memory_store_tool".to_string(),
            updated_at: now - Duration::days(3),
        };
        let project_metadata = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
            key: "project.alpha.status".to_string(),
            namespace: "project".to_string(),
            kind: "project_state".to_string(),
            tags: vec!["project".to_string(), "alpha".to_string()],
            freshness: MemoryFreshness::Rolling,
            source: "memory_store_tool".to_string(),
            updated_at: now - Duration::hours(1),
        };
        let entries = vec![
            (
                "pref.reply.style".to_string(),
                serde_json::json!("Use compact bullet lists with a short summary first."),
            ),
            (
                memory_metadata_key("pref.reply.style").unwrap(),
                serde_json::to_value(&preference_metadata).unwrap(),
            ),
            (
                "project.alpha.status".to_string(),
                serde_json::json!("Alpha launch is blocked on QA signoff."),
            ),
            (
                memory_metadata_key("project.alpha.status").unwrap(),
                serde_json::to_value(&project_metadata).unwrap(),
            ),
        ];

        let candidates = select_governed_memory_prompt_candidates_for_query(
            &entries,
            now,
            5,
            Some("How should you format replies to me?"),
        );

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].key, "pref.reply.style");
        assert_eq!(candidates[1].key, "project.alpha.status");
    }

    #[test]
    fn test_select_governed_memory_prompt_candidates_for_query_with_fusion_boosts_matching_active_record()
    {
        let now = Utc::now();
        let preference_metadata = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
            key: "pref.reply.style".to_string(),
            namespace: "pref".to_string(),
            kind: "preference".to_string(),
            tags: vec!["profile".to_string()],
            freshness: MemoryFreshness::Durable,
            source: "memory_store_tool".to_string(),
            updated_at: now - Duration::days(3),
        };
        let project_metadata = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
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
                "pref.reply.style".to_string(),
                serde_json::json!("Use compact bullet lists with a short summary first."),
            ),
            (
                memory_metadata_key("pref.reply.style").unwrap(),
                serde_json::to_value(&preference_metadata).unwrap(),
            ),
            (
                "project.alpha.status".to_string(),
                serde_json::json!("Alpha launch is blocked on QA signoff."),
            ),
            (
                memory_metadata_key("project.alpha.status").unwrap(),
                serde_json::to_value(&project_metadata).unwrap(),
            ),
        ];

        let candidates = select_governed_memory_prompt_candidates_for_query_with_fusion(
            &entries,
            now,
            5,
            Some("What is the alpha project status right now?"),
        );

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].candidate.key, "project.alpha.status");
        assert!(candidates[0].query_match_score > 0);
        assert!(candidates[0].source_weight > 1.0);
        assert_eq!(candidates[0].tie_break_priority, 1);
        assert_eq!(candidates[1].candidate.key, "pref.reply.style");
    }

    #[test]
    fn test_fuse_ranked_memory_context_candidates_prefers_weighted_shared_candidate() {
        let fused = fuse_ranked_memory_context_candidates(
            vec![
                RankedMemoryContextCandidate {
                    rendered: "Semantic memory [episodic] unlock code".to_string(),
                    source: MemoryContextSource::Semantic,
                    source_rank: 0,
                    source_weight: 1.0,
                    tie_break_priority: 3,
                },
                RankedMemoryContextCandidate {
                    rendered: "Shared memory [project.alpha.status] QA blocker".to_string(),
                    source: MemoryContextSource::Shared,
                    source_rank: 0,
                    source_weight: 1.32,
                    tie_break_priority: 0,
                },
            ],
            5,
        );

        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].source, MemoryContextSource::Shared);
        assert!(fused[0].fused_score > fused[1].fused_score);
        assert_eq!(fused[1].source, MemoryContextSource::Semantic);
    }

    #[test]
    fn test_rank_semantic_memory_context_candidates_deduplicates_and_labels() {
        let now = Utc::now();
        let agent_id = AgentId::new();
        let memory = MemoryFragment {
            id: MemoryId::new(),
            agent_id,
            content: "Use rustfmt only when explicitly requested.".to_string(),
            embedding: None,
            metadata: HashMap::from([(
                "key".to_string(),
                serde_json::Value::String("pref.editor".to_string()),
            )]),
            source: MemorySource::UserProvided,
            confidence: 1.0,
            created_at: now,
            accessed_at: now,
            access_count: 0,
            scope: "preferences".to_string(),
        };

        let ranked = rank_semantic_memory_context_candidates(&[memory.clone(), memory], 5);

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].source, MemoryContextSource::Semantic);
        assert_eq!(ranked[0].source_rank, 0);
        assert!(ranked[0].rendered.starts_with("Semantic memory "));
        assert!(ranked[0].rendered.contains("[pref.editor]"));
    }

    #[test]
    fn test_build_fused_memory_context_for_query_uses_shared_helper_for_final_order() {
        let now = Utc::now();
        let agent_id = AgentId::new();
        let semantic_memory = MemoryFragment {
            id: MemoryId::new(),
            agent_id,
            content: "WEIGHT-SEMANTIC is the device unlock phrase.".to_string(),
            embedding: None,
            metadata: HashMap::new(),
            source: MemorySource::UserProvided,
            confidence: 1.0,
            created_at: now,
            accessed_at: now,
            access_count: 0,
            scope: "episodic".to_string(),
        };
        let project_metadata = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
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
                memory_metadata_key("project.alpha.status").unwrap(),
                serde_json::to_value(&project_metadata).unwrap(),
            ),
        ];

        let result = build_fused_memory_context_for_query(
            "What is the alpha project status and what is the device unlock phrase?",
            &[semantic_memory],
            &entries,
            5,
            10,
            5,
            now,
        );

        assert_eq!(result.semantic_candidates, 1);
        assert_eq!(result.shared_candidates, 1);
        assert_eq!(result.fused_candidates.len(), 2);
        assert_eq!(result.fused_candidates[0].source, MemoryContextSource::Shared);
        assert_eq!(result.fused_candidates[1].source, MemoryContextSource::Semantic);
        assert!(result.fused_candidates[0].fused_score > result.fused_candidates[1].fused_score);
    }

    #[test]
    fn test_summarize_governed_memory_orchestration_for_query_surfaces_stale_and_promotion() {
        let now = Utc::now();
        let stale_project = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
            key: "project.alpha.status".to_string(),
            namespace: "project".to_string(),
            kind: "project_state".to_string(),
            tags: vec!["project".to_string(), "alpha".to_string()],
            freshness: MemoryFreshness::Rolling,
            source: "memory_store_tool".to_string(),
            updated_at: now - Duration::days(8),
        };
        let durable_pref = MemoryRecordMetadata {
            schema_version: MEMORY_METADATA_SCHEMA_VERSION,
            key: "pref.editor.theme".to_string(),
            namespace: "pref".to_string(),
            kind: "preference".to_string(),
            tags: vec!["profile".to_string(), "ui".to_string()],
            freshness: MemoryFreshness::Durable,
            source: "memory_store_tool".to_string(),
            updated_at: now - Duration::days(1),
        };
        let entries = vec![
            (
                "project.alpha.status".to_string(),
                serde_json::json!("Alpha is blocked on QA."),
            ),
            (
                memory_metadata_key("project.alpha.status").unwrap(),
                serde_json::to_value(&stale_project).unwrap(),
            ),
            (
                "pref.editor.theme".to_string(),
                serde_json::json!("Use solarized dark."),
            ),
            (
                memory_metadata_key("pref.editor.theme").unwrap(),
                serde_json::to_value(&durable_pref).unwrap(),
            ),
        ];

        let snapshot = summarize_governed_memory_orchestration_for_query(
            &entries,
            now,
            2,
            Some("What is the alpha project status and ui preference?"),
        );

        assert_eq!(snapshot.stale_review.len(), 1);
        assert_eq!(snapshot.stale_review[0].key, "project.alpha.status");
        assert_eq!(
            snapshot.stale_review[0].lifecycle.state,
            MemoryLifecycleState::Stale
        );
        assert_eq!(snapshot.promotion_candidates.len(), 1);
        assert_eq!(snapshot.promotion_candidates[0].key, "pref.editor.theme");
    }
}
