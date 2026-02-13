# ADR-007: Hybrid Confidence Computation (Deterministic Default, LLM On Demand)

## Status

Accepted

## Context

The effective confidence of a claim depends on its provenance, staleness, and relationships with other claims. Three computation approaches were evaluated:

1. **Purely deterministic**: A formula that aggregates inputs and produces a score. Fast, predictable, cacheable. Cannot capture nuance like "this source is generally reliable but on a topic outside their expertise."
2. **Purely LLM-assisted**: An LLM evaluates the claim in context. Richer assessment but introduces latency, cost, and nondeterminism. Two identical queries might get slightly different scores.
3. **Hybrid**: Deterministic by default, LLM-assisted on demand and via periodic Janitor sweeps.

## Decision

**Hybrid.** The fast path uses a deterministic formula, cached and invalidated on related claim changes. The deliberate path, activated by a parameter on the Query operation, triggers an LLM evaluation before returning results. The Janitor periodically runs LLM-assisted re-evaluation on claims in interesting states (heavily contradicted, frequently accessed, recently challenged) and bakes results into updated confidence values.

## Consequences

- Default reads are sub-millisecond with deterministic, reproducible scores.
- Agents can opt into deeper evaluation when making consequential decisions: "take your time, think about this."
- The deliberate path is **query-contextual** — the same claim may receive different confidence treatment depending on what the agent is asking about. This is more powerful than static re-evaluation.
- The deliberate path can return a reasoning narrative alongside scored claims, giving the agent not just numbers but insight into weak spots and contradictions.
- The Janitor folds LLM judgment into stored values asynchronously, improving the fast path over time without adding latency to reads.
- The specific deterministic formula is an implementation detail that will evolve. The interface contract is stable.
