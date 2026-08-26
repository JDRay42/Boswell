# boswell-synthesizer

Discovers emergent patterns across the claim graph and creates higher-order
**derived claims**. Part of Boswell's application layer (Phase 3, Stream D4).

See [`docs/architecture/06-synthesizer.md`](../../docs/architecture/06-synthesizer.md)
and [ADR-006](../../docs/ADRs/006-convention-based-namespaces.md) for the design.

## What it does

The Synthesizer examines clusters of related claims and asks an LLM whether they
*together* imply a single higher-order insight that no individual claim states on
its own — a pattern, trend, or principle. When one is found, it is stored as a new
claim linked back to its constituents via `derived_from` relationships, building
organic abstraction layers (first-order → second-order → …).

```text
candidate claims → cluster → LLM analysis → insight → gatekeeper → store
                                  │                        │
                            "no insight" ok          derived_from edges
```

## A single pass

```rust,no_run
use boswell_synthesizer::{Synthesizer, SynthesizerConfig, SynthesisScope};
use boswell_llm::OllamaProvider;
use boswell_store::SqliteStore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut store = SqliteStore::new("boswell.db", false, 0)?;
    let llm = OllamaProvider::new("http://localhost:11434", "llama3");
    let synthesizer = Synthesizer::new(llm, SynthesizerConfig::default());

    // All namespaces, task tier and above, up to 50 clusters.
    let report = synthesizer
        .run_pass(&mut store, SynthesisScope::all("task", 50))
        .await?;

    println!("{}", report.summary());
    Ok(())
}
```

## Background worker

```rust,no_run
use boswell_synthesizer::{SynthesizerWorker, SynthesizerConfig};
use boswell_llm::OllamaProvider;
use boswell_store::SqliteStore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::new("boswell.db", false, 0)?;
    let llm = OllamaProvider::new("http://localhost:11434", "llama3");
    let worker = SynthesizerWorker::new(llm, SynthesizerConfig::default());

    worker.run(store).await?; // runs on the configured interval until Ctrl+C
    Ok(())
}
```

## Design highlights

- **Candidate selection** — claims at or above a minimum tier (default `task`,
  skipping ephemeral noise), optionally scoped by namespace and recency.
- **Clustering** — transitive union-find over relationship edges *and* shared
  subjects, surfacing the high-density clusters most likely to yield insight.
- **"No insight" is valid** — the prompt makes an empty result a first-class,
  common outcome. Quality over quantity.
- **Confidence propagation** — a derived claim is never more certain than its
  weakest constituent; uncertainty widens outward through inference chains.
- **Depth limiting** — clusters whose claims already sit at the maximum
  `derived_from` depth are skipped, preventing runaway meta-synthesis.
- **Gatekeeper validation** — every synthesized claim is validated (entity
  format, confidence bounds, tier appropriateness, duplicates) before persisting.
- **Dry-run mode** — analyse and report what *would* be created without writing.

## Configuration

| Setting | Default | Description |
|---|---|---|
| `enabled` | `true` | Master switch; a disabled pass is a no-op |
| `synthesis_interval_hours` | `6` | Worker schedule |
| `min_tier` | `task` | Lowest tier considered (skips ephemeral noise) |
| `max_clusters_per_pass` | `50` | Cost cap on clusters evaluated per pass |
| `min_cluster_size` | `3` | Minimum claims for a cluster to be considered |
| `max_cluster_size` | `12` | Cluster truncation cap (keeps strongest claims) |
| `max_derivation_depth` | `5` | Limit on `derived_from` chain depth |
| `min_insight_confidence` | `0.5` | Insights below this upper-confidence are dropped |
| `dry_run` | `false` | Analyse without persisting |
| `cluster_timeout_secs` | `60` | Per-cluster LLM call timeout |

Presets: `SynthesizerConfig::default()`, `::aggressive()`, `::conservative()`.
Config can be loaded from TOML via `SynthesizerConfig::from_toml(..)` (bare or
under a `[synthesizer]` table).

## Testing

```bash
cargo test -p boswell-synthesizer
```

Covers confidence propagation, clustering, prompt/response parsing, and
end-to-end passes (insight creation, "no insight", dry-run, depth limiting,
low-confidence rejection, and scope filtering) against an in-memory SQLite store.
