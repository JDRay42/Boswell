# ADR-009: Provenance as Support Network, Not Dependency Tree

## Status

Accepted

## Context

Claims are derived from sources (documents, agent assertions, inference). When a source is invalidated (retracted, corrected, discredited), what happens to claims derived from it?

A dependency tree model would cascade deletion or invalidation — if the source is removed, everything derived from it falls. This is overly aggressive: just because a source document is retracted doesn't mean everything learned from it is incorrect. Other sources may independently corroborate the same claims.

## Decision

Provenance is a **support network**, not a dependency tree. Each claim can have multiple provenance entries, each contributing to the overall confidence. Invalidating a source **reduces confidence proportionally** rather than deleting derived claims.

## Consequences

- A claim with three independent corroborating sources barely flinches when one is invalidated. The remaining two still support it.
- A claim with a single source that gets invalidated sees its confidence interval widen significantly but is not deleted. Other sources may later corroborate it.
- Provenance is modeled as an array on each claim, not a single foreign key. Each entry records source type, source identifier, timestamp, confidence contribution, and context.
- Duplicate assertions (same claim from different sources) add provenance entries rather than creating duplicate claims. This is the corroboration model — analogous to "multiple people saying the same thing" being a signal of collective assessment.
- The corroboration signal must be weighted carefully to avoid confidence bubbles where hallucinated claims reinforce each other across agents. This is a tuning concern, not an architectural one.
