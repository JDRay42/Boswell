# ADR-013: Local Embedding Models by Default

## Status

Accepted

## Context

Embedding generation is the highest-frequency LLM-adjacent operation in the system — called on every Assert, Learn, and Query. Unlike the Extractor, Synthesizer, and Gatekeeper (which are invoked less frequently and benefit from frontier model quality), embeddings need to be fast, cheap, and free of network dependencies.

## Decision

**Default to local embedding models** running via ONNX. Ship with `bge-small-en-v1.5` (384 dimensions) and `nomic-embed-text` (768 dimensions). User selects at instance creation. The embedding model is a per-instance configuration.

The vector index exists to get queries into the right semantic neighborhood. Fine-grained reasoning about nuance happens in the LLM-backed subsystems, not in vector similarity.

## Consequences

- Zero network latency on writes and reads.
- Zero API cost per embedding.
- Full version control — the model never changes unless the user explicitly decides to change it. No risk of a provider deprecating an embedding model and forcing migration.
- Privacy: claim content never leaves the user's infrastructure for embedding purposes.
- Re-embedding migration path is built in from day one. Changing models requires an offline reindex: stop instance, run batch re-embed, rebuild vector index, restart. Estimated throughput: 500-1000 claims/second for `bge-small`, ~250-500 for `nomic`. 1 million claims reindexes in 15-60 minutes depending on model.
- Reindexing is a dead-stop operation. No maintenance mode, no accepting writes during reindex. Router graceful degradation handles the unavailability window.
