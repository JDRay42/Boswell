# Boswell

Boswell is a cognitive memory system for agents. It remembers two kinds of thing — what is
so, and how to do things — across sessions, and decides what is worth keeping, for how
long, and on whose word.

This file is a glossary and nothing else. Where a concept has several plausible names, one is
chosen and the rest are listed as `_Avoid_`. The design of record for the procedural half is
`docs/architecture/15-procedural-memory.md`.

## Memory

**Claim**:
Something asserted about the world — subject, predicate, object — held with a confidence
interval and never as settled truth. The declarative substrate.
_Avoid_: fact, truth, belief, assertion, triple, memory (as a count noun)

**Procedure**:
A stored how-to: a named, versioned body of instructions carrying the signature — parameters,
preconditions, required tools, postconditions — that says when it applies.
_Avoid_: skill, recipe, playbook, workflow, method, routine

**Goal**:
A navigational node: something an agent might want to accomplish, decomposed into the goals and
procedures that might accomplish it.
_Avoid_: task, objective, intent

**Goal edge**:
One child's placement under one parent goal, carrying what is true of *this placement* rather
than of the child itself. "Eggs on hand" belongs to the procedure; "prefer eggs here when LDL is
low" belongs to the edge.
_Avoid_: link, relation, association

**Decision aid**:
A procedure placed under a goal to help *choose* among that goal's candidates rather than to
accomplish it. Not a separate kind of thing — an ordinary procedure in a different role.
_Avoid_: selector, router, policy, chooser

**Tier**:
How long a memory is meant to last and how widely it is trusted: ephemeral, task, project,
permanent.
_Avoid_: level, scope, lifetime, priority

**Namespace**:
The ownership scope a memory belongs to. Nothing is handed over before scope is checked.
_Avoid_: tenant, partition, owner

**Decay**:
The default fate of a memory nothing uses. Memory fades unless it earns its keep.
_Avoid_: expiry, eviction, garbage collection

**Orphan**:
A goal or procedure left with no live parent after its parent was collected. Still a first-class
memory — findable and re-attachable — just out of the navigable graph.
_Avoid_: dangling node, stray, unreachable

## The reporting loop

The whole loop in one sentence: **the store issues a procedure with an execution receipt; the
executor answers it with an outcome report.**

**Issue**:
What the store does when it hands a procedure over for execution. Issuing creates an obligation.
_Avoid_: dispense, serve, hand out, return, retrieve

"Dispense" was a coinage and was removed: it describes a one-way transaction, which is
backwards for an act that creates an obligation running the other way.

**Execution receipt**:
The obligation created by issuing a procedure — the thing an executor owes an answer to.
_Avoid_: contract, ticket, lease, token

*Why not "contract", since a receipt is proof of something finished and this is an
obligation still owed?* Because "receipt" was already load-bearing — `receipt_id`,
`ReceiptStatus`, `receipt_store.rs`, the database schema and
`POST /v1/receipts/{id}/report` — and renaming it would have cost more than the better word
was worth. Cheap beat correct, on purpose and with eyes open. **Do not rename it back**;
this has been settled once and the argument for "contract" is known and rejected.

Note that "contract" remains correct in Boswell in a *different* sense — the
effectiveness-reporting contract and the `expand` contract in
`docs/architecture/15-procedural-memory.md`, meaning the promise an interface makes. Those
uses are deliberate and should survive any future pass over the word.

**Executor**:
Whoever runs an issued procedure and owes its outcome report. Not necessarily whoever asked for
the procedure.
_Avoid_: caller, client, consumer

**Outcome report**:
An executor's answer to a receipt: what happened, and on failure, whether the procedure or the
executor was at fault.
_Avoid_: result, feedback, response, review

**Effectiveness**:
A procedure's derived success rate over its answered receipts. Silence is not success: a receipt
nobody answers expires as unknown, and counts.
_Avoid_: score, rating, success rate, confidence

**Expand**:
Traversal of a goal into its children, returning the edge-local signals a caller needs to filter
and rank without fetching the children themselves. Expanding is free — it issues no receipt.
_Avoid_: walk, descend, drill down, list children

## Provenance and trust

**Provenance stamp**:
The record attached to every write: who wrote it, on whose behalf, under what authority, on what
evidence, and how well their identity was established.
_Avoid_: signature, audit entry, metadata

**Authority**:
What a principal may do in Boswell — which namespaces, up to which tier, with which operations.
Boswell's own decision, never delegated to the identity system.
_Avoid_: permissions, role, entitlements

**Assurance**:
How well the identity provider actually established who an author is. It caps how high anything
they write may climb, whatever their authority says.
_Avoid_: trust level, identity confidence

**Evidence**:
What a write is founded on — observation, inference, second-hand report, or tool output. Like
assurance, it caps how high the write may climb.
_Avoid_: source, basis, provenance (which is the stamp, not this field)

**Principal**:
Whoever an action is ultimately attributable to. The implementer is the root principal; agents
and subagents act on their behalf and are principals in their own right, each one further from
the root.
_Avoid_: user, account, identity, caller

**Delegation chain**:
The on-behalf-of path from the root principal down to the one that acted. Its *root* is the unit
of independence.
_Avoid_: lineage, call stack, provenance chain

**Grant**:
The implementer's one-time go-ahead, standing for months, from which every credential an agent
holds descends. Distinct from the short-lived thing an agent carries: revoking the grant is how
you cut off everything below it.
_Avoid_: login, session, consent, authorization (which is Authority)

**Attenuation**:
Narrowing a credential when passing it down — fewer namespaces, fewer operations, a lower tier
ceiling, less time. It only ever subtracts, it needs nobody's permission, and the holder of the
narrowed credential cannot widen it back.
_Avoid_: scoping, restriction, downgrade, sub-token

**Endorsement**:
A higher-authority principal's sign-off on someone else's entry, letting it climb a tier it
could not have reached alone.
_Avoid_: approval, vouch, upvote, sign-off

**Corroboration**:
Independent backing for an entry, measured by the diversity of its provenance — distinct
delegation roots — rather than by how many authors it has.
_Avoid_: consensus, agreement, votes, replication

**Delegation root** (the *independence unit*):
The one principal a stamp counts as, for corroboration: the root of its delegation chain,
falling back to its author when it has no chain, with any self-declared subagent path stripped.
It is what a credential resolves to, not what a caller calls itself — subagents of one agent, and
holders of tokens attenuated from one root, are all one delegation root.
`ProvenanceStamp::independence_root` in `boswell-domain` is the definition; everything that
counts independence calls it.
_Avoid_: identity, author, witness, source

**Promotion**:
A tier change. An entry *climbs* on endorsement, corroboration, or effectiveness, and *falls* on
higher-authority contradiction, repeated failure, or staleness. A fall always beats a climb.
_Avoid_: upgrade, boost, demotion

## Who decides

**Gatekeeper**:
The authority that decides what persists and at what tier. Agents advocate; the Gatekeeper
decides.
_Avoid_: validator, policy engine, moderator

**Janitor**:
The background pass that applies decay, expires unanswered receipts, and carries out the
Gatekeeper's verdicts.
_Avoid_: reaper, sweeper, cron job, GC

## Roadmap status

The four words [`docs/development/roadmap.md`](docs/development/roadmap.md) uses to say
where a slice of work stands. They are listed here because the distinction between the last
two is the one that keeps getting lost.

**Shipped**:
On `main`, cited by the PRs that did it.
_Avoid_: done, complete, closed

**In flight**:
Someone is working on it now.
_Avoid_: in progress, WIP, started

**Open**:
Not done, and no decision has been made about it.
_Avoid_: todo, backlog, pending

**Deferred**:
Not done *on purpose*, carrying the reason and what would unblock it. Procedure learning is
deferred, not open: it waits on a corpus of real episodes, and starting it early would
produce a worse inducer rather than an earlier one. Collapsing this into "open" throws away
the reasoning and invites someone to start.
_Avoid_: postponed, later, someday, blocked
