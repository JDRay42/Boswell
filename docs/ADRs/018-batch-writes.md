# ADR-018: Batch Writes with Partial Success and Cross-Domain Fan-Out

## Status

Accepted

## Context

Write operations (Assert, Learn) need to handle batches efficiently. An agent completing a task may produce many claims at once, and single-claim-per-call introduces significant network overhead. In multi-instance deployments, a batch may contain claims spanning multiple domains that need to be routed to different instances.

Two sub-questions:

1. Should batches be atomic (all-or-nothing) across instances?
2. How should the client handle batches that span multiple instances?

## Decision

**Batches are supported on all write operations.** Assert and Learn accept one or more claims per call.

**Cross-domain batches are routed by the client SDK.** The SDK receives instance topology (endpoints and expertise profiles) at session start, groups claims by target instance locally, and sends sub-batches directly to each instance in parallel. For claims the SDK cannot confidently route, it falls back to the Router for classification.

**Batches are not atomic across instances.** Each sub-batch succeeds or fails independently. The client receives per-claim status reporting. Partial success is a normal outcome when an instance is unreachable.

## Consequences

- Agents submit batches without worrying about domain routing. The client SDK handles classification and fan-out using the topology received at session start.
- Network overhead is minimized — one round trip per instance for a batch instead of N round trips for N claims. Cross-domain batches are grouped and sent in parallel to each target instance.
- No distributed transactions. This avoids significant complexity with minimal real-world downside, since agents can retry individual failed claims.
- In single-instance mode, all batches are accepted regardless of domain mix — there is only one instance.
- The per-claim result reporting gives agents full visibility into what succeeded and what didn't, enabling informed retry decisions. The client is responsible for retrying connections to unreachable instances.
- The Router is only consulted for claims the SDK cannot route confidently, keeping it out of the hot path for routine writes.
