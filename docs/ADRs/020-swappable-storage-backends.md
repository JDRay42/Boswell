# ADR-020: Swappable Storage Backends (SQLite Default, Postgres Optional)

## Status

Proposed

## Context

[ADR-005](005-sqlite-plus-hnsw.md) chose a two-engine store: SQLite as the source of truth plus
a rebuildable HNSW vector sidecar. That is an excellent local-first default.

As Boswell grows toward shared, multi-agent, and hosted deployments, some operators will want a
networked, concurrent, operationally mature datastore — in particular **PostgreSQL with the
`pgvector` extension**, which holds structured data and embeddings in one transactional system.

Persistence already sits behind the `ClaimStore` trait (`boswell-domain`), and `semantic_search`
is part of that contract, so an alternative engine is an **adapter, not a redesign**.

This is distinct from a lowest-common-denominator "memory connector" over systems like Obsidian
or Mem0 (see Alternatives): those cannot represent Boswell's claim model. Here we swap the
*storage engine underneath the same rich model*.

## Decision

Treat storage as a **swappable adapter behind `ClaimStore`**:

- **SQLite + local vector index remains the default embedded adapter** (ADR-005) — local-first,
  zero-dependency.
- **Add PostgreSQL + pgvector as an optional adapter** for shared / multi-agent / hosted
  deployments.
- Every adapter implements the **same** `ClaimStore` contract with the **same** Boswell
  semantics; the model, the gRPC/gateway API, and the tiers are identical regardless of engine.

## Consequences

- **Local-first default is preserved.** Postgres is opt-in; the embedded SQLite adapter stays the
  zero-friction default, and CI keeps running the in-process SQLite path (a Postgres adapter adds
  a service-backed test lane).
- **Adapter-specific durability.** SQLite backup snapshots the database (the vector index is a
  rebuildable projection — ADR-005); Postgres uses `pg_dump` / WAL point-in-time recovery with
  embeddings in-DB. See [`16-backup-recovery.md`](../architecture/16-backup-recovery.md).
- **Contract hardening is a prerequisite for a production-grade networked backend.** Two current
  shapes are SQLite-flavored and should be revisited before a Postgres adapter is real:
  1. `ClaimStore` is **synchronous with `&mut self`** and the service serializes on
     `Arc<Mutex<S>>`. A pooled, async Postgres client wants `&self` + async. Likely outcome: an
     **async `ClaimStore`**.
  2. **Query pushdown.** `ClaimQuery` does not carry subject/predicate/object; the gRPC service
     filters those in memory *above* the store. A SQL backend should push all filters into
     indexed `WHERE` clauses, so `ClaimQuery` needs enriching.
  - Vector-search parameters already generalize well (`semantic_search(text, limit,
    min_similarity)`); engine-specific knobs (HNSW `ef_search`, pgvector index type) stay below
    the trait.
- **A migration tool is needed** to move memories between adapters (e.g. SQLite → Postgres),
  preserving ids, tiers, confidence, provenance, and timestamps. Backlogged; it makes the "start
  simple, grow" path real so choosing SQLite first costs nothing later.

## Alternatives considered

- **Single hardcoded SQLite store.** Rejected: caps Boswell at single-node/local scale and blocks
  the shared/hosted deployments some users will need.
- **A generic multi-backend memory connector (Obsidian, Mem0, …).** Rejected: those systems
  cannot represent claims, tiers, confidence, or gatekeeping, forcing a lowest-common-denominator
  interface that dissolves Boswell's value. Swapping the storage *engine* under the same
  `ClaimStore` contract is a different thing entirely.
