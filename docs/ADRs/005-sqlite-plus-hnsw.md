# ADR-005: SQLite Plus HNSW Vector Sidecar

## Status

Accepted

## Context

The system needs to serve three access patterns well:

1. **Structured queries** — point lookups by ID, namespace prefix scans, filtered queries on tier/status/temporality. Relational territory.
2. **Semantic search** — nearest-neighbor lookup over embedding vectors. Specialized vector index territory.
3. **Event history** — temporal queries ("what changed since X"). Append-only log territory, but ULID-based primary keys make this a range scan.

No single embedded storage engine serves all three patterns optimally at scale (millions of claims). The write concurrency profile (dozens of agents at peak, handful typical) is moderate.

Engines evaluated:

- **SQLite alone**: Excellent for structured queries and temporal scans. Vector extensions (`sqlite-vss`, `sqlite-vec`) exist but degrade at millions of embeddings. WAL mode handles the write concurrency comfortably.
- **DuckDB**: Columnar, analytical-oriented, good for bulk queries. Less proven for concurrent transactional writes.
- **Multi-engine (SQLite + purpose-built vector index + separate event log)**: Optimal per-pattern performance but three-way synchronization complexity.

## Decision

**Two-engine architecture: SQLite as primary store and source of truth, plus a dedicated HNSW vector index as a sidecar.**

- SQLite (WAL mode) stores all claim data, provenance, relationships, lifecycle metadata. Serves structured queries and temporal queries.
- The HNSW sidecar (e.g., `usearch`, memory-mapped) stores only `(claim_id, embedding)` pairs. Serves semantic search.

## Consequences

- **Single source of truth** with ACID guarantees (SQLite). All writes go to SQLite first.
- **Optimized semantic search** via purpose-built index that scales to millions of vectors with memory-mapped access.
- **Simple backup and portability**: one SQLite file plus a rebuildable index file.
- **No synchronization headaches**: the vector index is updated as a post-write hook. If it falls behind, the worst case is a brief window where a new claim isn't semantically searchable yet but is still queryable by structure.
- **The vector index is a derived projection.** If corrupted or lost, it is fully rebuildable by scanning SQLite and re-indexing embeddings. Nothing is lost.
- **Temporal queries** are served by range scans on the ULID primary key, which embeds creation timestamps. No separate event log needed.
- **Reindexing (model change or index corruption) is an offline operation.** Instance goes down, reindex runs, instance comes back up. No maintenance mode complexity. The Router's graceful degradation handles unavailability during reindex.
