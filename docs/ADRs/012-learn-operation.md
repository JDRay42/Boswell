# ADR-012: Learn Operation for Direct Knowledge Loading

## Status

Accepted

## Context

The Extractor subsystem handles text → claims conversion via LLM, but there are scenarios where structured knowledge already exists in the correct format: exports from previous sessions, curated knowledge bases, domain ontologies, or other Boswell instances. Forcing pre-formatted data through the Extractor wastes inference and risks losing fidelity through re-interpretation.

Conceptual reference: the "I know kung fu" scene from The Matrix — directly loading a structured knowledge module into the system without the overhead of learning from scratch.

## Decision

A **Learn** operation that accepts pre-formatted claims (or batches) with relationships and provenance intact, validates the structure, and writes directly to the Claim Store. Provenance carries `source_type: direct_load`.

## Consequences

- Bulk import of structured knowledge is fast and preserves fidelity.
- No LLM cost for loading pre-formatted data.
- Trust level on learned claims is a parameter — the caller decides whether to load at high confidence ("these are curated") or moderate confidence ("load but let the system validate").
- Conflict policy on load is a parameter — flag contradictions immediately, load quietly for Janitor resolution, or reject conflicting subset.
- Namespace and tier targeting are explicit parameters — learned claims don't automatically land in persistent storage.
- Enables instance-to-instance knowledge transfer, session backup/restore, and bootstrapping from external knowledge bases.
