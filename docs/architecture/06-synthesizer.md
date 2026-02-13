# Boswell — Synthesizer

The Synthesizer is a background process that continuously examines the claim graph and discovers emergent ideas — clusters of related claims that together imply a higher-order insight no individual claim represents.

## Responsibility

- Discover patterns, connections, and higher-order insights across existing claims.
- Produce new claims with `source_type: inference` and `derived_from` relationships linking back to constituent claims.
- Enable organic abstraction layers: first-order → second-order → third-order.
- At the Router level (multi-instance): discover cross-domain connections between instances.

## Design

```mermaid
graph BT
    subgraph First["First-Order Claims"]
        A["Claim A<br/>(from document)"]
        B["Claim B<br/>(agent assertion)"]
        C["Claim C<br/>(from extraction)"]
        D["Claim D<br/>(from extraction)"]
        E["Claim E<br/>(agent assertion)"]
    end

    subgraph Second["Second-Order Claims (Synthesizer)"]
        AB["Insight AB<br/>(A + B → pattern)"]
        CDE["Insight CDE<br/>(C + D + E → trend)"]
    end

    subgraph Third["Third-Order Abstractions (Synthesizer)"]
        ABCDE["Principle<br/>(cross-pattern insight)"]
    end

    A -->|derived_from| AB
    B -->|derived_from| AB
    C -->|derived_from| CDE
    D -->|derived_from| CDE
    E -->|derived_from| CDE
    AB -->|derived_from| ABCDE
    CDE -->|derived_from| ABCDE
```

### Background Process

The Synthesizer runs on a configurable schedule, not triggered by API calls. It is a continuous background process that scans claim clusters and identifies opportunities for synthesis.

**Execution cycle:**

```mermaid
sequenceDiagram
    participant Timer as Scheduler
    participant SY as Synthesizer
    participant CS as Claim Store
    participant LLM as LLM Provider

    Timer->>SY: Trigger synthesis pass
    SY->>CS: Fetch candidate claims<br/>(by namespace, tier, recency, relationship density)
    CS-->>SY: Claim clusters

    loop For each promising cluster
        SY->>LLM: Analyze cluster for emergent insights
        LLM-->>SY: Candidate insights (or none)

        alt Insight found
            SY->>CS: Assert new claim<br/>(source_type: inference)
            SY->>CS: Create derived_from relationships<br/>to constituent claims
        end
    end

    SY->>SY: Log pass results
```

### Candidate Selection

The Synthesizer does not scan every claim on every pass. It prioritizes:

1. **Recently modified claims.** New assertions, challenges, and corroborations may create new synthesis opportunities.
2. **Clusters with high relationship density.** Claims with many `supports`, `related_to`, or `refines` edges are more likely to yield compound insights.
3. **Claims with wide confidence intervals.** Areas of uncertainty may benefit from synthesis that connects disparate evidence.
4. **Namespaces not recently synthesized.** Round-robin across namespaces prevents one active domain from monopolizing synthesis resources.

### Resolving the Hyperedge Question

The claim model uses only pairwise relationships (see ADR-002). The compound relationship "Claims A, B, and C together imply Claim D" is represented as:

- Claim D with `source_type: inference` in its provenance.
- Three `derived_from` relationships: D→A, D→B, D→C.
- Three provenance entries referencing A, B, and C as sources.

The Synthesizer handles the complexity of multi-claim reasoning. The schema stays simple.

### Confidence Propagation

Synthesized claims naturally have wider confidence intervals than their constituents:

- The lower bound of a derived claim cannot exceed the minimum lower bound of its constituents.
- The upper bound is bounded by the LLM's assessed confidence in the inference.
- **Uncertainty propagates outward** through inference chains, which is epistemically correct. A chain of inferences should be less certain than its foundations.

When a constituent claim's confidence changes (challenge, corroboration, staleness decay), the Synthesizer can flag derived claims for re-evaluation on its next pass.

### Cross-Domain Synthesizer (Router Level)

In multi-instance deployments, a separate Synthesizer can run at the Router level:

```mermaid
graph TB
    subgraph Dev["Development Instance"]
        D1["High-confidence<br/>persistent claims"]
    end
    subgraph Personal["Personal Instance"]
        P1["High-confidence<br/>persistent claims"]
    end
    subgraph Professional["Professional Instance"]
        PR1["High-confidence<br/>persistent claims"]
    end

    XS["Cross-Domain Synthesizer<br/>(Router level)"]

    D1 -->|"pull persistent tier"| XS
    P1 -->|"pull persistent tier"| XS
    PR1 -->|"pull persistent tier"| XS

    XS -->|"cross-domain insight"| Dev
    XS -->|"cross-domain insight"| Personal
```

- Pulls only high-confidence, persistent-tier claims from each instance. No ephemeral or task-level noise.
- Looks for cross-domain connections: "this software architecture pattern resembles this biological system."
- Insights can be pushed back to relevant instances or stored in a shared insights space at the Router level.
- When a previously-unreachable instance becomes available, the cross-domain Synthesizer prioritizes scanning it for changes.

## Trait Interface

```rust
pub trait Synthesizer {
    fn run_pass(&self, scope: SynthesisScope) -> Result<SynthesisReport, SynthesizerError>;
}

pub struct SynthesisScope {
    pub namespaces: Option<Vec<String>>,   // Limit to specific namespaces (None = all)
    pub min_tier: Tier,                     // Minimum tier to consider
    pub since: Option<DateTime>,            // Only claims modified since this time
    pub max_clusters: usize,                // Maximum clusters to evaluate per pass
}

pub struct SynthesisReport {
    pub claims_examined: usize,
    pub clusters_evaluated: usize,
    pub insights_created: usize,
    pub duration: Duration,
}
```

## Configuration

| Setting | Default | Description |
|---|---|---|
| `llm_provider` | (required) | LLM provider for synthesis |
| `schedule` | `every 6 hours` | How often to run synthesis passes |
| `namespaces` | `*` | Which namespaces to examine (glob pattern) |
| `min_tier` | `task` | Minimum tier to consider for synthesis (skip ephemeral noise) |
| `max_clusters_per_pass` | `50` | Limit on clusters evaluated per pass |
| `min_cluster_size` | `3` | Minimum claims in a cluster for synthesis consideration |
| `enabled` | `true` | Can be disabled entirely for instances where synthesis isn't needed |

## Considerations

**Cost control.** Each synthesis pass involves LLM calls. The schedule and max_clusters_per_pass settings control cost. For instances backed by expensive frontier models, less frequent passes with smaller cluster limits may be appropriate.

**Runaway synthesis.** The Synthesizer must not create unbounded chains of meta-insights. A depth limit on derived_from chains (e.g., maximum 5 levels of derivation) prevents this. Beyond that depth, the insight is likely too abstract to be useful.

**Quality over quantity.** The Synthesizer should produce fewer, higher-quality insights rather than many low-confidence speculative claims. The LLM prompt should emphasize that "no insight" is a valid outcome for a cluster.
