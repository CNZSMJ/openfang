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
}
