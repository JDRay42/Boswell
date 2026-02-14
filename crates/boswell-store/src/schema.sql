-- Boswell SQLite Schema (per ADR-005)
-- This schema supports claims, relationships, provenance, and confidence caching

-- Claims table - the core knowledge store
CREATE TABLE IF NOT EXISTS claims (
    -- ULID as 128-bit integer (stored as BLOB for efficient indexing)
    id BLOB PRIMARY KEY NOT NULL,
    
    -- Claim content
    namespace TEXT NOT NULL,
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL,
    object TEXT NOT NULL,
    
    -- Base confidence interval (from provenance aggregation)
    base_lower REAL NOT NULL CHECK (base_lower >= 0.0 AND base_lower <= 1.0),
    base_upper REAL NOT NULL CHECK (base_upper >= 0.0 AND base_upper <= 1.0),
    
    -- Tier and timestamps
    tier TEXT NOT NULL CHECK (tier IN ('ephemeral', 'task', 'project', 'permanent')),
    created_at INTEGER NOT NULL,
    stale_at INTEGER,
    
    -- Embedding vector (stored as JSON array for flexibility)
    -- In production, this could be optimized with custom storage
    embedding_vector TEXT,
    
    -- Metadata for semantic search quality
    content_hash TEXT,  -- For exact duplicate detection
    
    -- Table-level constraint to ensure confidence interval is valid
    CHECK (base_lower <= base_upper)
);

-- Indexes for common query patterns on claims
CREATE INDEX IF NOT EXISTS idx_claims_namespace ON claims(namespace);
CREATE INDEX IF NOT EXISTS idx_claims_tier ON claims(tier);
CREATE INDEX IF NOT EXISTS idx_claims_created_at ON claims(created_at);
CREATE INDEX IF NOT EXISTS idx_claims_content_hash ON claims(content_hash);

-- Relationships table (pairwise only, per ADR-002)
CREATE TABLE IF NOT EXISTS relationships (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    
    -- Source and target claims
    from_claim_id BLOB NOT NULL,
    to_claim_id BLOB NOT NULL,
    
    -- Relationship metadata
    relationship_type TEXT NOT NULL CHECK (relationship_type IN 
        ('supports', 'contradicts', 'derived_from', 'references', 'supersedes')),
    strength REAL NOT NULL CHECK (strength >= 0.0 AND strength <= 1.0),
    created_at INTEGER NOT NULL,
    
    -- Foreign keys
    FOREIGN KEY (from_claim_id) REFERENCES claims(id) ON DELETE CASCADE,
    FOREIGN KEY (to_claim_id) REFERENCES claims(id) ON DELETE CASCADE,
    
    -- Prevent duplicate relationships
    UNIQUE(from_claim_id, to_claim_id, relationship_type)
);

-- Indexes for relationship lookups
CREATE INDEX IF NOT EXISTS idx_relationships_from ON relationships(from_claim_id);
CREATE INDEX IF NOT EXISTS idx_relationships_to ON relationships(to_claim_id);
CREATE INDEX IF NOT EXISTS idx_relationships_type ON relationships(relationship_type);

-- Provenance table - tracks source of each claim
CREATE TABLE IF NOT EXISTS provenance (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    
    -- Which claim this provenance belongs to
    claim_id BLOB NOT NULL,
    
    -- Source information
    source TEXT NOT NULL,
    source_type TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    rationale TEXT,
    
    -- Confidence contribution from this source
    confidence_contribution REAL NOT NULL CHECK (confidence_contribution >= 0.0 AND confidence_contribution <= 1.0),
    
    -- Foreign key
    FOREIGN KEY (claim_id) REFERENCES claims(id) ON DELETE CASCADE
);

-- Index for provenance lookups
CREATE INDEX IF NOT EXISTS idx_provenance_claim ON provenance(claim_id);
CREATE INDEX IF NOT EXISTS idx_provenance_source_type ON provenance(source_type);

-- Confidence cache table - stores computed effective confidence for fast reads
CREATE TABLE IF NOT EXISTS confidence_cache (
    claim_id BLOB PRIMARY KEY NOT NULL,
    
    -- Cached effective confidence (after all adjustments)
    effective_lower REAL NOT NULL CHECK (effective_lower >= 0.0 AND effective_lower <= 1.0),
    effective_upper REAL NOT NULL CHECK (effective_upper >= 0.0 AND effective_upper <= 1.0),
    
    -- When this cache entry was computed
    computed_at INTEGER NOT NULL,
    
    -- Cache invalidation tracking
    -- This increases when relationships change, signaling recomputation needed
    version INTEGER NOT NULL DEFAULT 0,
    
    -- Table-level constraints
    CHECK (effective_lower <= effective_upper),
    
    -- Foreign key
    FOREIGN KEY (claim_id) REFERENCES claims(id) ON DELETE CASCADE
);

-- Metadata table for schema versioning and migrations
CREATE TABLE IF NOT EXISTS schema_info (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL,
    description TEXT
);

-- Insert initial schema version
INSERT INTO schema_info (version, applied_at, description) 
VALUES (1, strftime('%s', 'now') * 1000, 'Initial schema with claims, relationships, provenance, and confidence cache');

-- Notes on HNSW vector index:
-- The HNSW index is maintained separately in a memory-mapped file alongside this SQLite database.
-- The embedding_vector column in the claims table is primarily for reconstruction/debugging.
-- Vector similarity search queries will use the HNSW index, not SQL queries.
-- The HNSW index maps ULID → vector and provides approximate nearest neighbor search.
