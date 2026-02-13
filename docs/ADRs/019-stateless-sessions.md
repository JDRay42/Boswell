# ADR-019: Stateless Sessions with Topology Discovery

## Status

Accepted

## Context

The initial session design included namespace and routing hint in the session request, binding the client to a specific instance for the duration of the session. A subsequent revision made sessions stateless but kept the Router in the hot path for every operation — the client sent all operations to the Router, which classified and forwarded each one.

This had two problems:

1. The Router became a throughput bottleneck and single point of failure for all operations.
2. The client had no visibility into the topology, making it unable to make intelligent routing decisions or handle instance failures independently.

## Decision

**Sessions are topology discovery handshakes.** The session request is empty beyond mTLS identity. The response provides a token, a mode indicator ("router" or "instance"), and an instances array containing each instance's endpoint, expertise profile, and health state.

**The client SDK routes operations directly to instances.** Simple routing (matching routing hints or namespace prefixes against expertise profiles) is resolved locally in the SDK. The Router is a fallback for ambiguous cases and federated queries, not the default path for every operation.

**The client is responsible for handling instance failures.** If an instance is unreachable, the client retries. If retries persistently fail, the client can re-fetch the topology from the Router. There is no push-based topology update mechanism.

## Consequences

- The session handshake is simple: authenticate, receive topology. The same interface works for Routers and bare instances.
- The Router is out of the hot path for most operations. It handles session initiation, ambiguous routing fallback, and federated queries — not routine reads and writes.
- Agents can freely mix domains within a single session. The SDK routes each operation to the correct instance based on the topology received at session start.
- The client SDK takes on routing responsibility for simple cases. This moves intelligence to the edge and reduces Router load.
- Topology can become stale during long sessions. The client handles this through retry-and-refresh rather than push notifications or polling.
- Single-instance mode returns a one-element instances array. The SDK has one code path regardless of deployment mode.
