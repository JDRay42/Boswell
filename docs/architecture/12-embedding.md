# Boswell — Embedding Models

Embeddings are the highest-frequency model operation in Boswell. Every Assert, Learn, Extract, and semantic Query requires an embedding computation. For this reason, embeddings always run locally — no network calls, no API costs, no external dependency.

## Principles

1. **Local only.** Embedding models run in-process via ONNX. No network latency on the critical path.
2. **Per-instance configuration.** Each instance uses one embedding model. The model is chosen at instance creation and recorded in instance metadata.
3. **No mixing.** Embeddings from different models are incompatible. You cannot query a vector index built with one model using embeddings from another. The instance enforces consistency.
4. **Switchable with full reindex.** Changing embedding models requires taking the instance offline and rebuilding the vector index. This is a maintenance operation, not a hot swap.

## Shipped Models

Boswell ships with two embedding models available out of the box. The user selects one during instance creation.

### bge-small-en-v1.5

| Property | Value |
|---|---|
| Dimensions | 384 |
| Parameters | 33M |
| Throughput (CPU) | ~500-1000 sentences/sec |
| Languages | English (primarily) |
| License | MIT |
| Disk size | ~130 MB |

**When to use:** Default choice. Fast, small, sufficient for the semantic distinctions Boswell needs. The vector index is compact — 384 dimensions means roughly 1.5 KB per embedding. At 10 million claims, the index is approximately 15 GB on disk, easily memory-mapped on modest hardware.

### nomic-embed-text

| Property | Value |
|---|---|
| Dimensions | 768 |
| Parameters | 137M |
| Throughput (CPU) | ~200-500 sentences/sec |
| Languages | Multilingual |
| License | Apache 2.0 |
| Disk size | ~530 MB |

**When to use:** When multilingual support is needed, or when finer semantic granularity is desired. Larger index footprint — 768 dimensions means roughly 3 KB per embedding. At 10 million claims, the index is approximately 30 GB on disk.

### Comparison

| Factor | bge-small | nomic-embed |
|---|---|---|
| Speed | Faster (~2x) | Slower |
| Index size | Smaller (~2x) | Larger |
| Semantic quality | Good | Better (larger model) |
| Multilingual | English only | Yes |
| RAM impact | Lower | Higher |

For most users starting with English-language knowledge, `bge-small-en-v1.5` is the right default. The vector search only needs to get into the right neighborhood — the LLM-backed subsystems (Synthesizer, deliberate query) handle fine-grained semantic reasoning.

## Adding New Models

Boswell can use any ONNX-compatible embedding model. To add a new model:

1. Export the model to ONNX format (most popular models have ONNX exports available).
2. Place the model file in the configured models directory.
3. Register the model in the instance configuration with its dimension count.
4. Create a new instance with this model, or reindex an existing instance (see below).

The embedding pipeline accepts any model that takes text input and produces a fixed-dimension float vector. Tokenization is handled by the model's bundled tokenizer.

## Reindexing

When switching embedding models on an existing instance, a full reindex is required.

### Process

1. **Take the instance offline.** Stop all reads and writes. No partial availability, no queued writes. The instance is fully down during reindex.
2. **Update the embedding model configuration** to point to the new model.
3. **Run the reindex command:** `boswell reindex --instance <id>`
4. The reindex process:
   - Scans every claim in the SQLite database.
   - Computes a new embedding for each claim's `raw_expression` using the new model.
   - Updates the embedding column in SQLite.
   - Rebuilds the HNSW vector index from scratch.
5. **Bring the instance back online.**

### Duration Estimates

Based on the shipped models running on modern hardware (M4 Mac Mini or equivalent):

| Claim Count | bge-small | nomic-embed |
|---|---|---|
| 100,000 | ~2-3 minutes | ~4-8 minutes |
| 1,000,000 | ~15-30 minutes | ~30-80 minutes |
| 10,000,000 | ~2.5-5 hours | ~5-11 hours |

These are batch operations. Run overnight or during a maintenance window.

### Graceful Degradation

The Router marks the reindexing instance as unreachable. In multi-instance deployments, other instances continue operating normally. Federated queries return partial results with transparency about which instances responded. In single-instance deployments, the Router returns unavailability responses until reindexing completes. The system degrades gracefully — reindexing one instance does not affect the others.

## ONNX Runtime

Boswell uses the `ort` crate (ONNX Runtime bindings for Rust) for in-process model inference. Properties:

- **In-process.** No separate inference server. The embedding model is loaded into the Boswell instance process.
- **CPU by default.** Works on any hardware. No GPU requirement.
- **Hardware acceleration.** On Apple Silicon, ONNX Runtime can use the ANE (Apple Neural Engine) via CoreML. On NVIDIA GPUs, it can use CUDA. These are optional acceleration paths — CPU inference is the baseline.
- **Memory.** The model is loaded once at startup. bge-small uses ~200 MB resident; nomic-embed uses ~600 MB. After loading, per-inference memory is negligible.

## Configuration

```toml
[embedding]
model = "bge-small-en-v1.5"        # Active embedding model
models_dir = "./models"              # Directory containing ONNX model files
dimensions = 384                     # Must match the model's output dimensions
batch_size = 32                      # Embeddings computed in batches for throughput
```

The `dimensions` field is a safety check. If the configured model produces vectors of a different dimension than specified, the instance refuses to start. This prevents silent misconfigurations.

## Embedding in the Architecture

The embedding model is used by:

- **Claim Store** — embeds incoming claims during Assert/Learn, embeds query text during semantic Query.
- **Extractor** — embeds extracted claims before passing them to the Claim Store.
- **Duplicate detection** — compares incoming claim embeddings against the HNSW index.
- **Router** — if using embedding-similarity-based topic classification, the Router needs its own embedding model for comparing claims against expertise profile signatures.

The embedding model is **not** used by:
- **Synthesizer** — works with claims already in the store (already embedded).
- **Gatekeeper** — evaluates claims by content, not by embedding similarity.
- **Janitor** — operates on claim metadata and relationships, not embeddings.
