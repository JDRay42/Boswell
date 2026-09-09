# Procedural & Goal Memory for Agent Teams

**Status:** Phases 1–6 of §9 are implemented and merged, plus phase 7a (below).
Sections describing those parts describe shipped code; §8 (open problems) and
procedure/goal *learning* remain design.

## 1. Context & motivation

Boswell today stores **declarative** memory: `Claim`s — `(subject, predicate, object)`
triples with a confidence interval, tier, and `source_type`. That answers *what is true*.
It does not store *how to do things* or *how work decomposes*.

Boswell's real users are **agents** acting for humans — often a hierarchy of subagents
acting on behalf of subagents on behalf of an agent on behalf of a human. Two needs fall
out of that which claims alone cannot serve:

1. **Procedural memory** — a personalized, refined *how-to* (a technique), with its
   control flow intact, that an agent preserves and reuses across sessions rather than
   re-deriving each time.
2. **Goal memory** — how a high-level goal decomposes into sub-goals and, eventually,
   procedures, so an agent can descend from "I need to X" to an executable step, choosing
   contextually at each level.

And because the users are a *team* of agents with differing trustworthiness, memory must
carry **provenance** and must not let a confused or adversarial subagent poison shared
long-term memory.

### Inspiration: OaK, and where we diverge

This design was prompted by **OaK (Ontology-as-a-Kernel)** — *"Toward Effective and
Reliable LLM Agents via Dynamic Ontology"*, Zhang et al., arXiv:2608.22974. OaK packages
a task interface as a kernel `K = (S, F)`: a schema `S` of typed concepts/relations, and
typed functions `F` (retrieval, filtering, traversal, projection, aggregation) over a
schema-guided knowledge graph. A ReAct agent selects a function and binds typed arguments;
the kernel is a "semantic and procedural contract" bounding what the agent may do.

We keep OaK's central split — declarative structure vs. typed procedures — but diverge on
two points:

- **OaK freezes** a per-task kernel after refinement. We want procedures that **outlive
  their task and decay organically** (Boswell's model), reused when a *schema-compatible*
  task recurs. Reuse-safety is gated by **precondition/signature match**, not by "it was
  validated on this exact task."
- OaK's kernel is oriented to a single agent solving one task. We extend to **a team of
  agents** with a trust gradient over shared memory.

## 2. Scope & non-goals

**In scope (this design):** two new first-class entities (`Goal`, `Procedure`); a
stateless, agent-driven retrieval/traversal model; a provenance-stamped write path with
gatekept tier promotion; an `IdentityProvider` port; and an optional, repo-included
**devAuth** subsystem for local bring-up.

**Non-goals:**
- **Not a planner or decider.** The store *surfaces* candidates and factors; the agent
  decides. Decision knowledge itself is stored as a retrievable procedure, never as a
  hardcoded policy in the store.
- **Not a multi-backend abstraction.** This is Boswell-native. Backends like Obsidian or
  Mem0 cannot supply the schema, effectiveness, tiers, or gatekeeping this relies on, so a
  lowest-common-denominator connector would dissolve the value.

## 3. Entities

Two entities, **not** one polymorphic node. A unified `Node{kind}` would be null-heavy
(goals have no executable body; procedures have no children) — both a modeling smell and a
**traversal-speed** cost, since branch-walking would drag executable payloads it doesn't
need. Splitting keeps the navigation layer skinny and defers heavy bodies to the leaf.

### 3.1 `Procedure` — a stored how-to

Governing rule: **a rich, uniform, typed signature (for retrieval, gating, lifecycle) plus
an opaque, format-tagged body.** Effectiveness — not confidence — is the truth model.

| Group | Fields |
|---|---|
| **Identity / versioning** | `id` (UUIDv7), `namespace`, `name`, `version`, `supersedes` (prior-version link), `is_current` (head-of-lineage flag), `source` (`authored`\|`learned`\|`imported`) |
| **Grouping** | `goal` (handle grouping variants + versions that pursue the same outcome) |
| **Signature (uniform; drives retrieval + gating)** | `intent` (embedded text), `tags[]`, `parameters[]` (`{name,type,default?,desc}`), `preconditions[]`, `required_tools[]`, `postconditions[]`, `est_duration_sec?` |
| **Selection hints (soft; for ranking among siblings)** | `usage_notes` (prose), `context_tags[]` (e.g. `time:quick`, `mood:fancy`) |
| **Body (opaque, format-tagged)** | `body_format` (`prose`\|`dsl`\|`code`), `content_type`, `body` |
| **Effectiveness & lifecycle** | `tier` (`ephemeral`\|`task`\|`project`\|`permanent`), `use_count`, `success_count`, `failure_count`, `last_used_at`, `created_at`, `updated_at`, `stale_at`; `effectiveness` derived (success-rate × recency) |
| **Provenance** | see §5 (author, delegation chain, evidence, assurance) |

A **precondition** is structured and dogfoods Boswell — its `check` is a claim-query
pattern the store resolves against the claim store:

```json
{
  "kind": "resource",
  "description": "eggs on hand",
  "check": {
    "match": {"subject": "person:jd", "predicate": "attr:in-pantry", "object": "ingredient:eggs"},
    "min_confidence": 0.6,
    "expect": "exists"
  }
}
```

**Design decisions baked in:**
- **Effectiveness, not confidence.** A procedure isn't true/false; it's working/not.
- **Append-only versions.** "Refine" = a new row that `supersedes` the old; `is_current`
  marks the live head. History (and how you used to do it) is never lost, and effectiveness
  attaches per version. `is_current` is the cheap "live heads" filter; do **not** maintain a
  two-way `superseded_by` pointer (drifts) — derive it if ever needed.
- **Variants vs. versions.** `supersedes` is *same-technique* lineage. Genuinely different,
  equally-valid techniques for one outcome are **siblings** under a shared `goal`, all
  `is_current`. The schema declares no single best way.
- **Signature is format-invariant.** Adding `dsl`/`code` later touches only the body +
  executor, never the retrieval/gating surface. Ship `prose` first (the LLM is the
  interpreter); graduate to a DSL only when advisory prose demonstrably fails on control or
  inspectability.

**Worked example** — one goal, two live sibling procedures, chosen by context not
supersession:

```json
[
  {"name":"omelette-classic","goal":"goal:person:jd/cook-eggs","is_current":true,
   "context_tags":["mood:fancy","time:leisurely"],
   "usage_notes":"French-style, buttery, no color. 10 minutes, want it nice.",
   "body_format":"prose",
   "body":"Heat pan medium. Salt, pepper, butter. Beat 2–3 eggs; pour. Stir until just set; add filling if any. Flip once (twice if firm). Kill heat, rest ~30s, test firmness, serve.",
   "success_count":22,"failure_count":1,"tier":"project"},

  {"name":"eggs-quick-scramble","goal":"goal:person:jd/cook-eggs","is_current":true,
   "context_tags":["time:quick","effort:low"],
   "usage_notes":"3-minute soft scramble. Weekday default when rushing.",
   "body_format":"prose","body":"…","success_count":60,"failure_count":0,"tier":"project"}
]
```

### 3.2 `Goal` — a navigational decomposition node

Skinny by design (traversal touches these, not procedure bodies). Goals form a **DAG**
(not a tree — `cook-eggs` is reusable under `prepare-breakfast` *and* `quick-dinner`).

| Group | Fields |
|---|---|
| **Identity** | `id` (UUIDv7), `namespace`, `name`, `intent` (embedded — the semantic match key), `definition_of_done` (postconditions defining the target) |
| **Edges** | `children[]` — candidate ways to advance this goal; each child is *either* a sub-goal or a procedure, carrying edge-local `preconditions`, `context_tags`, `usage_notes`, `role` (see below), and cached `effectiveness` so a hop can be filtered and ranked **without fetching the child rows** |
| **Lifecycle** | `tier`, decay/`stale_at`, provenance (as §5) |

A child edge's `role` is `accomplish` (a way to make progress) or `decide` (a procedure
that helps *choose* among the accomplish-candidates). A decision-aid is **not a new type** —
it's a `Procedure` whose job is "choose among children of X," surfaced alongside the
candidates it ranks.

**Edge- vs. node-local conditions.** "Eggs on hand" is *node-intrinsic* to `cook-eggs` (a
precondition to run it at all). "Prefer eggs here when LDL is low" is *edge-contextual* (why
pick this child *under prepare-breakfast*). Intrinsic preconditions live on the entity;
contextual selection signals live on the edge.

### 3.3 The effectiveness-reporting contract

Effectiveness is only as good as the reports that feed it, so **retrieving a procedure for
execution carries an obligation to report the outcome** — the report is not an optional
hook. When the store hands out a procedure it issues an execution receipt:

```
execution_receipt: {
  receipt_id, procedure_id, version,
  issued_to: <principal>, task_id, session_id,
  expires_at, report_to,
  required: [outcome], optional: [failure_mode, executor_confidence, cost, notes]
}
```

The executor is obliged to report before `expires_at`:

```
report: { receipt_id, outcome: success | failure | abandoned,
          failure_mode?: preconditions_stale | step_failed(step) | bad_result | executor_error,
          executor_confidence?, cost?, notes? }
```

Rules:
- **The report is a provenance-stamped, gatekept write** (§5). A low-assurance executor's
  self-report is weak evidence and needs corroboration to move team-tier effectiveness, so a
  malicious or confused executor cannot tank a shared procedure with false failures.
- **`failure_mode` supplies attribution** (this is what open-problem #2 needs):
  `executor_error` does **not** demote the procedure; `bad_result`/`step_failed` do;
  `preconditions_stale` demotes the *precondition check*, not the body.
- **Silence is not success.** An unreported, expired receipt counts as `unknown` — mildly
  negative for reliability — so an agent cannot game stats by running a procedure and staying
  quiet on failure. Chronic non-reporting is itself an authority/provenance signal against the
  principal.
- Enforced operationally by the capture hooks (`SubagentStop`/`Stop`/`PostToolUse`).

## 4. Retrieval & traversal

**Stateless, agent-driven recursive descent.** The agent holds the cursor (consistent with
ADR-019 stateless sessions); the store never holds descent state. The agent queries one
level, chooses, and issues a new, more refined query — repeating until a candidate is an
executable leaf procedure.

```
"I need to eat" ─▶ expand(eat, ctx) ─▶ [prepare-breakfast, prepare-lunch, …]
                ─▶ expand(prepare-breakfast, ctx) ─▶ [cook-eggs, pour-cereal, …] (+ a decide-procedure)
                ─▶ expand(cook-eggs, ctx) ─▶ [omelette-classic, eggs-quick-scramble] (procedures)
```

### 4.1 The `expand(node, context)` contract — store-side, deterministic

Given a node and a context reference, the store returns a **candidate surface**:

1. **Filter** by hard preconditions — resolve each candidate's `check` against the claim
   store; drop candidates whose preconditions don't hold.
2. **Rank** the survivors deterministically by `effectiveness` and `context_tags` match.
3. **Return** the ranked candidates plus their `usage_notes` and the raw factor readings
   (e.g. `LDL=142, last-ate-eggs=yesterday, hunger=high`), and any `role: decide` procedures
   for this node.

**Surface, not decide.** Filtering by preconditions and ranking by effectiveness/context is
deterministic *surfacing*. The **weighting** — how much LDL outranks hunger today — is never
in the store; it lives with the agent, or with an agent-run `decide` procedure (itself just
stored prose that reads Boswell claims). The store can hand you *how you decide*; it never
decides.

Rationale for store-side (vs. returning raw adjacency): fewer round-trips and the
`check`-resolution happens co-located with the claim store. Deterministic-where-it-can-be
reduces the agent's effort per hop.

### 4.2 Performance model

Because the *agent* recurses, the store only ever does **single-hop adjacency reads** — a
b-tree lookup on an index over the edge's parent. No multi-hop graph query, therefore **no
graph database required**; Boswell's existing SQLite substrate suffices (with HNSW for the
top-level "I need to eat" → root-goal semantic match).

The real per-hop cost is precondition resolution (claim queries). Two mitigations:
- **Fetch the context slice once per descent.** LDL/hunger/pantry are stable across a short
  descent; pull the relevant claim slice once and evaluate every hop's `check`s in-process.
- **Self-describing edges.** Each candidate edge carries its precondition refs, hints,
  `context_tags`, and cached effectiveness inline — so a hop filters and ranks without
  fetching child rows. Heavy procedure bodies are fetched only at the leaf, at execution.

## 5. The write path: provenance and gatekept promotion

**The one rule: nothing writes directly to shared memory.** Every write is
provenance-stamped, enters at the lowest tier scoped to its author, and climbs only by
earning it. This is Boswell's existing **Gatekeeper** pattern (agents advocate; gatekeepers
decide what persists higher) pointed at the *agent hierarchy* rather than at claim confidence.

### 5.1 Provenance stamp (on every write — claim, procedure, or effectiveness update)

- `author` — stable agent identity, e.g. `agent:orch-7/sub:explore-3`.
- `delegation_chain` — the on-behalf-of path: `human:jd → agent:orch-7 → sub:explore-3`.
- `authority` — `{namespaces, max_tier, ops}` the writer may exercise (see §6).
- `evidence` — `observed` \| `inferred` \| `reported` \| `tool_output` (a trust-type).
- `assurance` — identity assurance from the `IdentityProvider` (see §6).
- `task`/`session` id + timestamp.

Boswell claims already carry `source_type` and provenance entries (source, rationale); this
extends that vocabulary rather than inventing a new store.

### 5.2 Entry tier and promotion

- **Entry tier = `min(requested, author.max_tier, ceiling(assurance, evidence))`.** A leaf
  subagent physically cannot land a `project`/team-tier entry.
- **Climbs a tier when:** a higher-authority parent **endorses** the advocated entry (it holds
  the verified outcome the child lacked); OR **independent corroboration** (N distinct authors
  assert the same claim / a procedure accrues M successes across distinct authors); OR an
  **effectiveness threshold** (success-rate × distinct-author-count × recency) is crossed.
- **Falls when:** a higher-authority **contradiction** (Boswell's contradiction janitor);
  **failure** at the serving tier (repeated failure → GC); or **decay** (unreinforced entries
  fade tier by tier).
- **Evidence-type sets the ceiling.** An entry whose only evidence is `tool_output` or
  low-authority `reported` cannot reach team tier on its own — it needs corroboration from a
  trusted-evidence author to raise the ceiling. This is the anti-poisoning lever (cf.
  AgentDojo, cited by OaK): a confused or adversarial leaf can only pollute its own ephemeral
  scope.

### 5.3 Reuse of existing machinery

| Write-path need | Reuses |
|---|---|
| Author authority → tier ceiling | Gateway API-key → namespace + max-tier scope (`boswell-gateway`) |
| Provenance stamp | Boswell provenance entries + `source_type` |
| Claim promotion / conflict | confidence + corroboration + contradiction janitor (`boswell-janitor`) |
| Climb/fade ladder | tiers + decay |
| Advocate/decide | the Gatekeeper (`boswell-gatekeeper`) |
| Procedure promotion signal | `effectiveness` (+ distinct-author counting) |
| Background promotion pass | Janitor/Synthesizer worker loop |

The write path is thus mostly **wiring**: provenance-stamp writes, generalize the gateway's
scope model from API keys to agent identities, and point the existing Gatekeeper/Janitor at
authority-and-corroboration-driven tier promotion.

## 6. Identity as a port (`IdentityProvider` / "IAuth")

Every trust claim above rests on **authenticated, unforgeable agent identity and a verifiable
delegation chain** — a real problem Boswell must not hardcode to any proprietary system.
Following Boswell's existing port pattern (`LlmProvider`, `ClaimStore`), identity is an
adapter behind a small, stable trait. The `boswell-gateway`'s API-key auth is effectively the
**first adapter** already.

```rust
trait IdentityProvider {                       // conceptual "IAuth"
    fn authenticate(&self, credential: &Credential) -> Result<Principal, AuthError>;
    fn verify_delegation(&self, chain: &DelegationChain) -> DelegationVerdict; // carries Assurance
}

enum Assurance { None, Asserted, Verified, Attested } // self-claimed → cryptographically signed
struct Principal { id, kind /* human|agent|service */, .. }
```

**Assurance is first-class, and the tier ceiling is a function of it.** An entry cannot climb
above tier `T` unless its author's identity `Assurance ≥` the level `T` requires:
`permanent`/team requires `Attested`; `ephemeral` accepts `Asserted`. This is what makes
"abstract identity now, choose a real system later" **safe by construction**:

- **No identity backend** → Boswell still runs as a **local, single-principal, ephemeral-tier**
  memory. Nothing self-asserted reaches shared/long-term tiers.
- **Plug in an attested provider later** (SPIFFE/SPIRE SVIDs, OIDC + mTLS, or Riptide
  Application Manager) → the *same* writes become eligible to climb, **with zero Gatekeeper
  changes**. Assurance flows up through the ceiling formula.

**Boundary:** the `IdentityProvider` **authenticates and attests** (who you are, is the
delegation chain real, at what assurance). Boswell **authorizes** (what a principal may write —
namespace, max_tier, ops). Do not let an external identity system own memory-authorization
policy; it's domain logic and belongs with the store (the same principal→scope mapping the
gateway config already does).

**Known non-closure:** verified identity yields *authenticated* principals, not *independent*
ones. One orchestrator can spawn N attested-but-correlated clones, so the corroboration
independence problem (§8) survives even a perfect provider. Identity raises the floor; it does
not close Sybil-weighting.

## 7. devAuth — an optional, repo-included development identity adapter

To let anyone stand Boswell up and exercise the whole trust gradient **without** a real
identity system, the repo ships an `IdentityProvider` adapter, **devAuth**
(`boswell-devauth`), with preset roles. It is a **stand-in until an operator brings their
own identity provider**, and it is deliberately, loudly **development/testing only**.

devAuth is reached only through the [`IdentityProvider`](#6-identity-as-a-port-identityprovider--iauth)
port: exactly one place in the tree (the instance server's composition root) names the
crate, and every other layer — gRPC, store, gateway, CLI — sees the port alone.

### 7.1 Sample identities

devAuth exposes a handful of assignable, fixed principals with differentiated authority so the
gradient's behavior is observable end-to-end:

| Identity | Namespace | max_tier | Ops | Nominal assurance | Purpose in testing |
|---|---|---|---|---|---|
| `standard-worker` | `agent:worker` | `task` | read, write | `Verified` | The ordinary agent; writes task-tier, advocates upward. |
| `untrusted-interloper` | `agent:interloper` | `ephemeral` | write (ephemeral only); evidence forced to `tool_output`/`reported` | `Asserted` | Red-team identity: demonstrates quarantine — its writes can't climb and its contradictions can't demote higher-tier memory. |
| `project-leader` | `project:*`, `agent:worker` | `project` | read, write, **endorse** | `Attested` | Demonstrates promotion via authority endorsement (endorses a worker's advocated entry → it climbs to project/team tier). Its authority spans the worker's namespace as well as its own, or it could never reach what the worker wrote (§8.3). |
| `memory-manager` | `*` | `permanent` | read, write, **endorse**, **curate** (promote/demote/forget/GC), resolve contradictions | `Attested` | The maintenance/curator role: demonstrates the Janitor-side lifecycle. Holds `endorse` because promotion is expressed through endorsement, and is the only identity combining it with a permanent ceiling — so top-tier promotion is reachable at all (§8.3). |

With these you can watch the full write path in a sandbox: the interloper advocates and stays
stuck at ephemeral; the worker writes task-tier; the project-leader endorses and the entry
climbs; the memory-manager curates and demotes.

### 7.2 Loud by design — mandatory safeguards

devAuth must make it impossible to *accidentally* treat it as real:

- **Refuses to run without explicit opt-in** — `BOSWELL_ALLOW_DEV_AUTH`; otherwise devAuth
  refuses to construct and the instance continues with *no* identity backend.
- **Hard production lockout** — `BOSWELL_ENV=production` is a fatal refusal regardless of
  opt-in. **An undeclared environment counts as production**: `BOSWELL_ENV` must be set to
  something non-production for devAuth to start. The realistic hazard for a bring-up adapter
  is not malice but drift — trial it, like it, deploy it, never think about identity again —
  and that path never sets `BOSWELL_ENV` at all, so reading "unset" as "not production" would
  keep fake identities alive straight through the transition.
- **Not behind a Cargo feature, deliberately.** An earlier draft of this section called for a
  non-default feature. That was wrong for what devAuth is *for*: someone clones the repo to
  watch the trust gradient work, and a build flag is friction aimed squarely at that audience.
  It would also ship untested, since CI builds default features only. The guarding is at
  startup instead (above), and nothing on the production path names the crate, so a build that
  never enables devAuth never constructs it.
- **Persistent warnings** — a startup banner, a warning on **every** principal assignment /
  token issuance, and a warning line in logs, all stating that these identities are for
  development and testing only and **must not be trusted for long-term memory**.
- **Surfaced downstream** — every response served under a devAuth principal carries an
  `X-Boswell-Auth: dev-untrusted` header, on rejected requests as much as successful ones. The
  instance reports its own status (`HealthCheckResponse.dev_auth`); the gateway learns it on
  connect and refreshes it on each health check, so the marker is derived from what is
  actually running rather than from gateway config an operator could forget to set.
- **Provenance tainting** — every write under devAuth is stamped `dev_provider: true` in its
  provenance, so dev-authored entries are always distinguishable and can be swept.
- **Store isolation (recommended default)** — devAuth points at a **separate, ephemeral** store
  namespace (or a throwaway DB), so dev memory cannot contaminate a real store, and encourage
  wiping it between runs.

devAuth is a bring-up and demonstration tool for the trust model — never a shortcut around it.

## 8. Open problems (explicit, unsolved)

1. **Sybil independence.** "N distinct authors corroborate" is gameable by correlated clones;
   corroboration needs an independence notion we don't have. **Measured, and mitigated as far as
   this layer can — see §8.3.** Corroboration counts authenticated principals, so a credential
   fanned out into subagents counts once whatever it claims. An adversary holding genuinely
   distinct credentials still counts distinctly; that part is the identity system's problem, and
   §8.4 accepts it.
2. **Effectiveness attribution.** On failure, was it the *procedure* or the *executor*?
   Demoting a good procedure for a bad executor's mistake is unfair. **Largely addressed** by
   the reporting contract (§3.3): the reporter supplies `failure_mode`, the gatekeeper weights
   it by reporter trust. What remains is *trusting the attribution*, which the trust gradient
   already bounds.
3. **Authoring & learning.** How Goals/Procedures get *into* memory and refine — hand-authored
   is fine to start, but "learned from experience" implies a procedure/goal extractor
   (inducing control flow and decomposition), meaningfully harder than claim extraction.
4. **Promotion timing.** Promotion belongs in a background sweep (Janitor-style), so a
   just-earned team fact lags until the sweep. Tunable, not free.
5. **Graph integrity under decay.** ~~Nodes and edges decaying independently can dangle the
   navigable graph; need a rule (an edge pins its child, or GC cascades/re-parents).~~
   **Resolved — see §8.2.** Both rules were prototyped; neither was adopted, and a third was.
6. **Cycle guards.** ~~The DAG must be kept acyclic (a `decide` procedure that re-enters a
   parent could loop); traversal needs guards.~~ **Resolved, both halves.** On write,
   `add_goal_edge` rejects any sub-goal edge that would close a cycle. At traversal,
   `DescentGuard` carries the visited set and the depth/node caps. Traversal is stateless and
   agent-driven (§4), so the store has no recursion of its own to bound — the guard lives with
   whoever recurses, and it *guards* without ever choosing a child. Belt and braces on purpose:
   the write guard makes a cycle unbuildable through the API, but a graph restored from backup
   or edited directly carries no such promise, and a descent that trusted the write guard alone
   would spin on one forever.

### 8.1 How we intend to make these tractable

Sorted by what actually resolves each — only one is genuine research:

- **Decide and test (engineering, not research):** promotion timing (#4 — a Janitor-style
  background pass, interval configurable, with a synchronous fast-track for authority
  endorsements); graph integrity under decay (#5 — pick a rule: an edge pins its child against
  GC, *or* cascade + re-parent orphans to the nearest live ancestor; prototype both, choose by
  behavior); cycle guards (#6 — reject on write any edge that would close a cycle, plus a
  visited-set + depth cap at traversal).
- **Largely handled by the reporting contract (§3.3):** attribution (#2). The reporter supplies
  `failure_mode`; the gatekeeper weights it by reporter trust.
- **Empirical — needs a running system + devAuth, not more design:** Sybil independence (#1).
  Stand up devAuth, script the four identities through cooperative and adversarial (clone-swarm)
  scenarios, and measure. Pragmatic proxy for independence: weight corroboration by **provenance
  diversity** — distinct delegation-chain roots, distinct sessions spread over time, distinct
  evidence types — and require **cross-authority** endorsement (a different org branch) for
  top-tier promotion. Accept it as mitigated, not solved; borrow from web-of-trust and
  reputation-system literature.
- **Sequence, don't block:** authoring & learning (#3). Ship hand-authoring; instrument real
  usage; build a procedure/goal extractor once there is a corpus of real episodes to learn from
  (the `Extractor` is the model). Both attribution and authoring echo **reinforcement-learning
  credit assignment** — borrow that framing rather than reinventing it.

Two cross-cutting principles make the unsolved ones safe to live with:
- **Bound the damage.** Where a problem can't be fully solved (Sybil), ensure the worst case is
  a *reversible* false promotion a `memory-manager` can demote — never irreversible corruption.
  Reversibility is the safety net.
- **Phase the risk.** Single-principal procedural memory (one agent's how-to across its own
  sessions) is useful on its own and trips almost none of these problems; the team-trust problems
  only bite at multi-agent scale — by which point a running single-principal system has produced
  the data needed to attack them.

### 8.2 Graph integrity under decay — what the prototypes showed

§8 #5 asked for a rule and named two candidates. Both were built and tested; the finding was
that **each fails in a way the other's failure hides**, and a third rule avoids both.

First, the failure being defended against. Goal edges carry `child_id` polymorphically (a child
is either a goal or a procedure), so the column can have no foreign key. Collect a child row by
any path that is not the collection API and its edges survive, pointing at nothing. That edge is
worse than absent: it carries its own cached effectiveness and usage notes (§4.2), so `expand`
surfaces it as a healthy, well-ranked candidate that cannot be fetched. **A dangling edge is
always a bug.** Every policy below prevents one, and `prune_dangling_goal_edges` repairs graphs
damaged another way (a stray `DELETE`, a restored backup, an older Boswell).

What the policies actually disagree about is **orphans** — children left with no live parent —
and that disagreement is a real trade-off:

| Rule | Dangles? | Orphans? | What it costs |
|---|---|---|---|
| `PinChildren` — an edge pins its child | never | never | **Decay stops at the first reference.** One forgotten edge keeps its whole subtree alive indefinitely. |
| `CascadeAndReparent` — re-attach to the nearest live ancestor | never | never | **Fabricates edges.** |
| `CascadeAndOrphan` — collect, drop the edges, leave children standing | never | yes, by design | Children fall out of the navigable graph. |

**Why pinning was rejected.** It is in direct tension with the premise that memory fades unless
it earns its keep. A pinned subtree is immortal by reference, not by usefulness, and the thing
holding it alive is exactly the kind of stale structure that should have decayed.

**Why re-parenting was rejected.** An edge's `preconditions`, `context_tags` and `usage_notes`
are **placement-specific** — that is the whole point of the edge-vs-node distinction in §3.2
("eggs on hand" is intrinsic to `cook-eggs`; "prefer eggs here when LDL is low" is contextual to
*this* placement). Re-parenting `cook-eggs` from `prepare-breakfast` up to `eat` must either
carry that context, asserting about the new placement something nobody said, or drop it, losing
the signal that made the child worth surfacing. It silently rewrites a hand-authored
decomposition, and the operator has no way to see that it happened.

**What was adopted: `CascadeAndOrphan`** (the default). Collect the node, drop every edge
touching it, leave surviving children exactly as they are. Nothing dangles and nothing is
invented. An orphan is still a first-class row — findable by `query_goals` and by direct id, and
re-attachable by anyone who wants it — and if nothing re-attaches it, it decays on its own clock
like any other unused memory. Decay proceeds; structure is only ever authored by an author.

All three ship, selectable per call, so an operator who disagrees can choose otherwise and
`orphaned_goals` makes the consequence measurable either way.

One structural note that fell out of the prototype: **re-parenting can never close a cycle.**
The edge it writes, `ancestor -> child`, is a shortcut over the `ancestor -> node -> child` path
that already existed; closing a cycle would need `child ->* ancestor` as well, which the
write-time guard (#6) already refuses. `collect_goal` still handles the cycle error defensively,
because a graph damaged outside the API carries no such guarantee.

### 8.3 Sybil independence — what the scenarios measured

§8.1 calls #1 *empirical, not design*, so it was measured rather than argued.
`crates/boswell-gatekeeper/tests/sybil_scenarios.rs` stands devAuth's four sample identities
up against the real `SqliteStore` write path, the real
`corroboration_facts_for_procedure`, and the real `PromotionGatekeeper`, and records the
end-to-end verdict. All four findings below have since been closed, so every scenario now asserts
an intent; while one was open it was named `finding_*` and pinned today's behaviour, so that
closing it showed up as a diff.

**What holds up.** The honest clone swarm is refused. Nine subagents fanned out under one
puppeteer produce nine distinct author identities and exactly **one** delegation root, and the
gatekeeper holds — raw author count would have promoted it. The interloper stays on the floor:
its evidence is coerced to `tool_output` and its authority caps entry at ephemeral, so nothing
it writes can climb. And corroboration is correctly *not* a grant of authority — two independent
roots backing a worker's procedure still cannot lift it past the worker's own `max_tier`.

**Finding 1 — the diversity signal was opt-in for the attacker. Resolved.** `root_of` derived a
stamp's delegation root from its chain and **fell back to the author when there was none**. A
clone that rooted its chain at itself, or omitted the chain entirely, was therefore
indistinguishable from an independent principal. The same nine-clone swarm that is held when it
declares its puppeteer climbed to project tier when it declined to — one compromised
project-lead credential, fanned out into self-rooted subagents, manufacturing the corroboration
the rule exists to require. The only difference between the held case and the promoted one was
whether the adversary chose to be honest about its provenance, which is not a property a defense
may depend on.

*The rule adopted: the independence unit is the **authenticated principal**, not the asserted
root.* Three candidates were considered and two rejected. (a) *An unrooted or self-rooted stamp
contributes no diversity root* fails closed, but `IdentityProvider::stamp_for` gives a direct
write the chain `[author]` — self-rooted by construction — so this would deny corroboration to
the entire no-delegation path, which is the most trustworthy case, not the least. (b) *All
self-rooted stamps collapse into one bucket* is the same objection one degree softer. (c)
*Count only roots whose delegation the provider verified* does not discriminate here: the clones
authenticate as a real identity with real assurance, and the thing they falsify is the chain,
which no current provider checks.

What separates the honest swarm from the dishonest one is not the chain — that is caller-supplied
in every path and can say anything — but the **author**, which the transport sets from the
principal that actually authenticated. `ProvenanceStamp::author` documents its own shape:
`agent:orch-7/sub:explore-3`, an authenticated principal plus a subagent path the principal chose
for itself. So a stamp's independence unit is now its delegation root — still falling back to the
author when there is no chain — **normalized to everything before the first `/`**. Nine
self-rooted `project:lead/sub:i` clones collapse onto `project:lead`; two genuinely distinct
principals writing directly still count as two. Both the honest and the dishonest swarm now
reach the same verdict, which was the point.

This does not *solve* Sybil independence and is not meant to: an adversary holding several
genuinely distinct credentials still counts several times, which is the irreducible part §8.4
accepts as mitigated rather than solved. What it closes is the free version — claiming
independence by declining to declare a chain. It also leans on the transport setting `author`
from the authenticated principal rather than from caller input; that holds today (no authoring
transport exists, and devAuth stamps the principal id directly), and it is the invariant any
future authoring endpoint must preserve.

**Finding 2 — the project leader could not endorse the worker it leads. Resolved.** devAuth's
module doc states the gradient plainly: "the worker writes task-tier, the project-leader can
endorse into project tier". Measured, that never happened. The leader's authority covered
`project*`; the worker writes into `agent:worker`; `endorse_procedure` refused on namespace
before it looked at anything else — so the endorsement half of §5.2 had only ever been exercised
by hand-built stamps, never by an identity provider.

*The repair:* the leader's authority now spans the worker's namespace as well as its own. §5.2's
own words are "climbs when a **higher-authority parent** endorses", and a parent whose authority
cannot reach its child is not a parent. The alternative — letting the worker write into the
project namespace — was rejected because the worker's own scratch namespace is the thing that
makes its writes *its own*, and widening it would have blurred the interloper demo alongside it.
The two authorities have to overlap somewhere for the gradient to be observable at all; the
overlap belongs on the side with more authority, not less.

**Finding 3 — top tier was unreachable in devAuth. Resolved.** A permanent climb needs an
endorsement whose `max_tier` is permanent (§5.2) *and* a cross-authority endorser (§8.1). The
only identity holding `Op::Endorse` was the project leader, capped at project; the memory manager
reached permanent but held `Curate`, not `Endorse`. So no combination of the four promoted
anything to permanent, however much corroboration was piled on, and the top-tier rule was
enforced without ever having run.

*The repair:* the memory manager holds `Endorse` alongside `Curate`. §7.1 already defines curation
as "promote/demote/forget/GC" — and promotion, in this model, is *expressed* through endorsement:
`endorsed_max_tier` is the only lever that raises the authority ceiling. A curator that could not
endorse could not perform the promotion its role is defined by. Granting it is not a widening of
the curator's power but an admission of what its power already was.

**Finding 4 — two of the three diversity axes were computed but never weighed. Resolved, with a
default that keeps them off.** §8.1's proxy is "distinct delegation-chain roots, distinct sessions
spread over time, distinct evidence types". The store computed all three and carried them on
`CorroborationFacts`; `PromotionConfig` read only `min_distinct_roots`, so a single burst promoted
exactly as readily as corroboration accumulated across sessions from varied evidence.

`PromotionConfig` now carries `min_distinct_sessions` and `min_distinct_evidence_types`, both
narrowing the **corroboration** trigger only — endorsement is an authority judgement and
effectiveness an outcome one, and neither is about independence. Both default to `0`, meaning
not enforced, and the default is the interesting part:

- **Sessions.** `distinct_sessions` is counted over an entry's *write* and *endorse* stamps.
  Reports are excluded from corroboration deliberately, and reports are the only stamps anything
  currently populates a session on (propagated from the receipt). Writes and endorsements are
  minted in-process today, where devAuth leaves `session_id` as `None` and the Janitor's own
  writes have no session to name. A non-zero default would therefore not make promotion stricter;
  it would make corroboration unreachable. The precondition for raising it is an authoring
  transport that stamps sessions onto writes, and that is recorded on the field itself.
- **Evidence types.** Weak evidence is already bounded, and more tightly than a diversity count
  would bound it: `EvidenceType::tier_ceiling` caps what `tool_output` or `reported` can reach at
  all, so corroboration built entirely on weak evidence cannot pass task tier however many roots
  back it. This axis is belt-and-braces over a control that already works, which is a reason to
  offer it and not a reason to impose it.

Shipping the mechanism switched off is the same call §8.2 made for its three collection policies:
build them, make the choice explicit and per-operator, and let the consequence be measurable
either way. What it is not is a claim that "spread over time" is now enforced — it is enforceable,
which is a smaller and more honest thing to have.

Findings 2 and 3 were defects in the *sample roster*, not in the trust model: they made the
gradient unobservable, which is precisely what devAuth exists to provide. Finding 1 was a defect
in the model's implementation and is the one that mattered for §8 #1. Finding 4 was unfinished
work against §8.1's own stated proxy.

Worth naming what findings 2 and 3 cost while they stood: **the endorsement path and the top-tier
rule had unit tests and no end-to-end evidence.** Both were correct as written and neither had
ever run against a stamp an identity provider minted, because the roster made them unreachable.
That is the argument for the scenarios existing at all — a policy can be right and still be
untested if nothing can get into the state it governs.


### 8.4 Posture & trust boundary — a thorny hedge, not a wall

Boswell does not aim to be perfect or unassailable; it aims to **work**. The target for the
whole trust model is a *thorny hedge*: raise the cost of casual or careless memory poisoning,
keep out the worst actors, and make damage recoverable — **not** immunity to a determined
adversary who already controls the host.

- **Local-first shifts the boundary to the implementer.** Boswell instances typically run on
  the operator's own hardware with locally-hosted agents. So **identity and usage governance are
  the implementer's responsibility.** Boswell supplies the tools — provenance, tiers,
  gatekeeping, assurance-gated ceilings — and a safe default; the operator decides which agents
  to run and what to trust with which tier. A lower-capability model *can* run away and dirty the
  substrate; guarding against that is a shared responsibility, not something Boswell can fully
  prevent from inside.
- **Recovery is the backstop, in two grades.** The **provenance scalpel** — targeted removal by
  author, tier, or time — handles most contamination without losing everyone else's memory;
  **backups** (full restore) handle the rest. See `docs/architecture/16-backup-recovery.md`.
  Together they make a bad actor's damage bounded and reversible, which is what lets us accept an
  imperfect Sybil defense.

## 9. Build status

Deliberately not recorded here. Status for every slice of this design — shipped, in
flight, open, and deferred with its reason — lives in
[`docs/development/roadmap.md`](../development/roadmap.md), which is the single source of
truth for where Boswell stands.

This section previously carried its own seven-phase list. It and the roadmap did not
reference each other in either direction, which is precisely how the two drifted apart:
work recorded in one was invisible from the other, and the roadmap ended up describing a
claims-only system while this design's entire build went unmentioned there. A design
document that also tracks status is a second place to look, and a second place to look is
a second place to be wrong.

What stays here is the design: the model (§3, §4), the trust and provenance rules (§5–§7),
and the open problems with what was measured against them (§8).

## 10. Relationship to existing components

- **Claims** (`02-claim-model`, `04-claim-store`) remain the declarative substrate; procedure
  preconditions resolve against them.
- **Tiers & decay** (`13-confidence`, `07-janitor`) provide the lifecycle ladder for both
  entities.
- **Gatekeeper** (`08-gatekeeper`) becomes the promotion authority for advocated writes.
- **Extractor** (`05-extractor`) is the closest analog for future procedure/goal *learning*.
- **Gateway** (`boswell-gateway`, `docs/integrations/http-api.md`) is the bootstrap identity
  adapter and the natural surface for `expand`/write endpoints.
- **Security** (`10-security`, ADR-017/019) — the identity port is the concrete path toward the
  aspirational per-instance trust model, degrading gracefully when no provider is present.
