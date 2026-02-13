# Boswell — Confidence Model

Boswell uses confidence intervals rather than single scores. This captures not just "how confident are we?" but "how certain are we about our confidence?" — two fundamentally different epistemic states that collapse into a single number.

## Why Intervals

Consider two claims:

- **Claim A:** One highly credible source, no corroboration, no contradictions. Single-score confidence: 0.8.
- **Claim B:** Ten mediocre sources all agreeing, two weak contradictions. Single-score confidence: 0.8.

As a single float, these are identical. As intervals:

- **Claim A:** `[0.50, 0.95]` — could be very right or somewhat wrong. Thin evidence.
- **Claim B:** `[0.70, 0.85]` — narrower range. Well-established through volume, slightly suppressed by contradictions.

An agent deciding how much to rely on these claims benefits from seeing the difference. Different tasks demand different things:

- "Give me claims where the lower bound is above 0.7" = things we're fairly sure about.
- "Give me claims where the interval is wide" = areas of uncertainty worth investigating.
- "Give me claims where the upper bound is high but the lower bound is low" = potentially important but unverified.

## Stored vs. Computed

- **`base_lower` and `base_upper`**: Set at write time. Derived from the initial provenance entry's confidence contribution. Stored in SQLite.
- **`effective_lower` and `effective_upper`**: Computed at read time from the base interval modified by staleness, corroboration, and contradiction. Cached and invalidated on related changes.

Agents see the effective interval. The base interval is available for debugging and audit.

## Deterministic Formula (Fast Path)

The effective confidence interval is computed without LLM involvement. It is deterministic, cacheable, and fast.

### Step 1: Provenance Aggregation

Each provenance entry contributes to the base confidence. Multiple independent sources narrow the interval (higher lower bound) and push the upper bound toward 1.0.

For a claim with provenance entries having confidence contributions `c₁, c₂, ..., cₙ`:

```
aggregate_upper = 1.0 - ∏(1.0 - cᵢ)    for all i
```

This is the "probability of at least one source being right" model. Each independent corroboration makes it harder for the claim to be wrong. Diminishing returns are built in — three sources saying the same thing is more trustworthy than one, but not three times as trustworthy.

The lower bound is more conservative:

```
aggregate_lower = max(cᵢ) × source_diversity_factor
```

Where `source_diversity_factor` scales from 0.5 (single source) to 1.0 (many independent sources of different types). The lower bound reflects the worst-case credibility — anchored to the strongest single source but boosted by diversity.

**Source diversity calculation:**

```
unique_source_types = count of distinct source_type values in provenance
source_diversity_factor = 0.5 + (0.5 × min(unique_source_types / 3, 1.0))
```

A claim supported by an extraction, an agent assertion, and user input (3 distinct types) gets full diversity credit. A claim with 10 provenance entries all from agent assertions gets partial credit.

### Step 2: Staleness Decay

Once `now() > staleness_at`, both bounds decay using a half-life model:

```
staleness_factor = 0.5 ^ (time_since_staleness_at / half_life)
```

Half-life is configurable per tier (see `07-janitor.md`):

| Tier | Default Half-Life |
|---|---|
| Ephemeral | 4 hours |
| Task | 3 days |
| Project | 4 weeks |
| Persistent | 6 months |

Applied uniformly to both bounds:

```
stale_lower = aggregate_lower × staleness_factor
stale_upper = aggregate_upper × staleness_factor
```

Decay is smooth. No cliffs. A claim one half-life past its staleness date retains 50% of its aggregated confidence. Two half-lives: 25%.

### Step 3: Relationship Adjustment

Walk the claim's relationships and adjust based on supporting and contradicting claims.

**Important:** To avoid circular dependencies, relationship adjustments use the related claims' **provenance-aggregated confidence** (Steps 1-2 only), not their fully-computed effective confidence. A claim's effective score depends on its neighbors' intrinsic strength, not on their own neighbors.

**Support boost:**

```
support_boost = 1.0 + (Σ (supporting_claim.stale_upper × relationship.strength) × BOOST_FACTOR)
```

Where `BOOST_FACTOR` is a tunable constant (default: 0.1). Support modestly widens the upper bound.

**Contradiction penalty:**

```
contradiction_penalty = 1.0 - (Σ (contradicting_claim.stale_upper × relationship.strength) × PENALTY_FACTOR)
```

Where `PENALTY_FACTOR` is a tunable constant (default: 0.2). Contradictions reduce both bounds. Penalty is weighted heavier than boost — it's easier to undermine confidence than to build it.

**Applied to the interval:**

```
adjusted_lower = stale_lower × contradiction_penalty
adjusted_upper = min(stale_upper × support_boost × contradiction_penalty, 1.0)
```

Contradictions always compress the interval. Support only expands the upper bound.

### Step 4: Instance Trust Scaling (Federated Queries Only)

When claims are returned via federated query through the Router, the instance trust score scales the interval:

```
final_lower = adjusted_lower × instance_trust
final_upper = adjusted_upper × instance_trust
```

For direct (non-federated) queries, instance trust is 1.0 (no scaling).

### Complete Formula

```
effective_lower = clamp(aggregate_lower × staleness_factor × contradiction_penalty × instance_trust, 0.0, 1.0)
effective_upper = clamp(aggregate_upper × staleness_factor × support_boost × contradiction_penalty × instance_trust, 0.0, 1.0)

// Ensure lower ≤ upper
if effective_lower > effective_upper:
    effective_lower = effective_upper
```

### Convenience Projections

Agents that don't need interval semantics can collapse to a single value:

- **Midpoint:** `(lower + upper) / 2` — balanced estimate.
- **Conservative:** `lower` — worst-case credibility.
- **Optimistic:** `upper` — best-case credibility.
- **Width:** `upper - lower` — degree of uncertainty.

The API returns the full interval. Projection is the consumer's choice.

## Deliberate Path (LLM-Assisted)

When a Query specifies `deliberate: true`, the fast-path formula is bypassed. Instead:

1. Relevant claims are retrieved using the fast path for initial ranking.
2. The claims, their provenance, relationships, and the query context are sent to the LLM.
3. The LLM evaluates each claim's confidence **in the context of the specific query**.
4. The LLM returns adjusted confidence intervals with reasoning.

The same claim may receive different confidence treatment depending on what the agent is asking. A claim about a software library's API might have high confidence for a "how does this work?" query and lower confidence for a "is this the best approach?" query.

The deliberate path is expensive (LLM call per query) and nondeterministic. Use for high-stakes decisions where nuance matters.

## Confidence Propagation in Synthesis

When the Synthesizer creates a derived claim from constituent claims:

- **Lower bound:** Cannot exceed the minimum lower bound of the constituents. Inference is at most as certain as its weakest foundation.
- **Upper bound:** Bounded by the LLM's assessed confidence in the inference, which should not exceed the maximum upper bound of the constituents.
- **Interval width:** Naturally wider than any constituent. Uncertainty propagates outward through inference chains. A chain of inferences should be less certain than its foundations.

This means third-order abstractions (derived from second-order claims derived from first-order claims) have wide intervals, reflecting genuine epistemic uncertainty about multi-hop inferences.

## Tunable Parameters

| Parameter | Default | Description |
|---|---|---|
| `BOOST_FACTOR` | `0.1` | How much supporting relationships increase the upper bound |
| `PENALTY_FACTOR` | `0.2` | How much contradicting relationships decrease both bounds |
| `source_diversity_max_types` | `3` | Number of distinct source types for full diversity credit |
| `staleness_half_life_*` | (per tier) | Half-life for confidence decay after staleness_at |

These are instance-level configuration. Getting them right requires experimentation with real data. Ship with sensible defaults, iterate.

## Formula Swappability

The confidence formula is implemented behind the same trait interface as everything else in the domain core. The specific formula described here is the v1 implementation. Future versions can swap in alternative formulas (Bayesian updates, Dempster-Shafer theory, fuzzy logic) without changing the API contract or the rest of the system. The interface is: given a claim with its provenance, temporality, and relationships, produce a `[lower, upper]` interval. The math behind that contract is an implementation detail.
