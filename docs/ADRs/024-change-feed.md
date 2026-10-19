# ADR-024: A Change Feed over Memory, Neutral to the Agent Harness

## Status

Accepted. Design of record is [`18-change-feed.md`](../architecture/18-change-feed.md).
Nothing here is built yet; [`docs/development/roadmap.md`](../development/roadmap.md)
records what exists.

## Context

Agents working one project in parallel duplicate each other's work. Agent A builds a
component; agent B, which needs one like it, never learns it exists and builds its own. The
pattern was observed in agent swarms generally, not in any one harness.

Three things cause it, and they need different fixes:

1. **Nothing is recorded.** A's component never enters shared memory, so no read can find it.
2. **Nothing is consulted.** B decides to build without asking what exists. The dossier
   ([ADR-023](023-questions-and-dossiers.md)) answers that question once asked; it does not
   make anyone ask.
3. **Nothing is announced.** A and B start the same thing at the same moment. Neither has
   finished, so neither's work is in memory to be found.

A change feed — "here is what memory learned since you last looked" — addresses the third
directly and the second partly, by putting relevant news in front of an agent between
decisions. It does nothing for the first, which is a write-side convention.

Boswell is meant to serve any agent harness. Claude Code is the expected first user, but a
mechanism that only works through Claude Code hooks fails that intent.

## Decision

**1. An append-only event log is the source of the feed.** Every change to a memory object
— created, climbed, fell, contradicted, superseded, forgotten, question status changed —
appends an event in the same transaction as the change. The feed is never reconstructed from
current state: Forget is a real delete and tier changes leave no row, so current state cannot
say what happened.

**2. Events are references, not copies.** An event carries the entity's id, kind, namespace,
tier movement, author, independence root and a one-line summary. The consumer fetches the
entity if it wants it. Forgetting an entity also strips the summaries from its earlier
events, so the log never preserves what Forget removed.

**3. The core is HTTP, pull first.** `GET /v1/feed?since=<cursor>` returns the events after
the cursor, filtered by namespace, kind, subject and tier, and scoped by the caller's token
like any other read. The cursor is the last event's ULID and the client holds it, which keeps
sessions stateless (ADR-019).

**4. Server-sent events on the same route** for clients that want live delivery: the same
events, the same filters, with `Last-Event-ID` as the cursor. Polling and streaming cannot
disagree, because they are one query.

**5. Harnesses get adapters, not features.** MCP resource subscriptions, Claude Code hooks
and any future harness integration sit at the edge and translate the feed. No memory
behavior depends on which adapter is in use.

**6. Write-side conventions are ordinary API writes.** A finished component is recorded as
claims plus a procedure for using it; work in progress is announced as a short-lived
task-tier claim. Both go through the existing write path, so no harness depends on
transcript extraction to capture them.

## Alternatives

**Periodic dossier refresh.** Each agent re-reads a dossier on a timer and diffs it. Works
with nothing new built. Rejected as the mechanism because the cost scales with agents times
subjects times frequency, it cannot report deletions or demotions, and the diff lands in the
agent's context whether anything changed or not. Kept as the re-sync path when a cursor has
aged out.

**A feed derived from `updated_at` and ULID order.** No new table. Rejected for the reason in
decision 1: it cannot report a forget or a fall.

**Push to registered webhooks.** The most harness-neutral delivery of all. Deferred, not
rejected: Boswell calling URLs that callers register is a server-side request forgery
surface and an outbound path past the gateway, which ADR-021 makes the security boundary.
It needs its own decision on allowlists and signing.

**Claude Code hooks as the delivery mechanism.** Simplest for the expected first user.
Rejected as the core because it confines the feature to one harness. Kept as an adapter.

**Full entities in events.** Saves the consumer a fetch. Rejected because it doubles storage,
keeps forgotten content alive in the log, and widens what a feed reader sees beyond what a
fetch would authorize at read time.

## Consequences

- **A new table and a write on every change.** Each write path — claims, procedures, goals,
  questions, and every Janitor verdict — appends an event in its transaction. Missing one is
  a silent gap in the feed, so the append belongs in the store layer, not in callers.
- **Retention is bounded.** Events older than `feed.retention_days` are dropped. A cursor
  older than the window gets `410 Gone`, and the client re-syncs from a dossier.
- **The first server push in Boswell.** ADR-019 says there is no push for topology; this does
  not change that, but SSE is a long-lived connection through the gateway. A stream must close
  when its token expires and must recheck revocation (ADR-022) on an interval, or a revoked
  token keeps reading.
- **`boswell-mcp` needs concurrency.** Its stdio loop answers requests one at a time and
  exposes only tools. Resource subscriptions need it to send notifications unprompted while
  serving requests.
- **The feed is per instance.** With several instances, the SDK holds one cursor per instance
  and merges by time. The Router does not merge feeds.
- **Conventions are not schema.** The in-progress announcement and the component record are
  predicate conventions, and an agent that ignores them is invisible to the others. Whether
  they should become schema is left open until they have been used.
