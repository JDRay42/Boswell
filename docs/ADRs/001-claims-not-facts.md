# ADR-001: Claims as the Fundamental Unit, Not Facts

## Status

Accepted

## Context

The system needs a core unit of stored knowledge. The initial framing used "fact" as this unit. During design, it became clear that "fact" implies certainty that the system cannot and should not guarantee.

## Decision

The fundamental unit of knowledge is a **claim** — a statement that the system believes to be true with some degree of confidence. Nothing stored in Boswell is treated as unconditionally true.

## Consequences

- Every claim carries a confidence interval, not a binary true/false state.
- The system never needs to be "wrong" — it has claims with varying confidence that evolve as evidence changes.
- Agents discovering contradictory information create tension in the system rather than error states, which is healthier for swarm scenarios where different agents may legitimately encounter conflicting information.
- Terminology throughout the system uses "claim" consistently.
