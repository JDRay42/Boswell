# ADR-016: Two-Tier Topic Classification with Instance Expertise Profiles

## Status

Accepted

## Context

In a multi-instance deployment, agents need to write claims to the correct instance. Without routing intelligence, every agent must understand the instance topology — which instance handles development knowledge, which handles personal knowledge, etc. This is a leaky abstraction that couples agents to deployment details.

## Decision

Each instance registers an **expertise profile** (a set of topic descriptors, keywords, or semantic signatures) with the Router. These profiles are delivered to the client SDK as part of the session response (see ADR-019).

Routing is handled in two tiers:

1. **Client-side (simple matches).** The client SDK matches routing hints or namespace prefixes against expertise profiles received at session start. A routing hint of "coding" matching an instance with expertise "programming" is resolved locally — no network call, no LLM. The client routes the operation directly to the matched instance.

2. **Router fallback (ambiguous cases).** When the SDK cannot confidently match a claim to an instance (no routing hint, ambiguous content, or the claim spans multiple domains), it sends the operation to the Router. The Router's Topic Classifier analyzes the claim's subject, predicate, and raw expression against registered expertise profiles. Classification can be LLM-assisted (more nuanced) or embedding-similarity-based (faster).

Write operations (Assert, Extract, Learn) and read operations (Query, Reflect) accept an optional transient **routing hint** that agents can provide if they know the domain. The hint is carried per-operation (not per-session, since sessions are topology discovery handshakes). The hint is not persisted on the claim — it is a routing instruction only.

## Consequences

- Simple routing happens at the edge with no Router involvement. The Router is only consulted for ambiguous cases.
- Agents interact with the system through the SDK and do not need to know instance topology, count, or specialization.
- Expertise profiles are part of the instance registry, configured during manual instance registration, and delivered to clients at session start.
- Cross-domain claims (relevant to multiple instances) require a configurable routing policy: duplicate, select primary, or flag for user. These always go through the Router since they require policy evaluation.
- The routing hint is transient because the claim's namespace already carries domain context once placed. Two sources of truth about domain classification are avoided.
- The claim's `subject` field (part of the semantic triple) serves a different purpose than the routing hint — `subject` identifies the entity the claim is about, while the routing hint identifies the knowledge domain for placement. These often correlate but are not the same thing.
