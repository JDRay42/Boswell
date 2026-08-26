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

pub mod vector_index;
pub mod embedding;
pub mod ollama_embedding;

use boswell_domain::{Claim, ClaimId, Relationship, RelationshipType};
use boswell_domain::traits::{ClaimStore, ClaimQuery};
use rusqlite::{Connection, params, OptionalExtension};
use std::path::Path;
use thiserror::Error;

pub use vector_index::VectorIndex;
pub use embedding::{EmbeddingModel, MockEmbeddingModel, cosine_similarity};
pub use ollama_embedding::OllamaEmbeddingModel;

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
    pub fn new<P: AsRef<Path>>(path: P, enable_vector_search: bool, embedding_dimension: usize) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        
        let (vector_index, embedding_model) = if enable_vector_search {
            (
                Some(VectorIndex::new(embedding_dimension)),
                Some(Box::new(MockEmbeddingModel::new(embedding_dimension)) as Box<dyn EmbeddingModel + Send + Sync>),
            )
        } else {
            (None, None)
        };
        
        let mut store = Self { conn, vector_index, embedding_model };
        store.initialize_schema()?;
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
        };
        store.initialize_schema()?;
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

        Ok(())
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
    
    /// Convert ClaimId to bytes for storage
    fn claim_id_to_bytes(id: ClaimId) -> Vec<u8> {
        id.value().to_be_bytes().to_vec()
    }
    
    /// Convert bytes to ClaimId
    fn bytes_to_claim_id(bytes: &[u8]) -> Result<ClaimId, StoreError> {
        if bytes.len() != 16 {
            return Err(StoreError::InvalidData(
                format!("Expected 16 bytes for ClaimId, got {}", bytes.len())
            ));
        }
        let mut arr = [0u8; 16];
        arr.copy_from_slice(bytes);
        Ok(ClaimId::from_value(u128::from_be_bytes(arr)))
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
            _ => Err(StoreError::InvalidData(format!("Unknown relationship type: {}", s))),
        }
    }
}

impl ClaimStore for SqliteStore {
    type Error = StoreError;
    
    fn assert_claim(&mut self, claim: Claim) -> Result<ClaimId, Self::Error> {
        let id_bytes = Self::claim_id_to_bytes(claim.id);
        
        // Check if the ID already exists
        let exists: bool = self.conn.query_row(
            "SELECT 1 FROM claims WHERE id = ?1",
            params![&id_bytes],
            |_| Ok(true)
        ).optional()?.unwrap_or(false);
        
        if exists {
            return Err(StoreError::Duplicate);
        }
        
        // Insert the claim
        self.conn.execute(
            "INSERT INTO claims (id, namespace, subject, predicate, object, source_type, base_lower, base_upper, tier, created_at, stale_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
            ],
        )?;
        
        // Auto-generate and add embedding if vector search is enabled
        if let (Some(embedding_model), Some(vector_index)) = 
            (&self.embedding_model, &self.vector_index) {
            // Create embedding text from claim content
            let text = format!("{} {} {}", claim.subject, claim.predicate, claim.object);
            
            match embedding_model.embed(&text) {
                Ok(embedding) => {
                    // Add to vector index (ignore errors for now)
                    let _ = vector_index.add(claim.id, &embedding);
                }
                Err(e) => {
                    // Log error but don't fail the claim insertion
                    eprintln!("Warning: Failed to generate embedding: {}", e);
                }
            }
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
        let mut sql = String::from(
            "SELECT id, namespace, subject, predicate, object, base_lower, base_upper, tier, created_at, stale_at, source_type
             FROM claims WHERE 1=1"
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        
        if let Some(namespace) = &query.namespace {
            sql.push_str(" AND namespace LIKE ?");
            params.push(Box::new(format!("{}%", namespace)));
        }
        
        if let Some(tier) = &query.tier {
            sql.push_str(" AND tier = ?");
            params.push(Box::new(tier.clone()));
        }
        
        if let Some(min_conf) = query.min_confidence {
            sql.push_str(" AND base_lower >= ?");
            params.push(Box::new(min_conf));
        }
        
        if let Some(limit) = query.limit {
            sql.push_str(" LIMIT ?");
            params.push(Box::new(limit));
        }
        
        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        
        let claims = stmt.query_map(&param_refs[..], |row| {
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
        })?.collect::<Result<Vec<_>, _>>()?;
        
        Ok(claims)
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
             FROM relationships WHERE from_claim_id = ?1 OR to_claim_id = ?1"
        )?;
        
        let relationships = stmt.query_map(params![&id_bytes], |row| {
            let from_bytes: Vec<u8> = row.get(0)?;
            let to_bytes: Vec<u8> = row.get(1)?;
            let rel_type_str: String = row.get(2)?;
            
            let from_claim = Self::bytes_to_claim_id(&from_bytes)
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
                    0, rusqlite::types::Type::Blob, Box::new(e)
                ))?;
            
            let to_claim = Self::bytes_to_claim_id(&to_bytes)
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
                    1, rusqlite::types::Type::Blob, Box::new(e)
                ))?;
            
            let relationship_type = Self::str_to_relationship_type(&rel_type_str)
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
                    2, rusqlite::types::Type::Text, Box::new(e)
                ))?;
            
            Ok(Relationship {
                from_claim,
                to_claim,
                relationship_type,
                strength: row.get(3)?,
                created_at: row.get::<_, i64>(4)? as u64,
            })
        })?.collect::<Result<Vec<_>, _>>()?;
        
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
}

/// Default HNSW `ef_search` quality parameter for trait-level semantic search.
const DEFAULT_EF_SEARCH: usize = 64;

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
        let vector_index = self.vector_index.as_ref()
            .ok_or_else(|| StoreError::InvalidData(
                "Vector search is not enabled for this store".to_string()
            ))?;
        
        // Search the vector index for similar claim IDs
        let similar_ids = vector_index.search(query_embedding, k, ef_search)
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
        let vector_index = self.vector_index.as_ref()
            .ok_or_else(|| StoreError::InvalidData(
                "Vector search is not enabled for this store".to_string()
            ))?;
        
        // Verify the claim exists
        if self.get_claim(claim_id)?.is_none() {
            return Err(StoreError::NotFound(claim_id.to_string()));
        }
        
        // Add to vector index
        vector_index.add(claim_id, embedding)
            .map_err(|e| StoreError::InvalidData(format!("Failed to add embedding: {}", e)))?;
        
        Ok(())
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
        assert_eq!(store.get_claim(id).unwrap().unwrap().source_type, "extraction");

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
}
