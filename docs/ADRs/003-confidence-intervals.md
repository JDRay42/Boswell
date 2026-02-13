# ADR-003: Confidence Intervals Over Single Scores

## Status

Accepted

## Context

The system needs a confidence model for claims. A single float (0.0-1.0) is the simplest representation but collapses distinct epistemic situations to the same value. Example:

- Claim A: one highly credible source, no corroboration. Confidence: 0.8
- Claim B: ten mediocre sources, two weak contradictions. Confidence: 0.8

These are very different situations that an agent would want to reason about differently.

## Decision

Confidence is represented as an **interval** `[lower_bound, upper_bound]` rather than a single score. The interval captures both the estimated truthfulness and the certainty of that estimate.

## Consequences

- A narrow interval means the system has a clear picture. A wide interval means uncertainty.
- Agents can collapse to a single value when they don't need nuance: lower bound for conservative decisions, midpoint for balanced, upper bound for exploratory.
- "Give me claims with wide intervals" becomes a useful query for identifying areas needing investigation.
- The Synthesizer can use interval width as a signal for where to focus effort.
- Derived claims naturally have wider intervals than their parents — uncertainty propagates outward through inference chains, which is epistemically correct.
- The deterministic confidence formula and the LLM-assisted deliberate path both produce intervals rather than points.
- The specific formula for computing intervals is treated as an implementation detail that will evolve. The interface contract (fast, deterministic, based on provenance/temporality/relationships) is the stable surface.
