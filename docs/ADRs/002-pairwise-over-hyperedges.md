# ADR-002: Pairwise Relationships Over Hyperedges

## Status

Accepted

## Context

Some conclusions are only valid when multiple claims are considered together — "Claims A, B, and C collectively support Claim D, but none of them do individually." This could be modeled as hyperedges (relationships involving three or more claims simultaneously) or as pairwise edges with synthesis.

Hyperedges would make compound relationships a first-class concept in the data model but add significant complexity to storage and graph traversal.

## Decision

The data model uses only **pairwise relationships**. Compound relationships are handled by the **Synthesizer subsystem**, which creates new claims representing emergent ideas and links them back to constituents via individual `derived_from` edges.

## Consequences

- The schema stays simple — relationships are always between exactly two claims.
- The Synthesizer process handles the complexity instead of the schema.
- Abstraction layers emerge organically: first-order claims from sources, second-order from synthesis, third-order from synthesis over synthesis.
- A new claim's confidence interval is naturally bounded by its constituents — uncertainty propagates outward through inference chains.
- This mirrors how human memory consolidation works: insights form during background processing (sleep, reflection), not in real-time during experience.
- The tradeoff is that compound support patterns are not directly queryable from the graph structure; they must be discovered through traversal or synthesis.
