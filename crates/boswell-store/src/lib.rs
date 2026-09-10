//! Boswell Storage Layer
//!
//! Implements the ClaimStore trait using SQLite + HNSW vector index per ADR-005.
//!
//! # Architecture
//!
//! - SQLite for structured claim data (content, metadata, relationships)
//! - HNSW for vector similarity search (to be integrated)
//! - Local embedding model for duplicate detection
//!
//! # Examples
//!
//! ```no_run
//! use boswell_store::SqliteStore;
//!
//! // Create store without vector search
//! let store = SqliteStore::new(":memory:", false, 0).unwrap();
//! // Store is now ready for claim operations
//! ```

#![warn(missing_docs)]

pub mod embedding;
pub mod goal_store;
pub mod ollama_embedding;
pub mod procedure_store;
pub mod provenance_store;
pub mod receipt_store;
pub mod vector_index;

use boswell_domain::traits::{ClaimQuery, ClaimStore};
use boswell_domain::{decayed_confidence, DecayConfig};
use boswell_domain::{Claim, ClaimId, Relationship, RelationshipType};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use thiserror::Error;

pub use embedding::{cosine_similarity, EmbeddingModel, MockEmbeddingModel};
pub use ollama_embedding::OllamaEmbeddingModel;
pub use vector_index::{VectorIndex, VectorIndexError};

/// Errors that can occur during storage operations
#[derive(Error, Debug)]
pub enum StoreError {
    /// Database error
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// Claim not found
    #[error("Claim not found: {0}")]
    NotFound(String),

    /// Invalid data format
    #[error("Invalid data: {0}")]
    InvalidData(String),

    /// Duplicate claim detected
    #[error("Duplicate claim detected")]
    Duplicate,

    /// A goal edge would introduce a cycle into the goal DAG (design §8, #6).
    #[error("Edge would create a cycle in the goal DAG: {0}")]
    Cycle(String),

    /// A provenance-stamped write was refused because the author's authority does
    /// not permit it (namespace or op out of scope; design §5, §6).
    #[error("Unauthorized write: {0}")]
    Unauthorized(String),
}

/// Outcome of rebuilding the in-memory vector index from persisted embeddings.
///
/// The HNSW index lives in memory (see [`vector_index`]), so it is reconstructed
/// from the `claims.embedding_vector` column each time a store is opened. This
/// report says how that reconstruction went, which distinguishes the two
/// remedies: `missing` claims need [`SqliteStore::backfill_embeddings`], while
/// `unusable` ones need a full [`SqliteStore::reindex_all`] (ADR-014).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexLoadReport {
    /// Embeddings successfully loaded into the vector index.
    pub loaded: usize,

    /// Claims with no persisted embedding yet (written before embeddings were
    /// persisted, or stored while the embedder was unavailable).
    pub missing: usize,

    /// Claims whose persisted embedding could not be indexed because it was
    /// corrupt or did not match the current model's dimension.
    pub unusable: usize,
}

impl IndexLoadReport {
    /// Whether any claim is absent from the vector index and so unsearchable.
    pub fn has_gaps(&self) -> bool {
        self.missing > 0 || self.unusable > 0
    }
}

/// SQLite-based implementation of ClaimStore
///
/// This store provides persistent storage for claims, relationships, and provenance.
/// It uses SQLite for structured data and HNSW for vector search.
///
/// # Thread Safety
///
/// SQLite connections are not thread-safe. Each thread should have its own SqliteStore instance.
pub struct SqliteStore {
    conn: Connection,
    vector_index: Option<VectorIndex>,
    embedding_model: Option<Box<dyn EmbeddingModel + Send + Sync>>,
    /// How the vector index was reconstructed when this store was opened.
    index_load: IndexLoadReport,
}

impl SqliteStore {
    /// Create a new SqliteStore with the given database path
    ///
    /// Use `:memory:` for an in-memory database (useful for testing).
    ///
    /// # Parameters
    ///
    /// - `path`: Path to the SQLite database file
    /// - `enable_vector_search`: If true, enables semantic search via HNSW index
    /// - `embedding_dimension`: Dimension of embedding vectors (e.g., 384)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use boswell_store::SqliteStore;
    ///
    /// // Without vector search
    /// let store = SqliteStore::new("boswell.db", false, 0).unwrap();
    ///
    /// // With vector search
    /// let store = SqliteStore::new("boswell.db", true, 384).unwrap();
    /// ```
    pub fn new<P: AsRef<Path>>(
        path: P,
        enable_vector_search: bool,
        embedding_dimension: usize,
    ) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;

        let (vector_index, embedding_model) = if enable_vector_search {
            (
                Some(VectorIndex::new(embedding_dimension)),
                Some(Box::new(MockEmbeddingModel::new(embedding_dimension))
                    as Box<dyn EmbeddingModel + Send + Sync>),
            )
        } else {
            (None, None)
        };

        let mut store = Self {
            conn,
            vector_index,
            embedding_model,
            index_load: IndexLoadReport::default(),
        };
        store.initialize_schema()?;
        store.index_load = store.load_vector_index()?;
        Ok(store)
    }

    /// Create a store with a caller-supplied embedding model.
    ///
    /// Vector search is enabled and sized to the model's own dimension, so this
    /// is the constructor to use with a real embedder such as
    /// [`OllamaEmbeddingModel`](crate::OllamaEmbeddingModel):
    ///
    /// ```no_run
    /// use boswell_store::{SqliteStore, OllamaEmbeddingModel};
    ///
    /// let embedder = OllamaEmbeddingModel::default_local("embeddinggemma").unwrap();
    /// let store = SqliteStore::with_embedding_model("boswell.db", Box::new(embedder)).unwrap();
    /// ```
    pub fn with_embedding_model<P: AsRef<Path>>(
        path: P,
        embedding_model: Box<dyn EmbeddingModel + Send + Sync>,
    ) -> Result<Self, StoreError> {
        let dimension = embedding_model.dimension();
        let conn = Connection::open(path)?;

        let mut store = Self {
            conn,
            vector_index: Some(VectorIndex::new(dimension)),
            embedding_model: Some(embedding_model),
            index_load: IndexLoadReport::default(),
        };
        store.initialize_schema()?;
        store.index_load = store.load_vector_index()?;
        Ok(store)
    }

    /// Initialize the database schema
    fn initialize_schema(&mut self) -> Result<(), StoreError> {
        // Read and execute the schema SQL
        let schema = include_str!("schema.sql");

        // Execute each statement (SQLite doesn't support multiple statements in one execute)
        self.conn.execute_batch(schema)?;

        // Apply incremental migrations for databases created by earlier versions.
        self.run_migrations()?;

        Ok(())
    }

    /// Apply incremental schema migrations to databases that predate newer columns.
    ///
    /// This is idempotent: on a freshly created database the columns already
    /// exist (via `schema.sql`), so the `ALTER TABLE` steps are skipped.
    fn run_migrations(&mut self) -> Result<(), StoreError> {
        // Migration: add `source_type` to the claims table if it is missing.
        if !self.column_exists("claims", "source_type")? {
            self.conn.execute_batch(
                "ALTER TABLE claims ADD COLUMN source_type TEXT NOT NULL DEFAULT 'assertion';",
            )?;
        }
        // Index is safe to (re)create now that the column is guaranteed to exist.
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_claims_source_type ON claims(source_type);",
        )?;

        // Migration: add `dev_provider` to provenance_stamps if it is missing
        // (procedural memory Phase 4). The table itself is created by schema.sql;
        // this only matters for a database created by the Phase 3 schema.
        if self.table_exists("provenance_stamps")?
            && !self.column_exists("provenance_stamps", "dev_provider")?
        {
            self.conn.execute_batch(
                "ALTER TABLE provenance_stamps ADD COLUMN dev_provider INTEGER NOT NULL DEFAULT 0;",
            )?;
        }

        // Migration: add `unknown_count` to procedures if it is missing
        // (procedural memory Phase 5, "silence is not success").
        if self.table_exists("procedures")? && !self.column_exists("procedures", "unknown_count")? {
            self.conn.execute_batch(
                "ALTER TABLE procedures ADD COLUMN unknown_count INTEGER NOT NULL DEFAULT 0;",
            )?;
        }

        // Migration: reverse adjacency index on goal_edges (procedural memory
        // Phase 8, graph integrity). Collection asks "who points at this node?",
        // which is a full scan without it.
        if self.table_exists("goal_edges")? {
            self.conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_goal_edges_child \
                 ON goal_edges(child_kind, child_id);",
            )?;
        }

        Ok(())
    }

    /// Check whether `table` exists in the SQLite catalog.
    fn table_exists(&self, table: &str) -> Result<bool, StoreError> {
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Check whether `column` exists on `table` via `PRAGMA table_info`.
    fn column_exists(&self, table: &str, column: &str) -> Result<bool, StoreError> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info({})", table))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            // Column 1 of table_info is the column name.
            let name: String = row.get(1)?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Build the `WHERE` tail shared by [`ClaimStore::query_claims`] and
    /// [`ClaimStore::count_claims`], returning it alongside its bound
    /// parameters.
    ///
    /// Both callers open with `WHERE 1=1`, so every clause here appends. Split
    /// out so the two cannot drift: a filter the count ignores is a count that
    /// disagrees with the query it claims to describe.
    ///
    /// `ClaimQuery::semantic_text` is not a filter here — semantic search is
    /// [`ClaimStore::semantic_search`], a different path.
    fn claim_filter_sql(query: &ClaimQuery) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
        let mut sql = String::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(namespace) = &query.namespace {
            sql.push_str(" AND namespace LIKE ?");
            params.push(Box::new(format!("{}%", namespace)));
        }

        // Exact, case-sensitive triple match. The columns carry no COLLATE, so
        // SQLite's `=` compares bytes — the same semantics as Rust's `==` on the
        // strings this replaced.
        if let Some(subject) = &query.subject {
            sql.push_str(" AND subject = ?");
            params.push(Box::new(subject.clone()));
        }

        if let Some(predicate) = &query.predicate {
            sql.push_str(" AND predicate = ?");
            params.push(Box::new(predicate.clone()));
        }

        if let Some(object) = &query.object {
            sql.push_str(" AND object = ?");
            params.push(Box::new(object.clone()));
        }

        if let Some(tier) = &query.tier {
            sql.push_str(" AND tier = ?");
            params.push(Box::new(tier.clone()));
        }

        if let Some(source_type) = &query.source_type {
            sql.push_str(" AND source_type = ?");
            params.push(Box::new(source_type.clone()));
        }

        if let Some(min_conf) = query.min_confidence {
            sql.push_str(" AND base_lower >= ?");
            params.push(Box::new(min_conf));
        }

        if let Some(limit) = query.limit {
            sql.push_str(" LIMIT ?");
            params.push(Box::new(limit));
        }

        (sql, params)
    }

    /// Convert ClaimId to bytes for storage
    fn claim_id_to_bytes(id: ClaimId) -> Vec<u8> {
        id.value().to_be_bytes().to_vec()
    }

    /// Convert bytes to ClaimId
    fn bytes_to_claim_id(bytes: &[u8]) -> Result<ClaimId, StoreError> {
        if bytes.len() != 16 {
            return Err(StoreError::InvalidData(format!(
                "Expected 16 bytes for ClaimId, got {}",
                bytes.len()
            )));
        }
        let mut arr = [0u8; 16];
        arr.copy_from_slice(bytes);
        Ok(ClaimId::from_value(u128::from_be_bytes(arr)))
    }

    /// The text a claim is embedded as, shared by the write path and any
    /// re-embedding pass so a rebuilt vector is identical to the original.
    fn embedding_text(subject: &str, predicate: &str, object: &str) -> String {
        format!("{} {} {}", subject, predicate, object)
    }

    /// Encode an embedding for storage in `claims.embedding_vector`.
    ///
    /// Stored as a JSON array, matching the column's documented format in
    /// `schema.sql` ("stored as JSON array for flexibility").
    fn encode_embedding(embedding: &[f32]) -> Result<String, StoreError> {
        serde_json::to_string(embedding)
            .map_err(|e| StoreError::InvalidData(format!("Failed to encode embedding: {}", e)))
    }

    /// Decode an embedding written by [`Self::encode_embedding`].
    fn decode_embedding(raw: &str) -> Result<Vec<f32>, StoreError> {
        serde_json::from_str(raw)
            .map_err(|e| StoreError::InvalidData(format!("Failed to decode embedding: {}", e)))
    }

    /// Convert RelationshipType to string for storage
    fn relationship_type_to_str(rt: RelationshipType) -> &'static str {
        match rt {
            RelationshipType::Supports => "supports",
            RelationshipType::Contradicts => "contradicts",
            RelationshipType::DerivedFrom => "derived_from",
            RelationshipType::References => "references",
            RelationshipType::Supersedes => "supersedes",
        }
    }

    /// Convert string to RelationshipType
    fn str_to_relationship_type(s: &str) -> Result<RelationshipType, StoreError> {
        match s {
            "supports" => Ok(RelationshipType::Supports),
            "contradicts" => Ok(RelationshipType::Contradicts),
            "derived_from" => Ok(RelationshipType::DerivedFrom),
            "references" => Ok(RelationshipType::References),
            "supersedes" => Ok(RelationshipType::Supersedes),
            _ => Err(StoreError::InvalidData(format!(
                "Unknown relationship type: {}",
                s
            ))),
        }
    }
}

impl ClaimStore for SqliteStore {
    type Error = StoreError;

    fn assert_claim(&mut self, claim: Claim) -> Result<ClaimId, Self::Error> {
        let id_bytes = Self::claim_id_to_bytes(claim.id);

        // Check if the ID already exists
        let exists: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM claims WHERE id = ?1",
                params![&id_bytes],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);

        if exists {
            return Err(StoreError::Duplicate);
        }

        // Embed before the insert so the vector is written in the same row as
        // the claim. Persisting it is what lets semantic search survive a
        // restart: the HNSW index is in-memory, and `load_vector_index` rebuilds
        // it from this column when the store is reopened.
        let embedding = match (&self.embedding_model, &self.vector_index) {
            (Some(embedding_model), Some(_)) => {
                let text = Self::embedding_text(&claim.subject, &claim.predicate, &claim.object);
                match embedding_model.embed(&text) {
                    Ok(embedding) => Some(embedding),
                    Err(e) => {
                        // Don't fail the write: the claim is still worth storing,
                        // and `backfill_embeddings` can embed it once the model
                        // is reachable again.
                        eprintln!("Warning: Failed to generate embedding: {}", e);
                        None
                    }
                }
            }
            _ => None,
        };
        let encoded_embedding = embedding
            .as_deref()
            .map(Self::encode_embedding)
            .transpose()?;

        // Insert the claim
        self.conn.execute(
            "INSERT INTO claims (id, namespace, subject, predicate, object, source_type, base_lower, base_upper, tier, created_at, stale_at, embedding_vector)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                &id_bytes,
                &claim.namespace,
                &claim.subject,
                &claim.predicate,
                &claim.object,
                &claim.source_type,
                claim.confidence.0,
                claim.confidence.1,
                &claim.tier,
                claim.created_at as i64,
                claim.stale_at.map(|t| t as i64),
                encoded_embedding,
            ],
        )?;

        if let (Some(vector_index), Some(embedding)) = (&self.vector_index, &embedding) {
            // An index insert failure leaves the claim stored and its embedding
            // persisted, so the next open reconstructs it.
            let _ = vector_index.add(claim.id, embedding);
        }

        Ok(claim.id)
    }

    fn get_claim(&self, id: ClaimId) -> Result<Option<Claim>, Self::Error> {
        let id_bytes = Self::claim_id_to_bytes(id);

        let claim = self.conn.query_row(
            "SELECT id, namespace, subject, predicate, object, base_lower, base_upper, tier, created_at, stale_at, source_type
             FROM claims WHERE id = ?1",
            params![&id_bytes],
            |row| {
                let id_bytes: Vec<u8> = row.get(0)?;
                let id = Self::bytes_to_claim_id(&id_bytes)
                    .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
                        0, rusqlite::types::Type::Blob, Box::new(e)
                    ))?;

                let stale_at: Option<i64> = row.get(9)?;

                Ok(Claim {
                    id,
                    namespace: row.get(1)?,
                    subject: row.get(2)?,
                    predicate: row.get(3)?,
                    object: row.get(4)?,
                    source_type: row.get(10)?,
                    confidence: (row.get(5)?, row.get(6)?),
                    tier: row.get(7)?,
                    created_at: row.get::<_, i64>(8)? as u64,
                    stale_at: stale_at.map(|t| t as u64),
                })
            }
        ).optional()?;

        Ok(claim)
    }

    fn query_claims(&self, query: &ClaimQuery) -> Result<Vec<Claim>, Self::Error> {
        let (tail, params) = Self::claim_filter_sql(query);
        let sql = format!(
            "SELECT id, namespace, subject, predicate, object, base_lower, base_upper, tier, created_at, stale_at, source_type
             FROM claims WHERE 1=1{}",
            tail
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let claims = stmt
            .query_map(&param_refs[..], |row| {
                let id_bytes: Vec<u8> = row.get(0)?;
                let id = Self::bytes_to_claim_id(&id_bytes).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        Box::new(e),
                    )
                })?;

                let stale_at: Option<i64> = row.get(9)?;

                Ok(Claim {
                    id,
                    namespace: row.get(1)?,
                    subject: row.get(2)?,
                    predicate: row.get(3)?,
                    object: row.get(4)?,
                    source_type: row.get(10)?,
                    confidence: (row.get(5)?, row.get(6)?),
                    tier: row.get(7)?,
                    created_at: row.get::<_, i64>(8)? as u64,
                    stale_at: stale_at.map(|t| t as u64),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(claims)
    }

    /// `SELECT COUNT(*)` over the same filter [`SqliteStore::query_claims`]
    /// builds, so nothing is decoded into a [`Claim`] to be counted and thrown
    /// away.
    ///
    /// The limit is applied inside the subquery rather than dropped, because the
    /// trait promises this equals what `query_claims` would return.
    fn count_claims(&self, query: &ClaimQuery) -> Result<u64, Self::Error> {
        let (tail, params) = Self::claim_filter_sql(query);
        let sql = format!(
            "SELECT COUNT(*) FROM (SELECT 1 FROM claims WHERE 1=1{})",
            tail
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let count: i64 = stmt.query_row(&param_refs[..], |row| row.get(0))?;

        Ok(count.max(0) as u64)
    }

    fn add_relationship(&mut self, relationship: Relationship) -> Result<(), Self::Error> {
        let from_bytes = Self::claim_id_to_bytes(relationship.from_claim);
        let to_bytes = Self::claim_id_to_bytes(relationship.to_claim);
        let rel_type = Self::relationship_type_to_str(relationship.relationship_type);

        self.conn.execute(
            "INSERT INTO relationships (from_claim_id, to_claim_id, relationship_type, strength, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(from_claim_id, to_claim_id, relationship_type) DO UPDATE SET
             strength = excluded.strength, created_at = excluded.created_at",
            params![
                &from_bytes,
                &to_bytes,
                rel_type,
                relationship.strength,
                relationship.created_at as i64,
            ],
        )?;

        Ok(())
    }

    fn get_relationships(&self, id: ClaimId) -> Result<Vec<Relationship>, Self::Error> {
        let id_bytes = Self::claim_id_to_bytes(id);

        let mut stmt = self.conn.prepare(
            "SELECT from_claim_id, to_claim_id, relationship_type, strength, created_at
             FROM relationships WHERE from_claim_id = ?1 OR to_claim_id = ?1",
        )?;

        let relationships = stmt
            .query_map(params![&id_bytes], |row| {
                let from_bytes: Vec<u8> = row.get(0)?;
                let to_bytes: Vec<u8> = row.get(1)?;
                let rel_type_str: String = row.get(2)?;

                let from_claim = Self::bytes_to_claim_id(&from_bytes).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        Box::new(e),
                    )
                })?;

                let to_claim = Self::bytes_to_claim_id(&to_bytes).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Blob,
                        Box::new(e),
                    )
                })?;

                let relationship_type =
                    Self::str_to_relationship_type(&rel_type_str).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

                Ok(Relationship {
                    from_claim,
                    to_claim,
                    relationship_type,
                    strength: row.get(3)?,
                    created_at: row.get::<_, i64>(4)? as u64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(relationships)
    }

    fn semantic_search(
        &self,
        query_text: &str,
        limit: usize,
        min_similarity: f32,
    ) -> Result<Vec<(Claim, f32)>, StoreError> {
        // Embed the query text, then search the vector index.
        let embedding_model = self.embedding_model.as_ref().ok_or_else(|| {
            StoreError::InvalidData("Vector search is not enabled for this store".to_string())
        })?;

        let embedding = embedding_model
            .embed(query_text)
            .map_err(|e| StoreError::InvalidData(format!("Failed to embed query: {}", e)))?;

        self.semantic_search_by_embedding(&embedding, limit, DEFAULT_EF_SEARCH, min_similarity)
    }

    fn supports_semantic_search(&self) -> bool {
        self.vector_index.is_some() && self.embedding_model.is_some()
    }

    fn delete_claim(&mut self, id: ClaimId) -> Result<bool, Self::Error> {
        let id_bytes = Self::claim_id_to_bytes(id);
        // ON DELETE CASCADE removes relationships, provenance, and confidence_cache.
        let affected = self
            .conn
            .execute("DELETE FROM claims WHERE id = ?1", params![&id_bytes])?;
        // Note: the vector index has no per-id removal; an orphaned vector is
        // harmless because semantic_search filters hits whose claim is gone.
        Ok(affected > 0)
    }

    fn update_claim_tier(&mut self, id: ClaimId, new_tier: &str) -> Result<bool, Self::Error> {
        let id_bytes = Self::claim_id_to_bytes(id);
        let affected = self.conn.execute(
            "UPDATE claims SET tier = ?1 WHERE id = ?2",
            params![new_tier, &id_bytes],
        )?;
        if affected > 0 {
            // Tier changes the decay rate, so drop any cached effective confidence.
            self.conn.execute(
                "DELETE FROM confidence_cache WHERE claim_id = ?1",
                params![&id_bytes],
            )?;
        }
        Ok(affected > 0)
    }
}

/// Default HNSW `ef_search` quality parameter for trait-level semantic search.
const DEFAULT_EF_SEARCH: usize = 64;

/// How long a cached effective-confidence entry is trusted before
/// [`SqliteStore::get_effective_confidence`] recomputes it on demand (seconds).
const CONFIDENCE_CACHE_FRESHNESS_SECS: u64 = 300;

impl SqliteStore {
    /// Perform semantic search for claims similar to the given embedding
    ///
    /// Returns claims ordered by cosine similarity (descending).
    ///
    /// # Parameters
    ///
    /// - `query_embedding`: The query vector to search for
    /// - `k`: Number of results to return
    /// - `ef_search`: HNSW search quality parameter (higher = better but slower)
    /// - `min_similarity`: Minimum cosine similarity threshold (0.0 to 1.0)
    ///
    /// # Returns
    ///
    /// Vec of (Claim, similarity_score) pairs, sorted by similarity descending
    ///
    /// # Errors
    ///
    /// Returns error if vector search is not enabled or if search fails
    pub fn semantic_search_by_embedding(
        &self,
        query_embedding: &[f32],
        k: usize,
        ef_search: usize,
        min_similarity: f32,
    ) -> Result<Vec<(Claim, f32)>, StoreError> {
        let vector_index = self.vector_index.as_ref().ok_or_else(|| {
            StoreError::InvalidData("Vector search is not enabled for this store".to_string())
        })?;

        // Search the vector index for similar claim IDs
        let similar_ids = vector_index
            .search(query_embedding, k, ef_search)
            .map_err(|e| StoreError::InvalidData(format!("Vector search failed: {}", e)))?;

        // Filter by minimum similarity and fetch full claims
        let mut results = Vec::new();

        for (claim_id, similarity) in similar_ids {
            if similarity < min_similarity {
                continue;
            }

            if let Some(claim) = self.get_claim(claim_id)? {
                results.push((claim, similarity));
            }
        }

        Ok(results)
    }

    /// Report describing how the vector index was rebuilt when this store was
    /// opened. See [`IndexLoadReport`].
    pub fn index_load_report(&self) -> &IndexLoadReport {
        &self.index_load
    }

    /// Number of vectors currently in the in-memory index.
    pub fn vector_index_len(&self) -> usize {
        self.vector_index.as_ref().map_or(0, |index| index.len())
    }

    /// Rebuild the in-memory vector index from embeddings persisted in SQLite.
    ///
    /// The HNSW index is not itself durable, so this is what makes semantic
    /// search survive a restart. It is called automatically when a store is
    /// opened and performs no embedding work — it only replays vectors already
    /// stored in `claims.embedding_vector`.
    ///
    /// A row that cannot be replayed (corrupt JSON, or a dimension that does not
    /// match the current model) is counted as `unusable` rather than failing the
    /// load, so one bad row can never stop the instance from starting.
    pub fn load_vector_index(&self) -> Result<IndexLoadReport, StoreError> {
        let Some(vector_index) = self.vector_index.as_ref() else {
            return Ok(IndexLoadReport::default());
        };

        let mut stmt = self
            .conn
            .prepare("SELECT id, embedding_vector FROM claims")?;
        let rows = stmt
            .query_map([], |row| {
                let id_bytes: Vec<u8> = row.get(0)?;
                let stored: Option<String> = row.get(1)?;
                Ok((id_bytes, stored))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut report = IndexLoadReport::default();
        for (id_bytes, stored) in rows {
            let Some(stored) = stored else {
                report.missing += 1;
                continue;
            };
            let claim_id = Self::bytes_to_claim_id(&id_bytes)?;
            match Self::decode_embedding(&stored) {
                Ok(embedding) => match vector_index.add(claim_id, &embedding) {
                    Ok(()) => report.loaded += 1,
                    // Wrong dimension: the embedding model changed under an
                    // existing store. Recoverable with `reindex_all`.
                    Err(VectorIndexError::DimensionMismatch { .. }) => report.unusable += 1,
                    Err(e) => {
                        return Err(StoreError::InvalidData(format!(
                            "Failed to rebuild vector index: {}",
                            e
                        )))
                    }
                },
                Err(_) => report.unusable += 1,
            }
        }

        Ok(report)
    }

    /// Embed and persist every claim that has no stored embedding, adding each
    /// to the vector index.
    ///
    /// This is the one-time catch-up for claims written before embeddings were
    /// persisted, and the retry path for claims stored while the embedder was
    /// unreachable. It is idempotent and a no-op once every claim is embedded,
    /// so it is safe to run on every startup.
    ///
    /// Returns the number of claims embedded. Each claim is persisted as it is
    /// embedded, so an interrupted run resumes where it left off.
    pub fn backfill_embeddings(&self) -> Result<usize, StoreError> {
        let (Some(embedding_model), Some(vector_index)) =
            (&self.embedding_model, &self.vector_index)
        else {
            return Ok(0);
        };

        // Collect first: the rows are updated on the same connection below.
        let mut stmt = self.conn.prepare(
            "SELECT id, subject, predicate, object FROM claims WHERE embedding_vector IS NULL",
        )?;
        let pending = stmt
            .query_map([], |row| {
                let id_bytes: Vec<u8> = row.get(0)?;
                Ok((
                    id_bytes,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut embedded = 0usize;
        for (id_bytes, subject, predicate, object) in pending {
            let text = Self::embedding_text(&subject, &predicate, &object);
            let embedding = embedding_model.embed(&text).map_err(|e| {
                StoreError::InvalidData(format!("Failed to embed claim during backfill: {}", e))
            })?;

            let encoded = Self::encode_embedding(&embedding)?;
            self.conn.execute(
                "UPDATE claims SET embedding_vector = ?1 WHERE id = ?2",
                params![encoded, &id_bytes],
            )?;

            let claim_id = Self::bytes_to_claim_id(&id_bytes)?;
            let _ = vector_index.add(claim_id, &embedding);
            embedded += 1;
        }

        Ok(embedded)
    }

    /// Re-embed every claim from scratch and rebuild the vector index.
    ///
    /// This is the recovery path ADR-014 describes for a deliberate embedding
    /// model change or a corrupt index: discard every stored vector, then embed
    /// all claims again with the store's current model. ADR-014 specifies this
    /// as an offline, dead-stop operation — run it with the instance down.
    ///
    /// Returns the number of claims re-embedded.
    pub fn reindex_all(&self) -> Result<usize, StoreError> {
        let Some(vector_index) = self.vector_index.as_ref() else {
            return Ok(0);
        };

        vector_index.clear();
        self.conn
            .execute("UPDATE claims SET embedding_vector = NULL", [])?;
        self.backfill_embeddings()
    }

    /// Add an embedding to the vector index for an existing claim                ///
    /// This is a helper method for when embeddings are generated after claim creation.
    ///
    /// # Parameters
    ///
    /// - `claim_id`: ID of the claim
    /// - `embedding`: The embedding vector
    ///
    /// # Errors
    ///
    /// Returns error if vector search is not enabled or if the claim doesn't exist
    pub fn add_embedding(&self, claim_id: ClaimId, embedding: &[f32]) -> Result<(), StoreError> {
        let vector_index = self.vector_index.as_ref().ok_or_else(|| {
            StoreError::InvalidData("Vector search is not enabled for this store".to_string())
        })?;

        // Verify the claim exists
        if self.get_claim(claim_id)?.is_none() {
            return Err(StoreError::NotFound(claim_id.to_string()));
        }

        // Persist alongside the index entry, so the vector is replayed on the
        // next open rather than being lost with the in-memory index.
        let encoded = Self::encode_embedding(embedding)?;
        self.conn.execute(
            "UPDATE claims SET embedding_vector = ?1 WHERE id = ?2",
            params![encoded, &Self::claim_id_to_bytes(claim_id)],
        )?;

        // Add to vector index
        vector_index
            .add(claim_id, embedding)
            .map_err(|e| StoreError::InvalidData(format!("Failed to add embedding: {}", e)))?;

        Ok(())
    }

    /// Recompute and persist age-decayed effective confidence for every claim
    /// into the `confidence_cache` table (per ADR-007).
    ///
    /// This is the "bake" pass the Janitor drives on a schedule: it reads each
    /// claim's base confidence and writes the decayed value keyed by claim id.
    /// `now` is a Unix timestamp in seconds. Returns the number of claims cached.
    pub fn recompute_confidence_cache(
        &self,
        config: &DecayConfig,
        now: u64,
    ) -> Result<usize, StoreError> {
        let claims = self.query_claims(&ClaimQuery::default())?;

        let tx = self.conn.unchecked_transaction()?;
        for claim in &claims {
            let (lower, upper) =
                decayed_confidence(claim.confidence, &claim.tier, claim.created_at, now, config);
            let id_bytes = Self::claim_id_to_bytes(claim.id);
            tx.execute(
                "INSERT INTO confidence_cache
                    (claim_id, effective_lower, effective_upper, computed_at, version)
                 VALUES (?1, ?2, ?3, ?4, 0)
                 ON CONFLICT(claim_id) DO UPDATE SET
                    effective_lower = excluded.effective_lower,
                    effective_upper = excluded.effective_upper,
                    computed_at = excluded.computed_at,
                    version = confidence_cache.version + 1",
                params![&id_bytes, lower, upper, now as i64],
            )?;
        }
        tx.commit()?;

        Ok(claims.len())
    }

    /// Get a claim's effective (age-decayed) confidence, using the cache when it
    /// is fresh and computing on demand otherwise (per ADR-007).
    ///
    /// On a cache miss or a stale entry (older than
    /// [`CONFIDENCE_CACHE_FRESHNESS_SECS`]), the value is recomputed from the
    /// claim's base confidence and written back. Returns `None` if the claim does
    /// not exist. `now` is a Unix timestamp in seconds.
    pub fn get_effective_confidence(
        &self,
        claim_id: ClaimId,
        config: &DecayConfig,
        now: u64,
    ) -> Result<Option<(f64, f64)>, StoreError> {
        let id_bytes = Self::claim_id_to_bytes(claim_id);

        let cached: Option<(f64, f64, i64)> = self
            .conn
            .query_row(
                "SELECT effective_lower, effective_upper, computed_at
                 FROM confidence_cache WHERE claim_id = ?1",
                params![&id_bytes],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;

        if let Some((lower, upper, computed_at)) = cached {
            let fresh = now.saturating_sub(computed_at as u64) <= CONFIDENCE_CACHE_FRESHNESS_SECS;
            if fresh {
                return Ok(Some((lower, upper)));
            }
        }

        // Miss or stale: recompute from the claim's base confidence and cache it.
        let Some(claim) = self.get_claim(claim_id)? else {
            return Ok(None);
        };
        let effective =
            decayed_confidence(claim.confidence, &claim.tier, claim.created_at, now, config);
        self.conn.execute(
            "INSERT INTO confidence_cache
                (claim_id, effective_lower, effective_upper, computed_at, version)
             VALUES (?1, ?2, ?3, ?4, 0)
             ON CONFLICT(claim_id) DO UPDATE SET
                effective_lower = excluded.effective_lower,
                effective_upper = excluded.effective_upper,
                computed_at = excluded.computed_at,
                version = confidence_cache.version + 1",
            params![&id_bytes, effective.0, effective.1, now as i64],
        )?;

        Ok(Some(effective))
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;

    /// A database created before `source_type` existed must gain the column on
    /// open (existing rows defaulting to "assertion") and remain fully usable.
    #[test]
    fn test_source_type_migration_on_legacy_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");

        // 1. Build a legacy `claims` table WITHOUT the source_type column and
        //    insert one row through a raw connection.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE claims (
                    id BLOB PRIMARY KEY NOT NULL,
                    namespace TEXT NOT NULL,
                    subject TEXT NOT NULL,
                    predicate TEXT NOT NULL,
                    object TEXT NOT NULL,
                    base_lower REAL NOT NULL,
                    base_upper REAL NOT NULL,
                    tier TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    stale_at INTEGER,
                    embedding_vector TEXT,
                    content_hash TEXT
                );",
            )
            .unwrap();

            let id_bytes = SqliteStore::claim_id_to_bytes(ClaimId::from_value(42));
            conn.execute(
                "INSERT INTO claims (id, namespace, subject, predicate, object, base_lower, base_upper, tier, created_at, stale_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![&id_bytes, "legacy", "user:alice", "knows", "user:bob", 0.7, 0.8, "task", 1000_i64, Option::<i64>::None],
            )
            .unwrap();
        }

        // 2. Opening through SqliteStore triggers run_migrations().
        let mut store = SqliteStore::new(&path, false, 0).unwrap();
        assert!(store.column_exists("claims", "source_type").unwrap());

        // 3. The pre-existing row now carries the default source_type.
        let legacy = store
            .get_claim(ClaimId::from_value(42))
            .unwrap()
            .expect("legacy claim should still be present");
        assert_eq!(legacy.source_type, "assertion");
        assert_eq!(legacy.subject, "user:alice");

        // 4. New inserts with an explicit source_type round-trip correctly.
        let new = Claim {
            id: ClaimId::from_value(99),
            namespace: "legacy".to_string(),
            subject: "team:atlas".to_string(),
            predicate: "trend:focus".to_string(),
            object: "topic:auth".to_string(),
            source_type: "inference".to_string(),
            confidence: (0.5, 0.7),
            tier: "task".to_string(),
            created_at: 2000,
            stale_at: None,
        };
        store.assert_claim(new).unwrap();
        let back = store.get_claim(ClaimId::from_value(99)).unwrap().unwrap();
        assert_eq!(back.source_type, "inference");

        // Re-running migrations is idempotent.
        store.run_migrations().unwrap();
        assert!(store.column_exists("claims", "source_type").unwrap());
    }

    /// A file-based database must be reopenable: `initialize_schema()` runs the
    /// full `schema.sql` on every `SqliteStore::new`, so the schema_info seed
    /// insert has to be idempotent (`INSERT OR IGNORE`). Opening the same path
    /// twice must both succeed and preserve prior data.
    #[test]
    fn test_reopen_file_based_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boswell.db");

        // First open creates the schema and seeds schema_info(version = 1).
        {
            let mut store = SqliteStore::new(&path, false, 0).unwrap();
            store
                .assert_claim(Claim::new(
                    ClaimId::from_value(7),
                    "ns".into(),
                    "s:1".into(),
                    "p:1".into(),
                    "o:1".into(),
                    (0.4, 0.6),
                    "task".into(),
                    100,
                ))
                .unwrap();
        }

        // Second open re-runs schema.sql; without OR IGNORE this fails with
        // "UNIQUE constraint failed: schema_info.version".
        let store = SqliteStore::new(&path, false, 0).unwrap();
        let back = store
            .get_claim(ClaimId::from_value(7))
            .unwrap()
            .expect("claim from the first session should survive reopen");
        assert_eq!(back.subject, "s:1");
    }

    /// source_type persists through assert/get, and Claim::new defaults it.
    #[test]
    fn test_source_type_roundtrip_and_default() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

        let c = Claim::new(
            ClaimId::new(),
            "ns".into(),
            "s:1".into(),
            "p:1".into(),
            "o:1".into(),
            (0.4, 0.6),
            "task".into(),
            100,
        )
        .with_source_type("extraction");
        let id = store.assert_claim(c).unwrap();
        assert_eq!(
            store.get_claim(id).unwrap().unwrap().source_type,
            "extraction"
        );

        // Claim::new defaults to "assertion".
        let d = Claim::new(
            ClaimId::new(),
            "ns".into(),
            "s:2".into(),
            "p:2".into(),
            "o:2".into(),
            (0.4, 0.6),
            "task".into(),
            100,
        );
        assert_eq!(d.source_type, "assertion");
    }

    /// `query_claims` filters on `source_type` when the query specifies one.
    #[test]
    fn test_query_filter_by_source_type() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

        store
            .assert_claim(
                Claim::new(
                    ClaimId::new(),
                    "ns".into(),
                    "s:a".into(),
                    "p".into(),
                    "o".into(),
                    (0.5, 0.6),
                    "task".into(),
                    100,
                )
                .with_source_type("assertion"),
            )
            .unwrap();
        store
            .assert_claim(
                Claim::new(
                    ClaimId::new(),
                    "ns".into(),
                    "s:b".into(),
                    "p".into(),
                    "o".into(),
                    (0.5, 0.6),
                    "task".into(),
                    100,
                )
                .with_source_type("extraction"),
            )
            .unwrap();

        let extraction_only = ClaimQuery {
            source_type: Some("extraction".to_string()),
            ..ClaimQuery::default()
        };
        let hits = store.query_claims(&extraction_only).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_type, "extraction");
        assert_eq!(hits[0].subject, "s:b");

        // No source_type filter returns both.
        let all = store.query_claims(&ClaimQuery::default()).unwrap();
        assert_eq!(all.len(), 2);
    }
}

#[cfg(test)]
mod embedding_persistence_tests {
    use super::*;
    use boswell_domain::{Claim, ClaimId};

    fn claim(subject: &str, object: &str) -> Claim {
        Claim::new(
            ClaimId::new(),
            "person".into(),
            subject.into(),
            "rel:uses".into(),
            object.into(),
            (0.8, 0.9),
            "project".into(),
            1_700_000_000,
        )
    }

    /// The regression this whole feature exists for: semantic search must still
    /// work after the process restarts. Claims survived before, but the HNSW
    /// index was in-memory only and nothing rebuilt it, so search went silent.
    #[test]
    fn test_semantic_search_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("persist.db");

        let target = {
            let mut store = SqliteStore::new(&path, true, 64).unwrap();
            let c = claim("person:jd", "lang:rust");
            let id = store.assert_claim(c).unwrap();
            // Sanity: searchable in the session that wrote it.
            assert!(store.vector_index_len() > 0);
            id
        };

        // Reopen: a fresh store, a fresh (empty) HNSW index.
        let store = SqliteStore::new(&path, true, 64).unwrap();
        let report = store.index_load_report();
        assert_eq!(report.loaded, 1, "embedding should be replayed on open");
        assert_eq!(report.missing, 0);
        assert_eq!(report.unusable, 0);
        assert_eq!(store.vector_index_len(), 1);

        let hits = store
            .semantic_search("person:jd rel:uses lang:rust", 5, 0.0)
            .unwrap();
        assert!(
            hits.iter().any(|(c, _)| c.id == target),
            "the claim must be findable by semantic search after reopen"
        );
    }

    /// Embeddings are written to the column the schema reserves for them.
    #[test]
    fn test_assert_persists_embedding_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("column.db");
        let mut store = SqliteStore::new(&path, true, 64).unwrap();
        store.assert_claim(claim("person:jd", "lang:rust")).unwrap();

        let stored: Option<String> = store
            .conn
            .query_row("SELECT embedding_vector FROM claims", [], |r| r.get(0))
            .unwrap();
        let vector = SqliteStore::decode_embedding(&stored.expect("embedding persisted")).unwrap();
        assert_eq!(vector.len(), 64);
    }

    /// A store opened without vector search must not fail, and must not claim
    /// to have loaded anything.
    #[test]
    fn test_load_is_noop_without_vector_search() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("novec.db");
        let mut store = SqliteStore::new(&path, false, 0).unwrap();
        store.assert_claim(claim("person:jd", "lang:rust")).unwrap();
        assert_eq!(store.index_load_report(), &IndexLoadReport::default());
        assert_eq!(store.vector_index_len(), 0);
    }

    /// Claims written before embeddings were persisted (embedding_vector NULL)
    /// are reported as `missing` and made searchable by a backfill.
    #[test]
    fn test_backfill_embeds_legacy_claims() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");

        // Write a claim, then blank its embedding to look like a pre-upgrade row.
        {
            let mut store = SqliteStore::new(&path, true, 64).unwrap();
            store.assert_claim(claim("person:jd", "lang:rust")).unwrap();
            store
                .conn
                .execute("UPDATE claims SET embedding_vector = NULL", [])
                .unwrap();
        }

        let store = SqliteStore::new(&path, true, 64).unwrap();
        assert_eq!(store.index_load_report().missing, 1);
        assert_eq!(store.vector_index_len(), 0, "nothing to replay yet");

        assert_eq!(store.backfill_embeddings().unwrap(), 1);
        assert_eq!(store.vector_index_len(), 1);

        // Idempotent: a second pass has nothing left to do.
        assert_eq!(store.backfill_embeddings().unwrap(), 0);

        // And it is durable from here on.
        let reopened = SqliteStore::new(&path, true, 64).unwrap();
        assert_eq!(reopened.index_load_report().loaded, 1);
    }

    /// An embedding stored under a different model dimension cannot be indexed.
    /// It must be counted as unusable rather than aborting the open, and
    /// `reindex_all` must recover it.
    #[test]
    fn test_dimension_change_is_recoverable_by_reindex() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dim.db");

        {
            let mut store = SqliteStore::new(&path, true, 64).unwrap();
            store.assert_claim(claim("person:jd", "lang:rust")).unwrap();
        }

        // Reopen with a different embedding dimension, as if the model changed.
        let store = SqliteStore::new(&path, true, 128).unwrap();
        assert_eq!(store.index_load_report().unusable, 1);
        assert_eq!(store.index_load_report().loaded, 0);
        assert!(store.index_load_report().has_gaps());
        assert_eq!(store.vector_index_len(), 0);

        // A backfill cannot help (the row has a vector, just the wrong one);
        // the offline reindex in ADR-014 is what recovers it.
        assert_eq!(store.backfill_embeddings().unwrap(), 0);
        assert_eq!(store.reindex_all().unwrap(), 1);
        assert_eq!(store.vector_index_len(), 1);

        let reopened = SqliteStore::new(&path, true, 128).unwrap();
        assert_eq!(reopened.index_load_report().loaded, 1);
        assert_eq!(reopened.index_load_report().unusable, 0);
    }

    /// A corrupt embedding must not stop the instance from opening the store.
    #[test]
    fn test_corrupt_embedding_does_not_block_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.db");

        {
            let mut store = SqliteStore::new(&path, true, 64).unwrap();
            store.assert_claim(claim("person:jd", "lang:rust")).unwrap();
            store
                .conn
                .execute("UPDATE claims SET embedding_vector = 'not json'", [])
                .unwrap();
        }

        let store = SqliteStore::new(&path, true, 64).unwrap();
        assert_eq!(store.index_load_report().unusable, 1);
        assert_eq!(store.reindex_all().unwrap(), 1);
        assert_eq!(store.vector_index_len(), 1);
    }
}
