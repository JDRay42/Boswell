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
    
    -- How this claim came to exist (assertion, extraction, inference, import)
    source_type TEXT NOT NULL DEFAULT 'assertion',

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
-- Note: idx_claims_source_type is created by run_migrations() in lib.rs, after
-- ensuring the source_type column exists on databases created before it was added.

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

-- Insert initial schema version.
-- OR IGNORE keeps this idempotent: initialize_schema() runs the full schema on
-- every SqliteStore::new(), so reopening an existing file-based database must not
-- fail on the schema_info.version UNIQUE (PRIMARY KEY) constraint.
INSERT OR IGNORE INTO schema_info (version, applied_at, description)
VALUES (1, strftime('%s', 'now') * 1000, 'Initial schema with claims, relationships, provenance, and confidence cache');

-- Procedures table - procedural memory (per docs/architecture/15-procedural-memory.md)
-- A procedure is a first-class entity (sibling to claims, NOT made of claims): a
-- typed, uniform signature plus an opaque, format-tagged body. Effectiveness is
-- DERIVED at read time (success-rate x recency), never stored. Collection-valued
-- signature fields are stored as JSON text; scalar/filterable fields are columns.
CREATE TABLE IF NOT EXISTS procedures (
    -- UUIDv7 as 128-bit integer (stored as BLOB for efficient indexing)
    id BLOB PRIMARY KEY NOT NULL,

    -- Identity / versioning
    namespace TEXT NOT NULL,
    name TEXT NOT NULL,
    version INTEGER NOT NULL,
    supersedes BLOB,                       -- prior-version id, or NULL
    is_current INTEGER NOT NULL DEFAULT 1 CHECK (is_current IN (0, 1)),
    source TEXT NOT NULL CHECK (source IN ('authored', 'learned', 'imported')),

    -- Grouping: handle for variants/versions pursuing the same outcome
    goal TEXT NOT NULL,

    -- Signature (uniform; drives retrieval + gating)
    intent TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',            -- JSON array of strings
    parameters TEXT NOT NULL DEFAULT '[]',      -- JSON array of parameter objects
    preconditions TEXT NOT NULL DEFAULT '[]',   -- JSON array of precondition objects
    required_tools TEXT NOT NULL DEFAULT '[]',  -- JSON array of strings
    postconditions TEXT NOT NULL DEFAULT '[]',  -- JSON array of strings
    est_duration_sec INTEGER,

    -- Selection hints (soft; ranking among siblings)
    usage_notes TEXT NOT NULL DEFAULT '',
    context_tags TEXT NOT NULL DEFAULT '[]',    -- JSON array of strings

    -- Body (opaque, format-tagged)
    body_format TEXT NOT NULL CHECK (body_format IN ('prose', 'dsl', 'code')),
    content_type TEXT NOT NULL,
    body TEXT NOT NULL,

    -- Effectiveness & lifecycle
    tier TEXT NOT NULL CHECK (tier IN ('ephemeral', 'task', 'project', 'permanent')),
    use_count INTEGER NOT NULL DEFAULT 0,
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    last_used_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    stale_at INTEGER,

    -- Optional embedding of `intent` for future semantic retrieval (ADR-005).
    -- Phase 1 matches goal/intent exactly or by substring; see TODO in the store.
    embedding_vector TEXT
);

-- Indexes for common procedure query patterns
CREATE INDEX IF NOT EXISTS idx_procedures_goal ON procedures(goal);
CREATE INDEX IF NOT EXISTS idx_procedures_namespace ON procedures(namespace);
CREATE INDEX IF NOT EXISTS idx_procedures_is_current ON procedures(is_current);

-- Goals table - navigational decomposition nodes (procedural memory Phase 2,
-- per docs/architecture/15-procedural-memory.md §3.2). Skinny by design:
-- traversal walks these rows, so they carry no children and no executable body.
-- Children live in the goal_edges table, keyed by parent, so a hop is a single
-- indexed adjacency read. Goals form a DAG (a goal may be reused under several
-- parents); acyclicity is enforced on edge insert by the store.
CREATE TABLE IF NOT EXISTS goals (
    -- UUIDv7 as 128-bit integer (stored as BLOB for efficient indexing)
    id BLOB PRIMARY KEY NOT NULL,

    namespace TEXT NOT NULL,
    name TEXT NOT NULL,
    intent TEXT NOT NULL,                        -- semantic match key (entry hop)
    definition_of_done TEXT NOT NULL DEFAULT '[]', -- JSON array of postconditions

    tier TEXT NOT NULL CHECK (tier IN ('ephemeral', 'task', 'project', 'permanent')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    stale_at INTEGER,

    -- Optional embedding of `intent` for future semantic entry-point match.
    embedding_vector TEXT
);

CREATE INDEX IF NOT EXISTS idx_goals_namespace ON goals(namespace);

-- Goal edges - self-describing decomposition edges (design §3.2, §4.2). Each
-- edge points at a child that is EITHER a sub-goal OR a procedure, and carries
-- inline everything a hop needs to filter and rank without fetching the child
-- row: edge-local preconditions, context_tags, usage_notes, role, and a cached
-- effectiveness. child_id is polymorphic (goals or procedures), so it has no
-- foreign key; parent_goal_id cascades from goals.
CREATE TABLE IF NOT EXISTS goal_edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,

    parent_goal_id BLOB NOT NULL,
    child_kind TEXT NOT NULL CHECK (child_kind IN ('goal', 'procedure')),
    child_id BLOB NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('accomplish', 'decide')),

    preconditions TEXT NOT NULL DEFAULT '[]',  -- JSON array of precondition objects
    context_tags TEXT NOT NULL DEFAULT '[]',   -- JSON array of strings
    usage_notes TEXT NOT NULL DEFAULT '',
    cached_effectiveness REAL NOT NULL DEFAULT 0.0
        CHECK (cached_effectiveness >= 0.0 AND cached_effectiveness <= 1.0),

    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,

    FOREIGN KEY (parent_goal_id) REFERENCES goals(id) ON DELETE CASCADE,

    -- One edge per (parent, child, role): re-adding updates it in place.
    UNIQUE(parent_goal_id, child_kind, child_id, role)
);

CREATE INDEX IF NOT EXISTS idx_goal_edges_parent ON goal_edges(parent_goal_id);

-- Notes on HNSW vector index:
-- The HNSW index is maintained separately in a memory-mapped file alongside this SQLite database.
-- The embedding_vector column in the claims table is primarily for reconstruction/debugging.
-- Vector similarity search queries will use the HNSW index, not SQL queries.
-- The HNSW index maps ULID → vector and provides approximate nearest neighbor search.
