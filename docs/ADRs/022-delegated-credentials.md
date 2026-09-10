# ADR-022: Delegated Credentials — OIDC at the Human Edge, Attenuable Tokens Below

## Status

Accepted.

## Context

Boswell is a memory server that agents reach on the implementer's behalf. Four constraints came
out of the design session, and they are the whole problem:

1. **The principal is a person.** Agents are delegates. Access means proving delegation, not
   proving identity.
2. **The grant is long-lived.** The implementer authorizes once and agents run for weeks.
   Re-authenticating daily, or every few days, is a failure.
3. **Verification is local and fast.** Task-tier memories are written and read constantly. A
   network round trip to an identity provider per request is not affordable. Signature
   verification is — Ed25519 costs tens of microseconds against a SQLite write and an embedding
   call measured in tens of milliseconds.
4. **Delegation is transitive.** An agent spins up subagents and grants them access. The
   subagent must be able to receive *less* authority than its parent, and must not be able to
   widen it back.

Deployment spans a local machine and a public host, so authentication at the boundary is
required rather than optional ([ADR-021](021-gateway-is-the-security-boundary.md)).

Constraint 4 is the one that eliminates the obvious answers. OAuth scopes do not narrow
transitively without the issuer in the loop, and putting the issuer in the loop for every
subagent spawn violates 2 and 3.

## Decision

**Two mechanisms, split at the human boundary.**

**Above the line — OIDC establishes the person.** The implementer authenticates once to an
external identity provider using the device-code grant, which needs no browser on the agent's
side. The resulting refresh credential is the go-ahead, and it lasts months. Boswell never
handles a password and never manages human accounts. Boswell ships no identity provider;
[Pocket ID](https://pocket-id.org) is a reasonable local choice and remains the operator's to
run and to govern.

**Below the line — attenuable tokens carry delegation.** The gateway trades a verified OIDC
identity for a **root token** scoped to that principal's namespaces, tiers and operations. The
agent holds it. When the agent spawns a subagent it **attenuates the token locally** — narrowing
namespace, operations, tier ceiling, or lifetime — with no network call and no issuer in the
loop. The narrowing is enforced cryptographically: a holder can only ever remove authority.

The mechanism is [biscuit](https://www.biscuitsec.org/) (`biscuit-auth`), whose properties are
the requirements restated: offline attenuation, and verification that needs only a public key.
That second property is what keeps federation open — an instance can verify a token it could
never have minted.

**Authority remains Boswell's.** As [`CONTEXT.md`](../../CONTEXT.md) already has it, what a
principal may do is Boswell's decision and is never delegated to the identity system. A token
carries a claim about delegation; the Gatekeeper still decides what may be written.

## Alternatives

**OIDC alone, no second mechanism.** Meets constraints 1, 2 and 3 cleanly. Fails 4: an agent
cannot narrow a token by itself, so every subagent spawn is a round trip to the identity
provider using a token-exchange feature most providers do not implement. In practice the parent
hands the subagent its own full token, which is the failure this ADR exists to prevent.

**Attenuable tokens alone, no identity provider.** Meets 3 and 4, and 2 with a long-lived root.
Fails the first step: something still has to establish that the person is who they say before
the first token is minted, and building that means Boswell manages human accounts and
credentials. That is a larger security surface than running an off-the-shelf provider.

**Macaroons rather than biscuits.** The same attenuation model, and the older, better-known one.
Rejected on verification: macaroons chain HMACs, so anything that can verify can also mint. With
one instance that is tolerable; it makes federation a redesign rather than an extension. The
Rust ecosystem also favours biscuit, whose reference implementation is Rust.

**Signed JWTs minted by Boswell**, as the Router already does. Simple and fast, but a JWT is a
fixed set of claims — there is no attenuation, so it fails 4 for the same reason as OIDC alone.

## Consequences

- The Pocket ID groundwork is load-bearing rather than incidental.
- **Revocation needs building.** Attenuated tokens are verified offline, so a revoked root is
  invisible until something checks. The gateway needs a revocation list keyed by token
  identifier, and every token needs a bounded lifetime so the list stays small. Long grant and
  fast revocation are only compatible if they are separate mechanisms.
- **Biscuit policies are Datalog.** A small dialect, but a real learning curve, and authorization
  logic written in it is not reviewable by reading Rust.
- **Corroboration must follow the delegation chain, not the caller.** `CONTEXT.md` already says
  the *root* of a delegation chain is the unit of independence, and #33 made corroboration count
  the authenticated principal. Once subagents hold their own tokens, the authenticated principal
  is the subagent — so ten subagents of one agent would read as ten independent witnesses unless
  corroboration resolves each token to its delegation root. The model already says what to do;
  the code will need to catch up when subagents exist.
- **Session stamping becomes reachable.** `min_distinct_sessions` and
  `min_distinct_evidence_types` default to 0 because writes are minted in-process with
  `session_id: None` (#35). A token-carrying transport can stamp sessions, which is what those
  settings were waiting for.
- Two mechanisms is more moving parts than one. The split is at the human boundary, which is
  where the requirements genuinely differ: identity is a person problem and delegation is a
  machine problem.
- Nothing here is built yet. This ADR records the decision; `docs/development/roadmap.md`
  records what exists.
