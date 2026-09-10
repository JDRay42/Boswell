# ADR-021: The Gateway Is the Security Boundary

## Status

Accepted. Supersedes [ADR-017](017-instance-level-security.md).

## Context

[ADR-017](017-instance-level-security.md) decided that security is enforced at the instance:
every Boswell instance would require mTLS on every inbound connection, with no unauthenticated
access modes, and each instance would be its own security boundary. The matching design is
[`10-security.md`](../architecture/10-security.md), which specifies mTLS everywhere, per-instance
short-lived tokens issued by the Router, and an `age`-encrypted Router config.

None of it was built. `boswell-grpc` checks that `auth_token` is non-empty at fourteen call
sites and does nothing else — no signature verification anywhere, so any process that can reach
the port can write to any tier by sending the string `"x"`. The Router mints a properly signed
JWT and the SDK carries it on every call; the instance never reads it. `enable_tls` refuses to
start rather than serve plaintext under a TLS banner (#36). Meanwhile the README instructs
operators to keep the instance on `127.0.0.1` *because* it does not authenticate.

So two documents of record contradicted each other, and the code followed neither. #36 stopped
the code from claiming security it does not provide; this ADR does the same for the design.

Meanwhile a second transport grew. `boswell-gateway` authenticates for real — SHA-256-hashed
bearer keys, per-key scopes, rate limits, namespace isolation — and its authenticated `/v1`
surface is a **superset** of the gRPC service: every gRPC method has a route in front of it,
plus `/v1/recall` and `/v1/hooks/ingest`, which gRPC does not have. The system had already
chosen a boundary in practice. Nobody had written it down.

## Decision

**The HTTP gateway is Boswell's security boundary. The gRPC instance sits inside it.**

- The gateway is the only component that faces a network and the only one that authenticates.
- The gRPC instance binds to loopback **by construction, not by recommendation**. A
  non-loopback bind is a startup error, in the same spirit as `enable_tls` refusing to serve
  plaintext.
- The `auth_token` field and its fourteen `is_empty()` checks are deleted rather than left
  looking functional. A check that accepts `"x"` is not weak authentication; it is the
  appearance of authentication, which is worse.
- gRPC stays as the internal protocol. It earns nothing on a single-host deployment, but
  removing it forecloses the Router path (see Consequences), and leaving it costs nothing once
  it is honestly labelled as inside the boundary.
- On a remote host — a droplet — the public surface is the gateway. The instance stays on that
  host's loopback.

ADR-017's *requirement* survives: with a publicly reachable deployment there are no
unauthenticated access modes at the boundary. Its *mechanism* does not. Manual per-client
certificate registration cannot survive agents that spin up subagents, because an agent cannot
issue a certificate to a subagent without the operator becoming a certificate authority. See
[ADR-022](022-delegated-credentials.md).

## Alternatives

**Finish ADR-017 as written — mTLS on the instance, certificate registration per client.**
Rejected. It commits Boswell to running a PKI, and the registration workflow is manual by
design, which is exactly wrong for credentials that agents mint for subagents at runtime.

**Authenticate both transports.** Rejected. Two authentication systems is two things to keep
correct forever, for a second door nothing outside the host uses.

**Delete gRPC entirely and have the gateway call the store in-process.** Tempting — it removes
a serialization round trip from the hot path — but it forecloses the Router, which is on the
aspirational backlog rather than abandoned. Deferred, not rejected.

## Consequences

- The security work has **one surface** to get right instead of two.
- Anything that can reach the instance's loopback port has full write access to every tier.
  On a single-tenant host that is the accepted trade; on a shared host it is not, and this ADR
  does not make that deployment safe.
- Federation is not foreclosed. What would foreclose it is credentials only the minting instance
  can interpret — addressed in [ADR-022](022-delegated-credentials.md) — not the choice of
  transport.
- [`10-security.md`](../architecture/10-security.md) now describes a model the project has
  rejected. It carries a superseding note and needs a rewrite; the roadmap tracks that.
- Nothing here is built yet. This ADR records the decision; `docs/development/roadmap.md`
  records what exists.
