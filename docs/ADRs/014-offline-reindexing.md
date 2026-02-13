# ADR-014: Reindexing Is a Dead-Stop Offline Operation

## Status

Accepted

## Context

When changing embedding models or recovering from vector index corruption, every claim must be re-embedded and the vector index rebuilt. This raises the question of whether the instance should remain available during reindex.

A maintenance mode that accepts writes during reindex creates edge cases: claims written with the old model's embeddings while the new model is being applied, queue ordering ambiguity, and partial index states requiring complex reconciliation logic.

## Decision

**Reindexing is a completely offline operation.** The instance goes down, the reindex runs to completion, the instance comes back up. There is no maintenance mode, no partial availability, no write queuing.

## Consequences

- Zero ambiguity about index state at any point in time. The index is either fully consistent or the instance is down.
- No complexity around write queuing, model mixing, or partial index states.
- The Router's graceful degradation handles the unavailability window naturally — the instance appears as unreachable, other instances continue serving, federated queries return partial results with transparency.
- Reindexing happens rarely (only on deliberate model change or index corruption). The operational cost of downtime is low relative to the complexity cost of maintaining consistency during online reindex.
- This is a correctness-over-availability tradeoff appropriate for a personal knowledge system where brief downtime of one instance is acceptable.
