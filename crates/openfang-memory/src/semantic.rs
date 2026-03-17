//! Semantic memory store with vector embedding support.
//!
//! Phase 1: SQLite LIKE matching (fallback when no embeddings).
//! Phase 2: Vector cosine similarity search using stored embeddings.
//!
//! Embeddings are stored as BLOBs in the `embedding` column of the memories table.
//! When a query embedding is provided, recall uses cosine similarity ranking.
//! When no embeddings are available, falls back to LIKE matching.

use chrono::Utc;
use openfang_types::agent::AgentId;
use openfang_types::error::{OpenFangError, OpenFangResult};
use openfang_types::memory::{MemoryFilter, MemoryFragment, MemoryId, MemorySource};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tracing::debug;

const HYBRID_RRF_K: f32 = 60.0;

/// Semantic store backed by SQLite with optional vector search.
#[derive(Clone)]
pub struct SemanticStore {
    conn: Arc<Mutex<Connection>>,
}

impl SemanticStore {
    /// Create a new semantic store wrapping the given connection.
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Store a new memory fragment (without embedding).
    pub fn remember(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
    ) -> OpenFangResult<MemoryId> {
        self.remember_with_embedding(agent_id, content, source, scope, metadata, None)
    }

    /// Store a new memory fragment with an optional embedding vector.
    pub fn remember_with_embedding(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
        embedding: Option<&[f32]>,
    ) -> OpenFangResult<MemoryId> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let id = MemoryId::new();
        let now = Utc::now().to_rfc3339();
        let source_str = serde_json::to_string(&source)
            .map_err(|e| OpenFangError::Serialization(e.to_string()))?;
        let meta_str = serde_json::to_string(&metadata)
            .map_err(|e| OpenFangError::Serialization(e.to_string()))?;
        let embedding_bytes: Option<Vec<u8>> = embedding.map(embedding_to_bytes);

        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted, embedding)
             VALUES (?1, ?2, ?3, ?4, ?5, 1.0, ?6, ?7, ?7, 0, 0, ?8)",
            rusqlite::params![
                id.0.to_string(),
                agent_id.0.to_string(),
                content,
                source_str,
                scope,
                meta_str,
                now,
                embedding_bytes,
            ],
        )
        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(id)
    }

    /// Search for memories using text matching (fallback, no embeddings).
    pub fn recall(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
    ) -> OpenFangResult<Vec<MemoryFragment>> {
        self.recall_with_embedding(query, limit, filter, None)
    }

    /// Search for memories using vector similarity when a query embedding is provided,
    /// falling back to LIKE matching otherwise.
    pub fn recall_with_embedding(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
        query_embedding: Option<&[f32]>,
    ) -> OpenFangResult<Vec<MemoryFragment>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;

        let mut sql = String::from(
            "SELECT id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, embedding
             FROM memories WHERE deleted = 0",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1;

        // Text search filter (only when no embeddings — vector search handles relevance)
        if query_embedding.is_none() && !query.is_empty() {
            sql.push_str(&format!(" AND content LIKE ?{param_idx}"));
            params.push(Box::new(format!("%{query}%")));
            param_idx += 1;
        }

        // Apply filters
        if let Some(ref f) = filter {
            if let Some(agent_id) = f.agent_id {
                sql.push_str(&format!(" AND agent_id = ?{param_idx}"));
                params.push(Box::new(agent_id.0.to_string()));
                param_idx += 1;
            }
            if let Some(ref scope) = f.scope {
                sql.push_str(&format!(" AND scope = ?{param_idx}"));
                params.push(Box::new(scope.clone()));
                param_idx += 1;
            }
            if let Some(min_conf) = f.min_confidence {
                sql.push_str(&format!(" AND confidence >= ?{param_idx}"));
                params.push(Box::new(min_conf as f64));
                param_idx += 1;
            }
            if let Some(ref source) = f.source {
                let source_str = serde_json::to_string(source)
                    .map_err(|e| OpenFangError::Serialization(e.to_string()))?;
                sql.push_str(&format!(" AND source = ?{param_idx}"));
                params.push(Box::new(source_str));
                let _ = param_idx;
            }
        }

        sql.push_str(" ORDER BY accessed_at DESC, access_count DESC");
        if query_embedding.is_none() {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let id_str: String = row.get(0)?;
                let agent_str: String = row.get(1)?;
                let content: String = row.get(2)?;
                let source_str: String = row.get(3)?;
                let scope: String = row.get(4)?;
                let confidence: f64 = row.get(5)?;
                let meta_str: String = row.get(6)?;
                let created_str: String = row.get(7)?;
                let accessed_str: String = row.get(8)?;
                let access_count: i64 = row.get(9)?;
                let embedding_bytes: Option<Vec<u8>> = row.get(10)?;
                Ok((
                    id_str,
                    agent_str,
                    content,
                    source_str,
                    scope,
                    confidence,
                    meta_str,
                    created_str,
                    accessed_str,
                    access_count,
                    embedding_bytes,
                ))
            })
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;

        let mut fragments = Vec::new();
        for row_result in rows {
            let (
                id_str,
                agent_str,
                content,
                source_str,
                scope,
                confidence,
                meta_str,
                created_str,
                accessed_str,
                access_count,
                embedding_bytes,
            ) = row_result.map_err(|e| OpenFangError::Memory(e.to_string()))?;

            let id = uuid::Uuid::parse_str(&id_str)
                .map(MemoryId)
                .map_err(|e| OpenFangError::Memory(e.to_string()))?;
            let agent_id = uuid::Uuid::parse_str(&agent_str)
                .map(openfang_types::agent::AgentId)
                .map_err(|e| OpenFangError::Memory(e.to_string()))?;
            let source: MemorySource =
                serde_json::from_str(&source_str).unwrap_or(MemorySource::System);
            let metadata: HashMap<String, serde_json::Value> =
                serde_json::from_str(&meta_str).unwrap_or_default();
            let created_at = chrono::DateTime::parse_from_rfc3339(&created_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let accessed_at = chrono::DateTime::parse_from_rfc3339(&accessed_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let embedding = embedding_bytes.as_deref().map(embedding_from_bytes);

            fragments.push(MemoryFragment {
                id,
                agent_id,
                content,
                embedding,
                metadata,
                source,
                confidence: confidence as f32,
                created_at,
                accessed_at,
                access_count: access_count as u64,
                scope,
            });
        }

        if query_embedding.is_some() {
            fragments = hybrid_rank_fragments(fragments, query, query_embedding, limit);
        } else {
            debug!("Text recall: {} results", fragments.len());
        }

        // Update access counts for returned memories
        for frag in &fragments {
            let _ = conn.execute(
                "UPDATE memories SET access_count = access_count + 1, accessed_at = ?1 WHERE id = ?2",
                rusqlite::params![Utc::now().to_rfc3339(), frag.id.0.to_string()],
            );
        }

        Ok(fragments)
    }

    /// Soft-delete a memory fragment.
    pub fn forget(&self, id: MemoryId) -> OpenFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        conn.execute(
            "UPDATE memories SET deleted = 1 WHERE id = ?1",
            rusqlite::params![id.0.to_string()],
        )
        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Update the embedding for an existing memory.
    pub fn update_embedding(&self, id: MemoryId, embedding: &[f32]) -> OpenFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let bytes = embedding_to_bytes(embedding);
        conn.execute(
            "UPDATE memories SET embedding = ?1 WHERE id = ?2",
            rusqlite::params![bytes, id.0.to_string()],
        )
        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(())
    }
}

fn normalize_query_text(input: &str) -> String {
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

fn query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut seen = HashSet::new();

    for term in normalize_query_text(query)
        .split_whitespace()
        .filter(|term| term.len() >= 2)
    {
        let term = term.to_string();
        if seen.insert(term.clone()) {
            terms.push(term);
        }
    }

    terms
}

fn fragment_search_text(fragment: &MemoryFragment) -> String {
    let metadata_text = serde_json::to_string(&fragment.metadata).unwrap_or_default();
    normalize_query_text(&format!(
        "{} {} {} {}",
        fragment.content,
        fragment.scope,
        fragment.source_string(),
        metadata_text
    ))
}

fn text_match_score(fragment: &MemoryFragment, query: &str, query_terms: &[String]) -> usize {
    if query.is_empty() {
        return 0;
    }

    let haystack = fragment_search_text(fragment);
    if haystack.is_empty() {
        return 0;
    }

    let mut score = 0;
    if haystack.contains(query) {
        score += 12;
    }

    for term in query_terms {
        if haystack.contains(term) {
            score += 4;
        }
    }

    score
}

fn reciprocal_rank_fusion(score: &mut f32, rank: usize) {
    *score += 1.0 / (HYBRID_RRF_K + rank as f32 + 1.0);
}

fn append_remaining_fragments(
    ordered: &mut Vec<usize>,
    fragments: &[MemoryFragment],
    limit: usize,
) {
    let seen: HashSet<usize> = ordered.iter().copied().collect();
    for idx in 0..fragments.len() {
        if seen.contains(&idx) {
            continue;
        }
        ordered.push(idx);
        if ordered.len() >= limit {
            break;
        }
    }
}

fn hybrid_rank_fragments(
    fragments: Vec<MemoryFragment>,
    query: &str,
    query_embedding: Option<&[f32]>,
    limit: usize,
) -> Vec<MemoryFragment> {
    if limit == 0 || fragments.is_empty() {
        return Vec::new();
    }

    let normalized_query = normalize_query_text(query);
    let query_terms = query_terms(query);

    let mut text_ranked: Vec<(usize, usize)> = fragments
        .iter()
        .enumerate()
        .filter_map(|(idx, fragment)| {
            let score = text_match_score(fragment, &normalized_query, &query_terms);
            (score > 0).then_some((idx, score))
        })
        .collect();
    text_ranked.sort_by(|(idx_a, score_a), (idx_b, score_b)| {
        score_b
            .cmp(score_a)
            .then_with(|| {
                fragments[*idx_b]
                    .accessed_at
                    .cmp(&fragments[*idx_a].accessed_at)
            })
            .then_with(|| {
                fragments[*idx_b]
                    .access_count
                    .cmp(&fragments[*idx_a].access_count)
            })
            .then_with(|| fragments[*idx_a].id.0.cmp(&fragments[*idx_b].id.0))
    });

    let vector_ranked: Vec<(usize, f32)> = query_embedding
        .map(|embedding| {
            let mut ranked: Vec<(usize, f32)> = fragments
                .iter()
                .enumerate()
                .filter_map(|(idx, fragment)| {
                    fragment
                        .embedding
                        .as_deref()
                        .and_then(|fragment_embedding| {
                            (fragment_embedding.len() == embedding.len())
                                .then_some((idx, cosine_similarity(embedding, fragment_embedding)))
                        })
                })
                .collect();
            ranked.sort_by(|(idx_a, score_a), (idx_b, score_b)| {
                score_b
                    .partial_cmp(score_a)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        fragments[*idx_b]
                            .accessed_at
                            .cmp(&fragments[*idx_a].accessed_at)
                    })
                    .then_with(|| {
                        fragments[*idx_b]
                            .access_count
                            .cmp(&fragments[*idx_a].access_count)
                    })
                    .then_with(|| fragments[*idx_a].id.0.cmp(&fragments[*idx_b].id.0))
            });
            ranked
        })
        .unwrap_or_default();

    if text_ranked.is_empty() && vector_ranked.is_empty() {
        return fragments.into_iter().take(limit).collect();
    }

    if text_ranked.is_empty() {
        debug!(
            "Vector recall: {} results from {} vector candidates",
            limit.min(vector_ranked.len()),
            vector_ranked.len()
        );
        let mut ordered: Vec<usize> = vector_ranked.into_iter().map(|(idx, _)| idx).collect();
        append_remaining_fragments(&mut ordered, &fragments, limit);
        return ordered
            .into_iter()
            .take(limit)
            .map(|idx| fragments[idx].clone())
            .collect();
    }

    if vector_ranked.is_empty() {
        debug!(
            "Text recall: {} results from {} text candidates",
            limit.min(text_ranked.len()),
            text_ranked.len()
        );
        let mut ordered: Vec<usize> = text_ranked.into_iter().map(|(idx, _)| idx).collect();
        append_remaining_fragments(&mut ordered, &fragments, limit);
        return ordered
            .into_iter()
            .take(limit)
            .map(|idx| fragments[idx].clone())
            .collect();
    }

    let mut fused_scores: HashMap<usize, f32> = HashMap::new();
    let text_score_map: HashMap<usize, usize> = text_ranked.iter().copied().collect();
    let vector_score_map: HashMap<usize, f32> = vector_ranked.iter().copied().collect();

    for (rank, (idx, _)) in text_ranked.iter().enumerate() {
        reciprocal_rank_fusion(fused_scores.entry(*idx).or_insert(0.0), rank);
    }
    for (rank, (idx, _)) in vector_ranked.iter().enumerate() {
        reciprocal_rank_fusion(fused_scores.entry(*idx).or_insert(0.0), rank);
    }

    let mut ranked: Vec<(usize, f32)> = fused_scores.into_iter().collect();
    ranked.sort_by(|(idx_a, fused_a), (idx_b, fused_b)| {
        fused_b
            .partial_cmp(fused_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                text_score_map
                    .get(idx_b)
                    .unwrap_or(&0)
                    .cmp(text_score_map.get(idx_a).unwrap_or(&0))
            })
            .then_with(|| {
                vector_score_map
                    .get(idx_b)
                    .unwrap_or(&-1.0)
                    .partial_cmp(vector_score_map.get(idx_a).unwrap_or(&-1.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                fragments[*idx_b]
                    .accessed_at
                    .cmp(&fragments[*idx_a].accessed_at)
            })
            .then_with(|| {
                fragments[*idx_b]
                    .access_count
                    .cmp(&fragments[*idx_a].access_count)
            })
            .then_with(|| fragments[*idx_a].id.0.cmp(&fragments[*idx_b].id.0))
    });

    debug!(
        "Hybrid recall: {} results from {} text candidates and {} vector candidates",
        limit.min(ranked.len()),
        text_ranked.len(),
        vector_ranked.len()
    );

    let mut ordered: Vec<usize> = ranked.into_iter().map(|(idx, _)| idx).collect();
    append_remaining_fragments(&mut ordered, &fragments, limit);
    ordered
        .into_iter()
        .take(limit)
        .map(|idx| fragments[idx].clone())
        .collect()
}

trait MemoryFragmentSearchView {
    fn source_string(&self) -> &'static str;
}

impl MemoryFragmentSearchView for MemoryFragment {
    fn source_string(&self) -> &'static str {
        match self.source {
            MemorySource::Conversation => "conversation",
            MemorySource::Document => "document",
            MemorySource::Observation => "observation",
            MemorySource::Inference => "inference",
            MemorySource::UserProvided => "user_provided",
            MemorySource::System => "system",
        }
    }
}

/// Compute cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < f32::EPSILON {
        0.0
    } else {
        dot / denom
    }
}

/// Serialize embedding to bytes for SQLite BLOB storage.
fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// Deserialize embedding from bytes.
fn embedding_from_bytes(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn setup() -> SemanticStore {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        SemanticStore::new(Arc::new(Mutex::new(conn)))
    }

    #[test]
    fn test_remember_and_recall() {
        let store = setup();
        let agent_id = AgentId::new();
        store
            .remember(
                agent_id,
                "The user likes Rust programming",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        let results = store.recall("Rust", 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Rust"));
    }

    #[test]
    fn test_recall_with_filter() {
        let store = setup();
        let agent_id = AgentId::new();
        store
            .remember(
                agent_id,
                "Memory A",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        store
            .remember(
                AgentId::new(),
                "Memory B",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        let filter = MemoryFilter::agent(agent_id);
        let results = store.recall("Memory", 10, Some(filter)).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "Memory A");
    }

    #[test]
    fn test_forget() {
        let store = setup();
        let agent_id = AgentId::new();
        let id = store
            .remember(
                agent_id,
                "To forget",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        store.forget(id).unwrap();
        let results = store.recall("To forget", 10, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_remember_with_embedding() {
        let store = setup();
        let agent_id = AgentId::new();
        let embedding = vec![0.1, 0.2, 0.3, 0.4];
        let id = store
            .remember_with_embedding(
                agent_id,
                "Rust is great",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&embedding),
            )
            .unwrap();
        assert_ne!(id.0.to_string(), "");
    }

    #[test]
    fn test_vector_recall_ranking() {
        let store = setup();
        let agent_id = AgentId::new();

        // Store 3 memories with embeddings pointing in different directions
        let emb_rust = vec![0.9, 0.1, 0.0, 0.0]; // "Rust" direction
        let emb_python = vec![0.0, 0.0, 0.9, 0.1]; // "Python" direction
        let emb_mixed = vec![0.5, 0.5, 0.0, 0.0]; // mixed

        store
            .remember_with_embedding(
                agent_id,
                "Rust is a systems language",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&emb_rust),
            )
            .unwrap();
        store
            .remember_with_embedding(
                agent_id,
                "Python is interpreted",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&emb_python),
            )
            .unwrap();
        store
            .remember_with_embedding(
                agent_id,
                "Both are popular",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&emb_mixed),
            )
            .unwrap();

        // Query with a "Rust"-like embedding
        let query_emb = vec![0.85, 0.15, 0.0, 0.0];
        let results = store
            .recall_with_embedding("", 3, None, Some(&query_emb))
            .unwrap();

        assert_eq!(results.len(), 3);
        // Rust memory should be first (highest cosine similarity)
        assert!(results[0].content.contains("Rust"));
        // Python memory should be last (lowest similarity)
        assert!(results[2].content.contains("Python"));
    }

    #[test]
    fn test_hybrid_recall_surfaces_keyword_match_without_embedding() {
        let store = setup();
        let agent_id = AgentId::new();

        store
            .remember(
                agent_id,
                "Export failed with error code E123 while syncing invoices",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        store
            .remember_with_embedding(
                agent_id,
                "A recent sync problem affected the billing workflow",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&[1.0, 0.0, 0.0, 0.0]),
            )
            .unwrap();

        let results = store
            .recall_with_embedding("E123 export", 2, None, Some(&[1.0, 0.0, 0.0, 0.0]))
            .unwrap();

        assert_eq!(results.len(), 2);
        assert!(results[0].content.contains("E123"));
    }

    #[test]
    fn test_update_embedding() {
        let store = setup();
        let agent_id = AgentId::new();
        let id = store
            .remember(
                agent_id,
                "No embedding yet",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();

        // Update with embedding
        let emb = vec![1.0, 0.0, 0.0];
        store.update_embedding(id, &emb).unwrap();

        // Verify the embedding is stored by doing vector recall
        let query_emb = vec![1.0, 0.0, 0.0];
        let results = store
            .recall_with_embedding("", 10, None, Some(&query_emb))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].embedding.is_some());
        assert_eq!(results[0].embedding.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_mixed_embedded_and_non_embedded() {
        let store = setup();
        let agent_id = AgentId::new();

        // One memory with embedding, one without
        store
            .remember_with_embedding(
                agent_id,
                "Has embedding",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&[1.0, 0.0]),
            )
            .unwrap();
        store
            .remember(
                agent_id,
                "No embedding",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();

        // Vector recall should rank embedded memory higher
        let results = store
            .recall_with_embedding("", 10, None, Some(&[1.0, 0.0]))
            .unwrap();
        assert_eq!(results.len(), 2);
        // Embedded memory should rank first
        assert_eq!(results[0].content, "Has embedding");
    }
}
