# ADR-008: Gatekeeper Pattern for Tier Promotion

## Status

Accepted

## Context

When agents complete tasks, they produce claims representing "things learned." The question is who decides which of these claims deserve promotion to higher tiers (longer persistence, broader visibility).

Options:

1. **Agent decides**: The producing agent determines the tier. Simple but dangerous — agents lack broader context and could pollute long-term memory with noise or hallucinated claims.
2. **Rules-based**: Automatic promotion based on access count, reference count, or other metrics. Objective but cannot evaluate semantic quality, redundancy, or contradiction with existing knowledge.
3. **Gatekeeper (LLM-backed evaluator)**: A subsystem with broader context that evaluates each claim against existing knowledge at the target tier.

## Decision

**Gatekeeper pattern.** Agents can advocate for promotion (via an advocacy tuple expressing perceived importance and confidence) but cannot unilaterally promote. The Gatekeeper evaluates claims against existing knowledge, considers the agent's advocacy as a signal, and makes an independent judgment.

## Consequences

- No edge agent has unilateral authority over long-term memory. This prevents noise accumulation, confidence bubbles, and swarm groupthink.
- Gatekeepers exist at every tier boundary, with skepticism increasing at higher tiers. The ephemeral → task Gatekeeper can be permissive; the project → persistent Gatekeeper should be stringent.
- Different Gatekeepers can use different LLM configurations matching their criticality (fast local model for low-stakes transitions, frontier model for consequential ones).
- The Gatekeeper's reasoning is stored as provenance on the claim, providing context for future evaluations.
- Rejected claims are not deleted — they remain at their current tier with their existing TTL. If new evidence surfaces, the Gatekeeper has context from the prior evaluation.
- The advocacy tuple `[perceived_importance, confidence]` lets agents express nuanced opinions that the Gatekeeper considers but isn't bound by.
