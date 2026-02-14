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
//! let store = SqliteStore::new(":memory:").unwrap();
//! // Store is now ready for claim operations
//! ```

#![warn(missing_docs)]

use boswell_domain::{Claim, ClaimId, Relationship, RelationshipType};
use boswell_domain::traits::{ClaimStore, ClaimQuery};
use rusqlite::{Connection, params, OptionalExtension};
use std::path::Path;
use thiserror::Error;

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
/// It uses SQLite for structured data and will integrate HNSW for vector search.
///
/// # Thread Safety
///
/// SQLite connections are not thread-safe. Each thread should have its own SqliteStore instance.
pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    /// Create a new SqliteStore with the given database path
    ///
    /// Use `:memory:` for an in-memory database (useful for testing).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use boswell_store::SqliteStore;
    ///
    /// let store = SqliteStore::new("boswell.db").unwrap();
    /// ```
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        let mut store = Self { conn };
        store.initialize_schema()?;
        Ok(store)
    }
    
    /// Initialize the database schema
    fn initialize_schema(&mut self) -> Result<(), StoreError> {
        // Read and execute the schema SQL
        let schema = include_str!("schema.sql");
        
        // Execute each statement (SQLite doesn't support multiple statements in one execute)
        self.conn.execute_batch(schema)?;
        
        Ok(())
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
        
        // TODO: Add duplicate detection via embedding similarity
        // For now, we just check if the ID already exists
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
            "INSERT INTO claims (id, namespace, subject, predicate, object, base_lower, base_upper, tier, created_at, stale_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                &id_bytes,
                &claim.namespace,
                &claim.subject,
                &claim.predicate,
                &claim.object,
                claim.confidence.0,
                claim.confidence.1,
                &claim.tier,
                claim.created_at as i64,
                claim.stale_at.map(|t| t as i64),
            ],
        )?;
        
        Ok(claim.id)
    }
    
    fn get_claim(&self, id: ClaimId) -> Result<Option<Claim>, Self::Error> {
        let id_bytes = Self::claim_id_to_bytes(id);
        
        let claim = self.conn.query_row(
            "SELECT id, namespace, subject, predicate, object, base_lower, base_upper, tier, created_at, stale_at
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
            "SELECT id, namespace, subject, predicate, object, base_lower, base_upper, tier, created_at, stale_at
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
}
