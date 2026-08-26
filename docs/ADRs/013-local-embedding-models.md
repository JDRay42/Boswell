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

## Update (2026-08-25): Ollama-backed implementation

The first real embedding backend is implemented against a local **Ollama** server (`OllamaEmbeddingModel` in `boswell-store`) rather than in-process ONNX, using **EmbeddingGemma** (Google, 2025; 768 dimensions, L2-normalized output) as the default model. Rationale:

- Ollama is already a project dependency (see ADR-015 for LLM providers), so model download, storage, and versioning are handled by one tool.
- EmbeddingGemma is newer and scores higher on MTEB than `bge-small-en-v1.5`/`nomic-embed-text` at a comparable size, and is purpose-built for on-device retrieval.
- The `EmbeddingModel` trait is unchanged, so an in-process ONNX backend remains a drop-in alternative if the service dependency becomes undesirable.

Deviations from the original decision, and their trade-offs:

- **"Zero network latency" becomes localhost latency.** Embedding is now a blocking HTTP call to `127.0.0.1:11434`. Content still never leaves the machine, so the privacy and cost consequences hold, but writes/reads incur a local round-trip (single-digit to low-tens of milliseconds).
- **Runtime service dependency.** Ollama must be running for embedding to work. `SqliteStore::with_embedding_model` probes the model at construction so misconfiguration fails fast; `SqliteStore::new` still uses the deterministic `MockEmbeddingModel` for tests and offline use.
- **Sync-over-async.** `EmbeddingModel::embed` is synchronous and is called from inside async request handlers, so the client uses `ureq` (blocking sockets, no runtime) rather than `reqwest::blocking` (which panics inside a Tokio runtime). Moving embedding off the request thread is a future optimization.
- **Dimension is 768** (EmbeddingGemma) rather than the ADR's 384/768 menu; the store sizes its HNSW index from the model's reported dimension, so this is configuration, not a schema change.
