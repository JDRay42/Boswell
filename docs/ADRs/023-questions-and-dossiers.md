# ADR-023: Questions as Memory, Dossiers as the Research Read

## Status

Accepted. Design of record is
[`17-research.md`](../architecture/17-research.md). Nothing here is built yet;
[`docs/development/roadmap.md`](../development/roadmap.md) records what exists.

## Context

Boswell's users are increasingly teams of agents researching one problem together. Research
puts two demands on memory that it does not meet today.

1. **A subject-shaped read.** An agent starting on a topic needs everything Boswell holds
   about it — claims, procedures, goals — together with how far each can be trusted and how
   independently it is backed. Today that takes a semantic search, a structural query per
   subject, a relationship walk per claim, and a separate procedure and goal retrieval, and
   the caller assembles the result. `Reflect` (`03-api-surface.md`) was specified for this
   and never built. It is LLM-first: it answers with a narrative, which every agent in a
   swarm would pay for and none could inspect.
2. **A record of what is not known.** When an agent searches for something and finds
   nothing, the finding evaporates with its context window. The next agent repeats the
   search. When a contradiction goes unresolved or a claim goes stale, nothing records that
   someone should look. Boswell remembers what is so; it has no way to remember what is
   open.

Two settled decisions constrain the answer. Corroboration counts delegation roots, not
authors (`CONTEXT.md`; ADR-022), so a team of agents under one grant is one witness however
many of them agree. And the reporting loop — issue, receipt, outcome report, silence counts
against — is the mechanism Boswell already uses to make obligations stick.

## Decision

**1. A Question is a first-class memory.** A fourth entity beside claims, procedures and
goals: something an agent, or the Janitor, has recorded as not known. It carries a subject,
an optional predicate, a namespace, a tier, a provenance stamp and a status, and it decays
like everything else. It is written through the same gatekept, provenance-stamped path as
any other write.

**2. Taking a question issues a receipt.** An agent that takes a question on owes an outcome
report — `answered`, `partial`, `unanswerable` or `abandoned` — and an expired receipt
counts as `unknown`. The reporting loop is reused rather than paralleled; receipts gain a
target that is either a procedure or a question.

**3. `unanswerable` is a result, not a failure.** It must carry what was tried. It is held
like any other memory: it decays, it can be corroborated by further roots that also came up
empty, and when it fades the question returns to open. This is the part that records what
is not known.

**4. The dossier is a deterministic read.** One call returns everything held about a subject,
grouped and annotated — settled, contested, thin, stale — with the independence roots behind
each group, the effectiveness of each procedure, and the questions open against it. No LLM
is involved, and like `expand` it issues no receipt. A narrative may be layered on top as a
separate, optional call. This replaces `Reflect`'s retrieval half and demotes its narrative
to an option.

**5. Independence stays at the delegation root.** Each agent's work is attributed by
`author` in the dossier, so a reader sees which agent said what; corroboration still counts
`ProvenanceStamp::independence_root`. A research team under one grant is one root.

Every agent under one grant authenticates as the same principal — an attenuated token keeps
its root's `key_id` — so per-agent attribution needs a label the transport cannot verify.
It comes from an `X-Boswell-Agent` header and is appended as a subagent path,
`writer/sub:explore-3`, which `authenticated_principal` already strips at the counting
end. A lying label misattributes work among one principal's own agents and changes no
count. Claim writes carry no author today; stamping them is a prerequisite of the dossier.

**6. Gaps are persisted, never computed on read.** The Janitor detects them — unresolved
contradictions, stale claims, thinly backed claims at high tiers, goals with no effective
procedure under them — and records them as Questions. The dossier only reads. A read that
wrote would put a write behind a read scope.

## Alternatives

**A question as a claim** — `(subject, "is unknown", predicate)`. No new entity, no new
store. Rejected because a claim is an assertion about the world held with a confidence
interval, and "nobody here knows X" is an assertion about the store. It would corroborate,
contradict and promote under rules built for the wrong thing, and every claim query would
have to filter it out.

**Gaps computed on read only.** Cheaper, and always current. Rejected because the record is
the point: a computed gap cannot carry who tried, what they tried, or that three independent
agents came up empty. It also cannot be taken on, so two agents would research it at once
without either knowing.

**`Reflect` as specified.** One LLM call returning a narrative. Rejected as the primary read
because its cost scales with the number of agents asking, and because a narrative hides the
structure — which claims are thin, whose word they rest on — that a researcher most needs.
Kept as an optional layer.

**Each agent its own delegation root.** Considered for this ADR and rejected. Agents launched
from one grant share a principal, usually a model, a prompt lineage and their sources. Giving
each its own root would let a team corroborate, and so promote, its own findings — the
false consensus ADR-022's consequences already warn about.

**One claimant per question, locked.** Prevents duplicated effort. Rejected because
independent answers to the same question are exactly what corroboration needs. Pending
receipts are shown in the dossier instead: agents can see a question is being worked and
choose. The store surfaces; the agent decides.

## Consequences

- **A new entity, store and set of routes.** `Question`, a question store beside the
  procedure and goal stores, and gateway routes to raise, take, report on and read. All
  writes take the `write` operation and namespace scope that exist today.
- **Receipts generalize.** `execution_receipts` currently keys on `procedure_id`. It gains a
  target kind, and the outcome vocabulary differs per target. The word "receipt" stays, per
  the glossary.
- **The dossier depends on semantic intent matching for procedures and goals,** which is
  *open* on the roadmap. Until it ships, procedures and goals join a dossier by substring
  match on intent, and the dossier says so in its coverage.
- **Raising a question is a write over the wire,** which puts it under the same constraint
  as goal and procedure authoring: `author` must come from the authenticated principal, never
  from caller input.
- **Questions can be spammed.** They assert nothing, so they cannot poison memory, but they
  can bury the real ones. Raising is idempotent by meaning — a near-duplicate returns the
  existing question and adds its raiser — and the number of distinct roots that raised a
  question is its demand, which is what ranks it.
