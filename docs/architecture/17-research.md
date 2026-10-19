# Research: Questions and Dossiers

**Status:** Design. Nothing here is built; status lives in
[`docs/development/roadmap.md`](../development/roadmap.md). The decision is
[ADR-023](../ADRs/023-questions-and-dossiers.md).

## 1. Context

Agents researching a problem together need two things from shared memory: everything
already held about a subject, read in one call with its trustworthiness exposed, and a
durable record of what is not yet known, so effort is not repeated and open threads are
not lost when a context window closes.

This document specifies both. The **dossier** is a deterministic, subject-anchored read
across claims, procedures, goals and questions. The **Question** is a new first-class
memory recording an unknown, taken on through the reporting loop that procedures already
use.

## 2. Scope and non-goals

**In scope:** the `Question` entity and its lifecycle; extending receipts to questions;
Janitor detection of gaps; the dossier read; gateway routes for all of it.

**Non-goals:**

- **Not a research agent.** Boswell does not search the web, read documents or answer
  questions. Agents do; Boswell records what they found and what they could not find.
- **Not a coordinator.** The store shows who has taken a question; it never assigns one or
  refuses a second taker. As with `expand`, the store surfaces and the agent decides.
- **No new trust rules.** Answers are ordinary claims and procedures written through the
  ordinary path. The Gatekeeper, corroboration and decay decide what climbs.

## 3. The `Question` entity

| Group | Fields |
|---|---|
| **Identity** | `id` (UUIDv7, as procedures and goals), `namespace` |
| **What is unknown** | `subject`, `predicate?`, `text` (embedded — the semantic match key), `context?` (why it matters), `goal_id?` (the goal it was raised in pursuit of) |
| **Origin** | `raised` by an agent, or `detected` with a `trigger` — `contradiction`, `stale`, `thin`, `empty_goal`, `answer_invalidated` — and a reference to the entity that triggered it |
| **State** | `status`, `demand` (distinct independence roots that raised it), `attempts[]`, `answers[]` |
| **Lifecycle** | `tier`, decay / `stale_at`, provenance stamp (as `15-procedural-memory.md` §5.1) |

`attempts[]` holds every outcome report against the question: its outcome, what was
tried, notes, and the reporter's stamp. `answers[]` holds references to the claims or
procedures that resolve it.

### 3.1 Raising

`POST /v1/questions` is a gatekept write. The entry tier follows the usual rule,
`min(requested, author.max_tier, ceiling(assurance, evidence))`.

**Raising is idempotent by meaning.** If an open question in the same namespace has the
same subject and predicate, or an embedding above `research.duplicate_similarity`, the
store returns that question instead of creating one, and adds the raiser's independence
root to its `demand`. A question raised by three independent roots ranks above one raised
three times by one.

### 3.2 Taking and reporting

`POST /v1/questions/{id}/take` issues a receipt to the caller. Receipts gain a target kind
(`procedure` or `question`); the receipt store, expiry sweep and
`POST /v1/receipts/{receipt_id}/report` are shared.

A question report:

```
report: { receipt_id,
          outcome: answered | partial | unanswerable | abandoned,
          answers?: [ { kind: claim | procedure, id } ],
          tried?: [ string ],
          notes? }
```

- `answered` requires at least one entry in `answers`, each one existing and readable by
  the reporter. Writing the answer is a separate, ordinary write; the report only links it.
- `partial` requires `answers` and `tried`.
- `unanswerable` requires `tried`. An empty search with no record of where it looked is
  worthless to the next agent.
- An expired receipt records `unknown` against the taker, as for procedures. It counts
  against the taker's reliability, not against the question.

Several receipts may be live against one question. Independent answers are what
corroboration needs, and the dossier shows pending receipts so a second agent can decide
whether to join.

### 3.3 Status

```
          take                    report: answered
  open ──────────▶ taken ───────────────────────────▶ answered
   ▲  ▲              │ report: unanswerable               │
   │  │              ▼                                    │ answer contradicted,
   │  └──────── unanswerable                              │ demoted or forgotten
   │   verdict decays                                     │
   └──────────────────────────────────────────────────────┘
```

- `taken` means at least one receipt is pending. When the last pending receipt is
  reported `abandoned` or expires, the question returns to `open`.
- A report that arrives after the question is `answered` is still recorded in `attempts`.
  A second independent `answered` report is how an answer gains corroboration.
- `unanswerable` is a verdict held like a claim: it has a `stale_at`, and each further
  independent root reporting `unanswerable` extends it. When it lapses the question reopens,
  carrying every attempt, so the next agent starts from what failed.
- `withdrawn` is set by the raiser or a higher-authority principal when a question no longer
  matters. A withdrawn question is not deleted; it decays.

`answered` records that someone answered, not that the answer is settled. The dossier
reports the answer's own standing (§5.2).

### 3.4 Tier and decay

An open question nobody raises again, takes or reports on decays like any other memory.
Each raise, take and report reinforces it. A question climbs a tier when its `demand`
reaches `research.demand_to_climb` distinct roots, or on endorsement. This keeps a
long-standing unknown that many agents keep hitting from fading at task tier.

## 4. Detection

A Janitor pass records gaps as `detected` questions. Each rule is idempotent on its
trigger reference, so a gap is recorded once however many sweeps see it.

| Trigger | Condition | Default |
|---|---|---|
| `contradiction` | A `Contradicts` relationship between active claims, unresolved for longer than `research.contradiction_grace` | 7 days |
| `stale` | A claim at `project` tier or above past its `staleness_at` | on |
| `thin` | A claim at `project` tier or above backed by one independence root (reachable by endorsement) | off |
| `empty_goal` | A goal with no `accomplish` child, or none with effectiveness above `research.min_effectiveness` | on |
| `answer_invalidated` | An `answered` question whose answer was contradicted, demoted or forgotten; reopens the question rather than creating one | on |

Detected questions enter at the tier of the entity that triggered them, capped at
`project`. Their provenance names the Janitor as author.

Detection never happens on read. A dossier that wrote questions would put a write behind
a read scope.

## 5. The dossier

### 5.1 Request

`POST /v1/dossier` takes the `read` operation.

```
{ subject?: string,          // exact anchor
  topic?: string,            // semantic anchor, embedded at query time
  namespace: string,         // exact, "ns/*", or "ns/*/n", as Query
  view?: width | depth,      // default width (§5.3)
  predicate?: string,        // narrow to one group; the usual follow-up to a width read
  hops?: int,                // relationship hops from anchored claims; default 1, max 3
  kinds?: [claim | procedure | goal | question],   // default all
  limit_per_kind?: int,      // default 50
  min_similarity?: float }
```

At least one of `subject` and `topic` is required. With both, the anchor set is the union.

### 5.2 Response

```
{ anchors:    [ { subject, matched_by: exact | semantic, score? } ],
  groups:     [ { subject, predicate,
                  standing: contested | stale | thin | settled,
                  roots: [independence_root], unattributed_count,
                  claims: [ { claim_id, object, confidence, tier, source_type,
                              authors: [ { author, independence_root } ],
                              stale_at } ],
                  contradictions: [ { claim_id, against } ] } ],
  procedures: [ { id, name, version, intent, effectiveness,
                  success_count, failure_count, unknown_count, tier } ],
  goals:      [ { id, name, intent, accomplish_children, best_child_effectiveness } ],
  questions:  [ { id, text, status, demand, origin,
                  pending: [ { author, expires_at } ],
                  attempts: [ { outcome, tried, author } ],
                  answers } ],
  coverage:   { intent_match: semantic | substring, truncated: [kind],
                unavailable: [kind] } }
```

`unavailable` lists kinds this instance cannot serve yet — `question`, until the `Question`
store exists.

**Grouping and standing.** Claims are grouped by `(subject, predicate)`. A group's standing
is the first of these that holds:

1. **contested** — a claim in the group has an unresolved `Contradicts` relationship.
2. **stale** — every claim in the group is past `staleness_at`.
3. **thin** — the group's claims span fewer than `research.min_roots_settled` distinct
   independence roots (default 2).
4. **settled** — otherwise.

Distinct objects under one predicate are not a contest; many predicates are multi-valued.
Only a recorded contradiction is.

**Attribution and independence.** Each claim shows its `authors` and their
`independence_root`s. A group's `roots` counts the latter. Five agents of one grant agreeing
show five authors and one root, and the group reads `thin`. A claim written before claim
writes were stamped has no author; it counts toward `unattributed_count` and toward no root.

An author is the authenticated principal, optionally followed by a self-declared agent
label: `writer/sub:explore-3`. The label comes from the `X-Boswell-Agent` request header and
is never trusted — `authenticated_principal` strips it wherever independence is counted —
so a caller can only mislabel which of its own agents wrote something.

**Procedures and goals** join by intent match against `topic`, or against `subject` when no
topic is given. Procedures appear as signatures with effectiveness; bodies are not
returned, and no receipt is issued, as with `expand`. To execute one, the agent retrieves it
through `GET /v1/procedures/{id}`, which issues the receipt.

**Questions** join by subject match and by embedding. Answered questions appear with their
answers; the standing of the group those answers fall in tells the reader whether the
answer has been corroborated.

### 5.3 Width and depth

A team of agents needs a view a mile wide and an inch deep: enough to know what exists,
what is in dispute and what is open, without paying for the detail of every claim. The
default `view: width` returns the same structure with the detail removed:

- each group carries `subject`, `predicate`, `standing`, `claim_count`, `root_count`,
  `unattributed_count`, and `lead` — the object of its highest-confidence claim — but no
  `claims` or `contradictions`;
- each procedure carries `id`, `name`, `intent` and `effectiveness`;
- each goal carries `id`, `name` and `accomplish_children`;
- each question carries `id`, `text`, `status` and `demand`.

Going deeper is a second call. `view: depth` with the same anchor returns everything in
§5.2; adding `predicate` narrows it to one group. Procedures and goals have their own
routes for depth: `GET /v1/procedures/{id}` (which issues a receipt) and
`GET /v1/goals/{id}/expand`.

The vocabulary matches OpenRig's context packs, whose atoms are labeled
`purpose: width | depth`; see [`docs/integrations/openrig.md`](../integrations/openrig.md).

### 5.4 Narrative

`POST /v1/dossier/narrate` takes the same request, builds the dossier, and asks the
configured LLM provider for a summary that cites claim, procedure and question ids. The
narrative is not stored. It is an option for a human reader or a final report; agents
working the problem should read the dossier. It ships after the dossier, and it
supersedes `Reflect` in `03-api-surface.md`.

## 6. Routes

| Route | Operation | Purpose |
|---|---|---|
| `POST /v1/questions` | write | Raise a question (idempotent by meaning) |
| `GET /v1/questions` | read | List by namespace, subject, status, origin |
| `GET /v1/questions/{id}` | read | Fetch one question with its attempts |
| `POST /v1/questions/{id}/take` | write | Take a question on; issues a receipt |
| `POST /v1/questions/{id}/withdraw` | write | Withdraw a question |
| `POST /v1/receipts/{receipt_id}/report` | write | Existing; now accepts question outcomes |
| `POST /v1/dossier` | read | The research read |
| `POST /v1/dossier/narrate` | read | Optional LLM summary of a dossier |

All writes set `author` from the authenticated principal, never from the request body.

## 7. Configuration

```toml
[research]
duplicate_similarity = 0.92
demand_to_climb = 3
min_roots_settled = 2
min_effectiveness = 0.5
contradiction_grace_days = 7
detect_thin = false
question_receipt_ttl_hours = 24
```

## 8. Open problems

1. **Subject identity.** Subjects are free strings, so "Postgres" and "PostgreSQL" anchor
   apart. The semantic anchor mitigates this; nothing canonicalizes subjects, and this
   design does not try.
2. **Source independence.** Independence stops at the delegation root. Two agents with
   separate grants who read the same blog post are still one witness, and Boswell cannot
   tell. Evidence carries a type, not a source identity. Recording source identity on
   `observed` and `reported` evidence would let corroboration count sources; it is a change
   to the provenance stamp and belongs in its own ADR.
3. **Detection thresholds.** The defaults in §4 are guesses. `thin` is off because it is
   likely to be noisy, and `contradiction_grace` exists so routine disagreement does not
   flood the question store. Both want tuning against real research sessions.
4. **Federation.** A dossier is single-instance. A federated dossier through the Router
   follows whatever federated `Query` becomes (`09-router.md`).

## 9. Relationship to existing components

- **Claims** (`02-claim-model`, `04-claim-store`) supply the groups; contradiction
  relationships supply `contested`.
- **Procedures and goals** (`15-procedural-memory`) join by intent and contribute
  effectiveness. The dossier is `expand`'s sibling: deterministic, free and receipt-less.
- **Receipts** (`15-procedural-memory` §3.3) generalize to take a question as their target.
- **Janitor** (`07-janitor`) gains the detection pass and the `unanswerable` lapse.
- **Gatekeeper** (`08-gatekeeper`) gates question writes exactly as other writes.
- **Provenance** (`CONTEXT.md`, ADR-022) supplies `author` for attribution and
  `independence_root` for counting.
- **Change feed** (`18-change-feed`) carries question status changes, and uses the dossier
  as its re-sync path.
- **`Reflect`** (`03-api-surface`) is superseded by §5.4.
