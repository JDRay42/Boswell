# The Change Feed

**Status:** The event log, polling (§4.1) and the MCP `boswell_feed` tool are built;
streaming (§4.2) and the adapters in §5 are design. Status lives in
[`docs/development/roadmap.md`](../development/roadmap.md). The decision is
[ADR-024](../ADRs/024-change-feed.md).

## 1. Context

Agents working a project in parallel need to know what the others have learned and built,
between decisions, without each one re-reading memory to find out. The feed is that signal:
an ordered, filterable log of every change to a memory object, readable by any client that
speaks HTTP and translated for specific harnesses by adapters.

It is one of three parts of keeping a swarm coherent:

| Part | Closes | Mechanism |
|---|---|---|
| **Record** | Work that never enters memory | Write conventions (§6) |
| **Consult** | Building without checking what exists | The dossier ([`17-research.md`](17-research.md)) |
| **Announce** | Two agents starting the same thing at once | The feed, carrying in-progress claims (§6.1) |

## 2. Scope and non-goals

**In scope:** the event log; the HTTP feed and its SSE form; adapters for MCP and Claude
Code; the write conventions that give the feed something to carry.

**Non-goals:**

- **Not a message bus.** Agents do not send each other messages through Boswell. They write
  memory, and the feed reports that memory changed.
- **Not delivery-guaranteed.** The feed is at-least-once within the retention window and
  nothing after it. A consumer that falls behind re-syncs from a dossier.
- **No webhooks yet.** Deferred in ADR-024 pending a decision on outbound requests.

## 3. The event

```
event: { cursor,           // the log's sequence number, as a string
         at,
         namespace,
         kind:   claim | procedure | goal | question,
         entity_id,
         change: created | climbed | fell | contradicted | superseded
               | forgotten | status_changed,
         tier:   { before?, after? },
         status: { before?, after? },          // questions only
         subject?,                             // claims and questions
         summary?,                             // one line; absent once forgotten
         author,
         independence_root }
```

The cursor is a sequence number, not a ULID. Two ULIDs minted in the same millisecond
are not ordered, so a reader holding one could skip an event written just after it. An
`AUTOINCREMENT` sequence only increases and is never reused, even after pruning. Clients
treat it as an opaque string.

`author` is the principal and any agent label on the write that created the object, and is
absent for changes the store makes on its own: Janitor verdicts, collection, expiry.

`summary` is the claim's `raw_expression`, the procedure's or goal's `name` and `intent`, or
the question's `text`, truncated to 160 characters. When an entity is forgotten, its
`forgotten` event carries no summary or subject, and the same transaction strips them from
its earlier events.

Changes that do not alter what a reader would act on — `last_accessed` bumps, confidence
recomputation within a tier, decay that does not cross a tier — emit nothing. A feed that
reported every access would be mostly noise.

### 3.1 Writing events

The store appends the event in the same transaction as the change, in each store's write
path: claims, procedures, goals, questions, receipts that change a question's status, and
every Janitor verdict. Callers never append events themselves; a call site that forgot would
be a silent hole in the feed.

## 4. The core: HTTP

### 4.1 Polling

`GET /v1/feed` takes the `read` operation.

| Parameter | Meaning | Default |
|---|---|---|
| `since` | Cursor: return events after this one | the oldest retained event |
| `from` | `head`: return no events, only the current cursor | — |
| `namespace` | The namespace and its children (`:` or `/`), never a raw prefix | the token's namespace |
| `kinds` | Comma-separated kinds | all |
| `changes` | Comma-separated change types | all |
| `subject` | Exact subject | — |
| `min_tier` | Lowest tier reported | `task` |
| `exclude_self` | Omit events whose author's root is the caller's principal | `true` |
| `limit` | Page size | 100 |

The response is `{ events, next, head }`. `next` is the cursor to send next time. It moves
past every event scanned, including those the filter dropped, so a narrow reader does not
rescan the same rows. `head` is the newest event's cursor. A reader that wants only what
happens from now on starts with `from=head`.

The token's scope filters before anything else: an attenuated token sees only events in
namespaces it may read, whatever `namespace` asks for.

A `since` older than the retention window returns `410 Gone` with `head`. The client
re-syncs by reading a dossier for its subjects, then continues from `head`. The instance
records the highest pruned cursor, so it can tell a stale cursor from a quiet log.

### 4.2 Streaming

The same route with `Accept: text/event-stream` returns server-sent events. Filters are
identical. `Last-Event-ID` is the cursor, so a dropped connection resumes where it stopped
under the standard SSE reconnect. The server sends a comment heartbeat every
`feed.heartbeat_seconds`.

The stream closes when the caller's token expires and rechecks revocation every
`feed.revocation_check_seconds`. Without the recheck, a revoked token would keep reading for
as long as its connection lasted.

### 4.3 Multiple instances

Each instance has its own log and its own cursors. A client talking to several instances
holds one cursor per instance and merges by `at`. The Router does not merge feeds.

## 5. Adapters

Adapters translate the feed for a harness. They hold whatever state the harness needs; the
core holds none.

### 5.1 MCP

`boswell-mcp` exposes `boswell://feed/{namespace}` as a resource. A client that subscribes
receives `notifications/resources/updated` when new events arrive, and reading the resource
returns the events since that subscription's last read. The adapter holds the cursor per
subscription and fills it from the SSE stream.

This is the push channel for any MCP-speaking harness. It needs the MCP server's stdio loop
to send notifications while serving requests, which it cannot do today.

### 5.2 Claude Code hooks

- **`SessionStart`** reads a dossier for the session's subjects and injects it.
- **`UserPromptSubmit`** polls the feed from a cursor stored in a local file and injects a
  digest when anything arrived.
- **`PreToolUse`** on file creation can read a dossier for the thing being created. This is
  the consult step, enforced.

The digest is deterministic: events grouped by kind and change, the questions the agent
raised or took listed first, then up to `digest.max_items` events ordered by tier. No LLM
is involved.

### 5.3 Anything else

A harness with no MCP support and no hooks polls `GET /v1/feed` between steps, or holds an
SSE connection. That is the whole contract.

## 6. Write conventions

The feed can only carry what was written. These conventions are ordinary API writes,
available to every harness. They are predicate conventions, not schema.

### 6.1 Announcing work in progress

Before building something another agent might also build, assert:

```
subject:   <the thing, named as another agent would search for it>
predicate: "is being built"
object:    <a one-line description of its purpose>
tier:      task
ttl:       <expected duration>
```

The claim appears in the feed immediately and in any dossier on that subject. It expires on
its own, so an agent that dies mid-task leaves no permanent false lead. On completion the
agent forgets it, or lets it lapse, and records the component (§6.2).

### 6.2 Recording a component

When a component is finished, write:

- claims for what it is: `(<name>, "is located at", <path>)`,
  `(<name>, "provides", <capability>)`, one claim per capability, so dossiers on the
  capability find it;
- a procedure for using it, placed under the goal it serves, so `expand` on that goal offers
  it.

The claims carry `evidence: observed`, since the author built the thing.

### 6.3 Answered questions

A question's status changes reach the feed as `status_changed` events. An agent that raised
or took a question finds them at the top of its digest. This needs no convention; it is
listed here because it is the event most likely to matter to whoever receives it.

## 7. Configuration

```toml
[feed]
retention_days = 7           # built: pruned on each Janitor sweep
heartbeat_seconds = 30       # design: streaming
revocation_check_seconds = 60  # design: streaming

[feed.digest]
max_items = 20               # design: the hook adapter's digest
```

`retention_days` belongs to the instance config, since pruning runs with the instance's
Janitor and stops if the Janitor is off. The streaming settings will belong to the
gateway, which holds the connections.

## 8. Open problems

1. **Conventions or schema.** §6 relies on agents using the conventions and naming things as
   others would search for them. If the conventions hold up in use, an `Announcement` entity
   with a real `expires_at` and an owner may be worth it. If they don't, schema will not
   rescue them.
2. **Relevance.** Filters are structural: namespace, kind, subject, tier. An agent working on
   date handling would want events *about* date handling, which is a semantic filter over
   summaries. It is cheap to add to polling and expensive to add to streaming, where every
   event would be embedded and compared per connection.
3. **Volume.** A busy swarm writing task-tier claims constantly could outrun any digest.
   `min_tier` and `exclude_self` are the first defenses; real usage will show whether they
   are enough.
4. **Webhooks.** Deferred by ADR-024 on server-side request forgery and outbound access.

## 9. Relationship to existing components

- **Stores** (`04-claim-store`, `15-procedural-memory`, `17-research`) append events in their
  write transactions.
- **Janitor** (`07-janitor`) verdicts — decay across a tier, demotion, collection — append
  events like any other write.
- **Gateway** (`boswell-gateway`, ADR-021) serves the route and applies token scope;
  revocation (ADR-022) bounds streams.
- **Dossier** (`17-research`) is the re-sync path and the consult step.
- **MCP server** (`boswell-mcp`) hosts the resource adapter.
- **Claude Code hooks** (`docs/integrations/claude-code-hooks.md`) host the hook adapter.
