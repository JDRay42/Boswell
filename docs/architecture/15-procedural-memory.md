# Procedural & Goal Memory for Agent Teams

**Status:** Design (not yet implemented). This document captures a design worked out in
depth; it is a proposal for review, not a description of shipped code.

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
execution_contract: {
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
identity system, the repo ships an optional `IdentityProvider` adapter, **devAuth**
(`boswell-devauth`, feature-gated, off by default). It is deliberately, loudly
**development/testing only**.

### 7.1 Sample identities

devAuth exposes a handful of assignable, fixed principals with differentiated authority so the
gradient's behavior is observable end-to-end:

| Identity | Namespace | max_tier | Ops | Nominal assurance | Purpose in testing |
|---|---|---|---|---|---|
| `standard-worker` | `agent:worker` | `task` | read, write | `Verified` | The ordinary agent; writes task-tier, advocates upward. |
| `untrusted-interloper` | `agent:interloper` | `ephemeral` | write (ephemeral only); evidence forced to `tool_output`/`reported` | `Asserted` | Red-team identity: demonstrates quarantine — its writes can't climb and its contradictions can't demote higher-tier memory. |
| `project-leader` | `project:*` | `project` | read, write, **endorse** | `Attested` | Demonstrates promotion via authority endorsement (endorses a worker's advocated entry → it climbs to project/team tier). |
| `memory-manager` | `*` | `permanent` | read, write, **curate** (promote/demote/forget/GC), resolve contradictions | `Attested` | The maintenance/curator role: demonstrates the Janitor-side lifecycle. |

With these you can watch the full write path in a sandbox: the interloper advocates and stays
stuck at ephemeral; the worker writes task-tier; the project-leader endorses and the entry
climbs; the memory-manager curates and demotes.

### 7.2 Loud by design — mandatory safeguards

devAuth must make it impossible to *accidentally* treat it as real:

- **Refuses to run without explicit opt-in** — a config flag (`allow_dev_auth = true`) or CLI
  flag; otherwise the process exits with an explanatory error.
- **Hard production lockout** — selecting devAuth while a production environment/flag is set is
  a fatal error; the adapter is behind a non-default Cargo feature so production builds can
  exclude it entirely.
- **Persistent warnings** — a startup banner, a warning on **every** principal assignment /
  token issuance, and a warning line in logs, all stating that these identities are for
  development and testing only and **must not be trusted for long-term memory**.
- **Surfaced downstream** — every response served under a devAuth principal carries a marker
  (e.g. an `X-Boswell-Auth: dev-untrusted` header and/or a `warnings[]` field) so calling
  agents and the gateway see it too.
- **Provenance tainting** — every write under devAuth is stamped `dev_provider: true` in its
  provenance, so dev-authored entries are always distinguishable and can be swept.
- **Store isolation (recommended default)** — devAuth points at a **separate, ephemeral** store
  namespace (or a throwaway DB), so dev memory cannot contaminate a real store, and encourage
  wiping it between runs.

devAuth is a bring-up and demonstration tool for the trust model — never a shortcut around it.

## 8. Open problems (explicit, unsolved)

1. **Sybil independence.** "N distinct authors corroborate" is gameable by correlated clones;
   corroboration needs an independence notion we don't have.
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
5. **Graph integrity under decay.** Nodes and edges decaying independently can dangle the
   navigable graph; need a rule (an edge pins its child, or GC cascades/re-parents).
6. **Cycle guards.** The DAG must be kept acyclic (a `decide` procedure that re-enters a parent
   could loop); traversal needs guards.

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

## 9. Suggested phasing

1. `Procedure` entity + store + prose executor; retrieval by `goal`/intent + precondition
   filter; `expand`/rank for procedure siblings. (Read path, single level.)
2. `Goal` entity + edges + recursive `expand(node, context)`; decision-aid `role`.
3. Provenance stamp + authority-scoped entry tiers; wire Gatekeeper/Janitor to promotion.
4. `IdentityProvider` port + assurance-gated ceilings; **devAuth** adapter.
5. Effectiveness feedback capture (via hooks: `SubagentStop`/`Stop`/`PostToolUse`).
6. Tackle §8 as research: attribution, Sybil-weighting, procedure learning.

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
